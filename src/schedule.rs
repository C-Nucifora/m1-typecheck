//! Scheduling analysis (#145) — project-wide checks over the script
//! write/read graph and each script's call rate (`SelectedTrigger`).
//!
//! - **T088 circular-dependency (default-on):** the manual (p.29, Scheduling)
//!   calls a circular dependency "the most common error"; this surfaces every
//!   same-rate write/read cycle between scripts. It mirrors M1 Build's Warning
//!   1640 "circular schedule dependency found and resolved" (one cycle each on
//!   the real AV-M1 project), so it is default-on. Cross-rate feedback (a 100 Hz
//!   script reading back a 1000 Hz result) is never flagged.
//! - **T089 rate-inversion (OPT-IN, `--select T089`):** a faster script reading
//!   a channel written only by a slower one sees stale values between writer
//!   ticks. M1 Build does NOT flag this — downsampled inputs are intentional and
//!   accepted — so default-on would diverge from M1 Build; it stays an opt-in
//!   exploration aid.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make_project_for};
use crate::project::Project;
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use crate::typer::{is_expr, path_text};
use m1_core::{Field, Kind, Node, Severity};
use std::collections::{BTreeMap, BTreeSet};

/// Per-script channel access sets, resolved against the project.
struct ScriptIo {
    fn_path: String,
    rate_hz: Option<f64>,
    writes: BTreeSet<String>,
    reads: BTreeSet<String>,
    /// User functions/methods this script calls (resolved symbol paths) — the
    /// edges of the T097 call graph.
    calls: BTreeSet<String>,
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
        calls: BTreeSet::new(),
    };
    collect(cst.root(), &scope, &mut Vec::new(), &mut io);
    Some(io)
}

/// Active `expand` variable bindings, innermost last: `(name, start, end)`.
type ExpandBindings = Vec<(String, i64, i64)>;

/// The `(variable, start, end)` binding of an `expand` statement whose bounds
/// are integer literals. Constant-named bounds (allowed by the manual) are not
/// evaluated — their bodies keep the pre-#170 behaviour (templated paths stay
/// unresolved).
fn expand_binding(n: &Node) -> Option<(String, i64, i64)> {
    let var = n.child_by_field(Field::Variable)?.text().trim().to_string();
    let lo: i64 = n.child_by_field(Field::Start)?.text().trim().parse().ok()?;
    let hi: i64 = n.child_by_field(Field::End)?.text().trim().parse().ok()?;
    // A degenerate or huge range is not worth enumerating (T084 flags bad
    // bounds; real corpora use single-digit ranges).
    (lo <= hi && hi - lo < 256).then_some((var, lo, hi))
}

/// Every compile-time substitution of `$(VAR)` templates in `text` under the
/// active `expand` bindings — `expand` is text substitution (manual p.33), so
/// `Seg $(SEG).Volt` under `expand (SEG = 1 to 2)` accesses `Seg 1.Volt` AND
/// `Seg 2.Volt`. Text without templates passes through as itself; a runaway
/// combination count returns empty (treated as unresolvable).
fn substituted(text: &str, bindings: &ExpandBindings) -> Vec<String> {
    if !text.contains("$(") {
        return vec![text.to_string()];
    }
    let mut variants = vec![text.to_string()];
    for (var, lo, hi) in bindings {
        let needle = format!("$({var})");
        if !variants[0].contains(&needle) {
            continue;
        }
        let mut next = Vec::with_capacity(variants.len() * (hi - lo + 1) as usize);
        for v in &variants {
            for val in *lo..=*hi {
                next.push(v.replace(&needle, &val.to_string()));
            }
        }
        variants = next;
        if variants.len() > 4096 {
            return Vec::new();
        }
    }
    variants
}

