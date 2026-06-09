//! Scheduling analysis (#145) — project-wide checks over the script
//! write/read graph and each script's call rate (`SelectedTrigger`).
//!
//! Both checks are OPT-IN (`--select T088` / `--select T089`): the real
//! corpora contain deliberate same-rate feedback loops (CAN transceive <->
//! control updates) that M1 Build accepts — a reader simply sees the previous
//! tick's value — so neither check is safe as default-on. They are
//! exploration/audit tools:
//!
//! - **T088 circular-dependency:** the manual (p.29, Scheduling) calls a
//!   circular dependency "the most common error"; this surfaces every
//!   same-rate write/read cycle between scripts for review. Cross-rate
//!   feedback (a 100 Hz script reading back a 1000 Hz result) is never
//!   flagged.
//! - **T089 rate-inversion:** a faster script reading a channel written only
//!   by a slower one sees stale values between writer ticks. Often
//!   intentional (downsampled inputs).
use crate::diagnostics::{TypeCode, TypeDiagnostic, make_project_for};
use crate::project::Project;
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use crate::typer::{is_expr, path_text};
use m1_core::{Kind, Node, Severity};
use std::collections::{BTreeMap, BTreeSet};

/// Per-script channel access sets, resolved against the project.
struct ScriptIo {
    fn_path: String,
    rate_hz: Option<f64>,
    writes: BTreeSet<String>,
    reads: BTreeSet<String>,
}

/// Walk one script, resolving every project-symbol reference into the
/// read/write sets. Assignment targets are writes (compound assignments are
/// both); everything else in expression position is a read.
fn script_io(project: &Project, file_name: &str, source: &str) -> Option<ScriptIo> {
    let fn_path = project.function_symbol_for_script(file_name)?;
    let cst = m1_core::parse(source);
    if !cst.syntax_diagnostics().is_empty() || cst.root().max_depth() > m1_core::MAX_RECURSION_DEPTH
    {
        return None;
    }
    let group = project.group_for_script(file_name);
    let scope = Scope {
        locals: crate::rules::collect_locals(cst.root(), Some(project), group.as_deref()),
        group,
        project: Some(project),
        fn_symbol: Some(fn_path.clone()),
    };
    let rate_hz = project.symbols().get(&fn_path).and_then(|s| s.call_rate_hz);
    let mut io = ScriptIo {
        fn_path,
        rate_hz,
        writes: BTreeSet::new(),
        reads: BTreeSet::new(),
    };
    collect(cst.root(), &scope, &mut io);
    Some(io)
}

fn channel_path(n: Node, scope: &Scope) -> Option<String> {
    if !matches!(n.kind(), Kind::Identifier | Kind::MemberExpression) {
        return None;
    }
    match resolve(&path_text(n), scope) {
        Resolution::Symbol(s) if matches!(s.kind, SymbolKind::Channel | SymbolKind::Parameter) => {
            Some(s.path.clone())
        }
        _ => None,
    }
}

fn collect(n: Node, scope: &Scope, io: &mut ScriptIo) {
    if n.kind() == Kind::AssignmentStatement {
        let kids = n.named_children();
        let target = kids.iter().find(|c| is_expr(c.kind()));
        let compound = !n.children().iter().any(|c| c.kind() == Kind::Assign);
        if let Some(t) = target {
            if let Some(path) = channel_path(*t, scope) {
                io.writes.insert(path.clone());
                if compound {
                    io.reads.insert(path); // `+=` reads the target first
                }
            }
            // Walk only the value side for reads.
            for c in kids.iter().filter(|c| !std::ptr::eq(*c, t)) {
                collect(*c, scope, io);
            }
            return;
        }
    }
    if let Some(path) = channel_path(n, scope) {
        io.reads.insert(path);
        return; // a member chain resolves as a whole; don't descend
    }
    for c in n.children() {
        collect(c, scope, io);
    }
}

/// Run the scheduling checks. `scripts` pairs file names with sources (the
/// same shape `infer_return_types` takes); `t088`/`t089` opt the respective
/// check in (both are off by default — see the module docs).
pub fn check(
    project: &Project,
    scripts: &[(String, String)],
    t088: bool,
    t089: bool,
) -> Vec<TypeDiagnostic> {
    if !t088 && !t089 {
        return Vec::new();
    }
    let ios: Vec<ScriptIo> = scripts
        .iter()
        .filter_map(|(f, s)| script_io(project, f, s))
        .collect();
    let mut out = Vec::new();

    // channel -> writer scripts (indices).
    let mut writers: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, io) in ios.iter().enumerate() {
        for w in &io.writes {
            writers.entry(w).or_default().push(i);
        }
    }

    // Script-level edges: reader -> writer (the reader DEPENDS on the writer),
    // kept only between scripts at the same known rate (T088's scope).
    let mut deps: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let mut t089_seen: BTreeSet<(usize, String)> = BTreeSet::new();
    for (i, io) in ios.iter().enumerate() {
        for r in &io.reads {
            for &w in writers.get(r.as_str()).map(Vec::as_slice).unwrap_or(&[]) {
                if w == i {
                    continue;
                }
                let same_rate = match (io.rate_hz, ios[w].rate_hz) {
                    (Some(a), Some(b)) => (a - b).abs() < f64::EPSILON,
                    _ => false,
                };
                if same_rate {
                    deps.entry(i).or_default().insert(w);
                }
                // T089: this script reads `r`, and EVERY writer of `r` is
                // strictly slower.
                if t089
                    && let (Some(a), Some(b)) = (io.rate_hz, ios[w].rate_hz)
                    && a > b
                    && writers[r.as_str()]
                        .iter()
                        .all(|&x| ios[x].rate_hz.is_some_and(|rb| rb < a))
                    && t089_seen.insert((i, r.clone()))
                {
                    out.push(make_project_for(
                        TypeCode::T089,
                        Severity::Hint,
                        format!(
                            "`{}` ({} Hz) reads `{r}`, written only by `{}` ({} Hz) — the value is stale between writer ticks",
                            io.fn_path, a, ios[w].fn_path, b
                        ),
                        &io.fn_path,
                    ));
                }
            }
        }
    }

    // T088: cycles in the same-rate dependency graph (DFS, each cycle reported
    // once via its lexicographically-smallest member).
    let mut reported: BTreeSet<String> = BTreeSet::new();
    if !t088 {
        deps.clear();
    }
    for &start in deps.keys() {
        let mut stack = vec![(start, vec![start])];
        while let Some((node, path)) = stack.pop() {
            for &next in deps.get(&node).into_iter().flatten() {
                if next == start {
                    let mut names: Vec<&str> =
                        path.iter().map(|&i| ios[i].fn_path.as_str()).collect();
                    let anchor = names.iter().min().unwrap_or(&"").to_string();
                    if reported.insert(anchor.clone()) {
                        names.push(ios[start].fn_path.as_str());
                        out.push(make_project_for(
                            TypeCode::T088,
                            Severity::Warning,
                            format!(
                                "circular write/read dependency at the same rate: {}",
                                names.join(" -> ")
                            ),
                            anchor,
                        ));
                    }
                } else if !path.contains(&next) && path.len() < 16 {
                    let mut p = path.clone();
                    p.push(next);
                    stack.push((next, p));
                }
            }
        }
    }
    out.sort_by(|a, b| a.inner.message.cmp(&b.inner.message));
    out
}
