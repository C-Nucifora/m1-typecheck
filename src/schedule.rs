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
    resolve_channel(&path_text(n), scope)
}

/// Resolve `path` to a `Channel`/`Parameter` symbol path, or `None`. Handles the
/// `This.` anchor (the enclosing group) that [`resolve`] otherwise treats as an
/// opaque library root, so a member read/write written as `This.Debounce` /
/// `This.Foo.Set(…)` counts toward the group's symbol (without it those reads are
/// invisible and the symbol looks never-read/assigned — the T094/T093 false
/// positive on `This.`-only access).
fn resolve_channel(path: &str, scope: &Scope) -> Option<String> {
    let anchored;
    let p = match (path.strip_prefix("This."), scope.group.as_ref()) {
        (Some(rest), Some(group)) => {
            anchored = format!("{group}.{rest}");
            anchored.as_str()
        }
        _ => path,
    };
    match resolve(p, scope) {
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
    // A method call on a channel/parameter: `Chan.Set*(…)` writes it (the manual's
    // imperative setter), any other method (`Get`/`AsInteger`/`Lookup`/…) reads it.
    // The plain-expression path below only catches `Chan = …` / bare reads, so
    // without this every `Chan.Set(v)` would be mis-counted as a *read* of `Chan`
    // and `Chan` would look never-assigned (the T093 false positives).
    if n.kind() == Kind::CallExpression
        && let Some(callee) = n
            .named_children()
            .into_iter()
            .find(|c| matches!(c.kind(), Kind::Identifier | Kind::MemberExpression))
    {
        let text = path_text(callee);
        if let Some((receiver, method)) = text.rsplit_once('.')
            && let Some(chan) = resolve_channel(receiver, scope)
        {
            if method.starts_with("Set") {
                io.writes.insert(chan);
            } else {
                io.reads.insert(chan);
            }
            // Reads in the arguments still count; skip the callee (already handled).
            if let Some(args) = n
                .children()
                .into_iter()
                .find(|c| c.kind() == Kind::ArgumentList)
            {
                collect(args, scope, io);
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

/// Project-wide usage audit (opt-in): channels no script writes (T093, mirroring
/// M1 Build Error 1627 "Channel value not assigned") and parameters no script
/// reads (T094, M1 Build Error 1631 "Parameter value not read").
///
/// OPT-IN audit aids, not default-on (same rationale as T088/T089): a real project
/// legitimately has channels valued by sources this static pass cannot see — tables,
/// hardware/IO resources, firmware — so some entries are expected and warrant review
/// rather than being hard errors. Exemptions remove the obvious non-script sources:
/// CAN signals (`BuiltIn.CAN.*`) and members of a package-object instance (MoTeC
/// Input/Output sensors, etc., via `crate::audit::ObjectOwnership`); only plain
/// `BuiltIn.Channel` / `BuiltIn.Parameter` are considered. Pass every project script
/// in `scripts` so the union of writes/reads is complete.
pub fn check_usage(
    project: &Project,
    scripts: &[(String, String)],
    t093: bool,
    t094: bool,
) -> Vec<TypeDiagnostic> {
    if !t093 && !t094 {
        return Vec::new();
    }
    let ios: Vec<ScriptIo> = scripts
        .iter()
        .filter_map(|(f, s)| script_io(project, f, s))
        .collect();
    let mut written: BTreeSet<&str> = BTreeSet::new();
    let mut read: BTreeSet<&str> = BTreeSet::new();
    for io in &ios {
        written.extend(io.writes.iter().map(String::as_str));
        read.extend(io.reads.iter().map(String::as_str));
    }

    let table = project.symbols();
    let ownership = crate::audit::ObjectOwnership::new(table);
    let class_by_path: std::collections::HashMap<&str, &str> = table
        .iter()
        .filter_map(|s| Some((s.path.as_str(), s.classname.as_deref()?)))
        .collect();
    // The auto-created `Value` child of a Table or a value-bearing compound group
    // is sourced by that parent component (interpolation / compound logic), not by
    // a script — M1 Build does not require a script assignment for it.
    let is_component_value = |path: &str| {
        path.rsplit_once('.').is_some_and(|(parent, leaf)| {
            leaf == "Value"
                && matches!(
                    class_by_path.get(parent),
                    Some(&"BuiltIn.Table") | Some(&"BuiltIn.GroupCompound")
                )
        })
    };
    // A channel/parameter sourced outside any script (CAN bus, package-object
    // resource, table/compound output) is not expected to be script-assigned — exempt it.
    let exempt = |s: &crate::symbols::Symbol| {
        s.classname
            .as_deref()
            .is_some_and(|c| c.starts_with("BuiltIn.CAN."))
            || ownership.owned_by_object(&s.path)
            || is_component_value(&s.path)
    };

    let mut out = Vec::new();
    if t093 {
        for s in table
            .iter()
            .filter(|s| s.kind == SymbolKind::Channel)
            .filter(|s| s.classname.as_deref() == Some("BuiltIn.Channel"))
            .filter(|s| !exempt(s))
        {
            if !written.contains(s.path.as_str()) {
                out.push(make_project_for(
                    TypeCode::T093,
                    Severity::Warning,
                    format!(
                        "channel `{}` is never assigned by any script (M1 Build Error 1627: \"Channel value not assigned\")",
                        s.path
                    ),
                    &s.path,
                ));
            }
        }
    }
    if t094 {
        for s in table
            .iter()
            .filter(|s| s.kind == SymbolKind::Parameter)
            .filter(|s| s.classname.as_deref() == Some("BuiltIn.Parameter"))
            .filter(|s| !exempt(s))
        {
            if !read.contains(s.path.as_str()) {
                out.push(make_project_for(
                    TypeCode::T094,
                    Severity::Warning,
                    format!(
                        "parameter `{}` is never read by any script (M1 Build Error 1631: \"Parameter value not read\")",
                        s.path
                    ),
                    &s.path,
                ));
            }
        }
    }
    out.sort_by(|a, b| a.inner.message.cmp(&b.inner.message));
    out
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    use crate::project::Project;

    const XML: &str = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Ctrl"/>
  <Component Classname="BuiltIn.FuncUser" Filename="Ctrl.Update.m1scr" Name="Root.Ctrl.Update"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Ctrl.Written"><Props Type="f32" Security="Tune"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Ctrl.SetWritten"><Props Type="f32" Security="Tune"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Ctrl.Orphan"><Props Type="f32" Security="Tune"/></Component>
  <Component Classname="BuiltIn.CAN.Signal" Name="Root.Ctrl.CanIn"><Props Type="f32"/></Component>
  <Component Classname="BuiltIn.Table" Name="Root.Ctrl.Map"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Ctrl.Map.Value"><Props Type="f32" Security="Tune"/></Component>
  <Component Classname="BuiltIn.Parameter" Name="Root.Ctrl.ReadParam"><Props Type="f32" Security="Tune"/></Component>
  <Component Classname="BuiltIn.Parameter" Name="Root.Ctrl.UnreadParam"><Props Type="f32" Security="Tune"/></Component>
  <Component Classname="BuiltIn.Parameter" Name="Root.Ctrl.Debounce"><Props Type="f32" Security="Tune"/></Component>
</Project>"#;

    fn run(src: &str) -> Vec<String> {
        let project = Project::from_xml(XML).unwrap();
        let scripts = vec![("Ctrl.Update.m1scr".to_string(), src.to_string())];
        check_usage(&project, &scripts, true, true)
            .into_iter()
            .map(|d| d.inner.message)
            .collect()
    }

    #[test]
    fn assignment_and_set_writes_count_this_reads_count_orphans_flag() {
        let msgs = run(
            "Written = 1.0;\nSetWritten.Set(2.0);\nlocal r = ReadParam;\nlocal d = This.Debounce;\n",
        );
        let flagged = |needle: &str| msgs.iter().any(|m| m.contains(needle));
        // Never-written channel → T093; never-read parameter → T094.
        assert!(
            flagged("`Root.Ctrl.Orphan`"),
            "orphan channel flagged: {msgs:?}"
        );
        assert!(
            flagged("`Root.Ctrl.UnreadParam`"),
            "unread parameter flagged: {msgs:?}"
        );
        // Written by `=` and by `.Set()` are NOT flagged (the .Set fix).
        assert!(!flagged("`Root.Ctrl.Written`"), "= write counts: {msgs:?}");
        assert!(
            !flagged("`Root.Ctrl.SetWritten`"),
            ".Set() write counts: {msgs:?}"
        );
        // Read by a bare ref and by `This.` are NOT flagged (the This. fix).
        assert!(
            !flagged("`Root.Ctrl.ReadParam`"),
            "bare read counts: {msgs:?}"
        );
        assert!(
            !flagged("`Root.Ctrl.Debounce`"),
            "This. read counts: {msgs:?}"
        );
        // Exemptions: CAN signals and a Table's `.Value` are never flagged.
        assert!(!flagged("`Root.Ctrl.CanIn`"), "CAN signal exempt: {msgs:?}");
        assert!(
            !flagged("`Root.Ctrl.Map.Value`"),
            "table value exempt: {msgs:?}"
        );
    }

    #[test]
    fn off_by_default_flags() {
        // Neither code selected → no work, no findings.
        let project = Project::from_xml(XML).unwrap();
        let scripts = vec![("Ctrl.Update.m1scr".to_string(), String::new())];
        assert!(check_usage(&project, &scripts, false, false).is_empty());
    }
}