/// Every channel/parameter symbol path `n` accesses, after `expand`
/// substitution: one path for a plain reference, several for a templated one.
fn channel_paths(n: Node, scope: &Scope, bindings: &ExpandBindings) -> Vec<String> {
    if !matches!(n.kind(), Kind::Identifier | Kind::MemberExpression) {
        return Vec::new();
    }
    substituted(&path_text(n), bindings)
        .iter()
        .filter_map(|p| resolve_channel(p, scope))
        .collect()
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

fn collect(n: Node, scope: &Scope, bindings: &mut ExpandBindings, io: &mut ScriptIo) {
    // `expand` body: bind the loop variable so `$(VAR)` templates in access
    // paths substitute to the channels M1 Build's compile-time expansion
    // actually touches (#170 — the EV-M1 T093/T094 false positives).
    if n.kind() == Kind::ExpandStatement
        && let Some(b) = expand_binding(&n)
    {
        bindings.push(b);
        for c in n.children() {
            collect(c, scope, bindings, io);
        }
        bindings.pop();
        return;
    }
    if n.kind() == Kind::AssignmentStatement {
        let kids = n.named_children();
        let target = kids.iter().find(|c| is_expr(c.kind()));
        let compound = !n.children().iter().any(|c| c.kind() == Kind::Assign);
        if let Some(t) = target {
            for path in channel_paths(*t, scope, bindings) {
                if compound {
                    io.reads.insert(path.clone()); // `+=` reads the target first
                }
                io.writes.insert(path);
            }
            // Walk only the value side for reads.
            for c in kids.iter().filter(|c| !std::ptr::eq(*c, t)) {
                collect(*c, scope, bindings, io);
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
        // T097: a callee that resolves to a user function/method is a call-graph
        // edge. Recorded in addition to (not instead of) the read/write logic
        // below — `Chan.Set(…)` still counts as a channel write, and the
        // arguments are still walked for reads either way.
        for variant in substituted(&text, bindings) {
            if let Resolution::Symbol(s) = resolve(&variant, scope)
                && matches!(s.kind, SymbolKind::Function | SymbolKind::Method)
            {
                io.calls.insert(s.path.clone());
            }
        }
        if let Some((receiver, method)) = text.rsplit_once('.') {
            let chans: Vec<String> = substituted(receiver, bindings)
                .iter()
                .filter_map(|r| resolve_channel(r, scope))
                .collect();
            if !chans.is_empty() {
                for chan in chans {
                    if method.starts_with("Set") {
                        io.writes.insert(chan);
                    } else {
                        io.reads.insert(chan);
                    }
                }
                // Reads in the arguments still count; skip the callee (already handled).
                if let Some(args) = n
                    .children()
                    .into_iter()
                    .find(|c| c.kind() == Kind::ArgumentList)
                {
                    collect(args, scope, bindings, io);
                }
                return;
            }
        }
    }
    let read_paths = channel_paths(n, scope, bindings);
    if !read_paths.is_empty() {
        io.reads.extend(read_paths);
        return; // a member chain resolves as a whole; don't descend
    }
    for c in n.children() {
        collect(c, scope, bindings, io);
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
    t097: bool,
) -> Vec<TypeDiagnostic> {
    if !t088 && !t089 && !t097 {
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
    // T097: cycles in the user-function CALL graph (manual: scripts are
    // event-scheduled functions on a fixed-stack runtime — a call cycle can
    // never complete). Same DFS shape as T088, but over call edges, rates are
    // irrelevant, and self-edges (a function calling itself) are kept.
    if t097 {
        let index_of: BTreeMap<&str, usize> = ios
            .iter()
            .enumerate()
            .map(|(i, io)| (io.fn_path.as_str(), i))
            .collect();
        let mut call_edges: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for (i, io) in ios.iter().enumerate() {
            for callee in &io.calls {
                if let Some(&j) = index_of.get(callee.as_str()) {
                    call_edges.entry(i).or_default().insert(j);
                }
            }
        }
        let mut reported: BTreeSet<String> = BTreeSet::new();
        for &start in call_edges.keys() {
            let mut stack = vec![(start, vec![start])];
            while let Some((node, path)) = stack.pop() {
                for &next in call_edges.get(&node).into_iter().flatten() {
                    if next == start {
                        let mut names: Vec<&str> =
                            path.iter().map(|&i| ios[i].fn_path.as_str()).collect();
                        let anchor = names.iter().min().unwrap_or(&"").to_string();
                        if reported.insert(anchor.clone()) {
                            names.push(ios[start].fn_path.as_str());
                            out.push(make_project_for(
                                TypeCode::T097,
                                Severity::Error,
                                format!(
                                    "recursive user-function call cycle: {}",
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
    }
    out.sort_by(|a, b| a.inner.message.cmp(&b.inner.message));
    out
}

/// Project-wide usage audit (default-on): channels no script writes (T093,
/// mirroring M1 Build Error 1627 "Channel value not assigned") and parameters no
/// script reads (T094, M1 Build Error 1631 "Parameter value not read").
///
/// Matches M1 Build (0 on the real AV-M1 project, like M1 Build) once writes/reads
/// are detected correctly — `Channel.Set*(…)` and `This.`-qualified access count
/// (see `script_io`) — and the obvious non-script value sources are exempt: CAN
/// signals (`BuiltIn.CAN.*`), package-object members (MoTeC Input/Output sensors,
/// via `crate::audit::ObjectOwnership`), and a Table / value-compound's
/// auto-created `.Value` child. Only plain `BuiltIn.Channel`/`BuiltIn.Parameter`
/// are considered.
///
/// IMPORTANT: `scripts` must be the COMPLETE set of project scripts (the CLI gathers
/// every `.m1scr` under the project, not just the files on the command line) — a
/// missing script would make the channels it writes look never-assigned. The `t093`/
/// `t094` flags let a caller still scope to one code.
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
                    Severity::Error,
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
                    Severity::Error,
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

/// Cross-script writer audit (default-on, T096): a channel assigned by more
/// than one *periodically scheduled* function — M1 Build Error 1022 "Object
/// assigned to from multiple periodically scheduled functions", which fails
/// the build. The intra-script complement is T040 (multiple assignments on
/// one code path). Only functions with a known trigger rate count: a Startup
/// or event function initialising a channel one scheduled function owns is
/// not a 1022 (and an unparseable rate conservatively stays silent).
pub fn check_multi_writers(project: &Project, scripts: &[(String, String)]) -> Vec<TypeDiagnostic> {
    let ios: Vec<ScriptIo> = scripts
        .iter()
        .filter_map(|(f, s)| script_io(project, f, s))
        .filter(|io| io.rate_hz.is_some())
        .collect();
    let mut writers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for io in &ios {
        for w in &io.writes {
            writers.entry(w).or_default().insert(io.fn_path.as_str());
        }
    }
    let mut out = Vec::new();
    for (chan, fns) in &writers {
        if fns.len() < 2 {
            continue;
        }
        let list = fns
            .iter()
            .map(|f| format!("`{f}`"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push(make_project_for(
            TypeCode::T096,
            Severity::Error,
            format!(
                "channel `{chan}` is assigned by multiple periodically scheduled functions: {list} (M1 Build Error 1022)"
            ),
            *chan,
        ));
    }
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
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Ctrl.Seg 1"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Ctrl.Seg 2"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Ctrl.Seg 1.Volt"><Props Type="f32" Security="Tune"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Ctrl.Seg 2.Volt"><Props Type="f32" Security="Tune"/></Component>
  <Component Classname="BuiltIn.Parameter" Name="Root.Ctrl.Seg 1.Gain"><Props Type="f32" Security="Tune"/></Component>
  <Component Classname="BuiltIn.Parameter" Name="Root.Ctrl.Seg 2.Gain"><Props Type="f32" Security="Tune"/></Component>
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
    fn expand_substituted_access_counts_for_usage_audit() {
        // EV-M1 pattern (#170): `expand` is compile-time text substitution
        // (manual p.33), so a write/read whose path embeds `$(VAR)` reaches
        // every substituted channel — M1 Build sees the assignments and builds.
        // Both `Seg N.Volt` channels are written and both `Seg N.Gain`
        // parameters read; none may be flagged.
        let msgs = run(
            "Written = 1.0;\nSetWritten.Set(2.0);\nlocal r = ReadParam;\nlocal d = This.Debounce;\nexpand (SEG = 1 to 2)\n{\n\tSeg $(SEG).Volt = Seg $(SEG).Gain;\n}\n",
        );
        assert!(
            !msgs.iter().any(|m| m.contains("`Root.Ctrl.Seg")),
            "expand-substituted access must count: {msgs:?}"
        );
    }

    #[test]
    fn t093_t094_are_errors_like_m1_build_1627_1631() {
        // M1 Build fails the build on 1627/1631, so the parity diagnostics
        // must be Errors (non-zero exit), not Warnings.
        let project = Project::from_xml(XML).unwrap();
        let scripts = vec![("Ctrl.Update.m1scr".to_string(), String::new())];
        let diags = check_usage(&project, &scripts, true, true);
        assert!(!diags.is_empty(), "empty script must flag orphans");
        for d in &diags {
            assert_eq!(
                d.inner.severity,
                Severity::Error,
                "{:?} must be Error: {}",
                d.inner.code,
                d.inner.message
            );
        }
    }

    #[test]
    fn multi_writer_channel_is_flagged_across_scheduled_functions() {
        // Probe from #177: `Root.Ctrl.Written` assigned by two periodically
        // scheduled functions → M1 Build Error 1022 (build fails). A channel
        // with a single scheduled writer must not be flagged.
        let project = Project::from_xml(XML_TWO_FNS).unwrap();
        let scripts = vec![
            (
                "Ctrl.Update.m1scr".to_string(),
                "Written = 1.0;\nOnly Mine = 2.0;\n".to_string(),
            ),
            (
                "Aux.Update.m1scr".to_string(),
                "Ctrl.Written = 3.0;\n".to_string(),
            ),
        ];
        let diags = check_multi_writers(&project, &scripts);
        assert_eq!(diags.len(), 1, "{diags:?}");
        let d = &diags[0];
        assert_eq!(d.inner.severity, Severity::Error);
        assert!(
            d.inner.message.contains("`Root.Ctrl.Written`")
                && d.inner.message.contains("Root.Ctrl.Update")
                && d.inner.message.contains("Root.Aux.Update"),
            "{}",
            d.inner.message
        );
    }

    #[test]
    fn unscheduled_second_writer_is_not_1022() {
        // M1 Build 1022 is specific to *periodically scheduled* functions: a
        // Startup function (no trigger rate) initialising a channel that one
        // scheduled function owns is fine.
        let project = Project::from_xml(XML_TWO_FNS).unwrap();
        let scripts = vec![
            (
                "Ctrl.Update.m1scr".to_string(),
                "Written = 1.0;\n".to_string(),
            ),
            (
                "Aux.Startup.m1scr".to_string(),
                "Ctrl.Written = 0.0;\n".to_string(),
            ),
        ];
        assert!(check_multi_writers(&project, &scripts).is_empty());
    }

    const XML_TWO_FNS: &str = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Ctrl"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Aux"/>
  <Component Classname="BuiltIn.FuncUser" Filename="Ctrl.Update.m1scr" Name="Root.Ctrl.Update"><Props SelectedTrigger="Root.Events.On 100Hz"/></Component>
  <Component Classname="BuiltIn.FuncUser" Filename="Aux.Update.m1scr" Name="Root.Aux.Update"><Props SelectedTrigger="Root.Events.On 50Hz"/></Component>
  <Component Classname="BuiltIn.FuncUser" Filename="Aux.Startup.m1scr" Name="Root.Aux.Startup"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Ctrl.Written"><Props Type="f32" Security="Tune"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Ctrl.Only Mine"><Props Type="f32" Security="Tune"/></Component>
</Project>"#;

    #[test]
    fn flags_gate_each_code_independently() {
        // The t093/t094 flags scope the audit to one code (e.g. for `--select`);
        // neither selected → no work, no findings.
        let project = Project::from_xml(XML).unwrap();
        let scripts = vec![("Ctrl.Update.m1scr".to_string(), String::new())];
        assert!(check_usage(&project, &scripts, false, false).is_empty());
    }
}
