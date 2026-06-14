//! Rule trait, registry, and the single-walk runner.
use crate::diagnostics::{CheckResult, TypeDiagnostic, make};
use crate::project::Project;
use crate::resolve::Scope;
use m1_core::Node;
use std::path::Path;

pub mod t001_unresolved;
pub mod t002_float_eq;
pub mod t003_int_float_mix;
pub mod t004_signed_unsigned_cmp;
pub mod t005_modulo_on_float;
pub mod t020_enum_member;
pub mod t021_enum_numeric_cmp;
pub mod t030_assign_mismatch;
pub mod t042_dbc_signal_range;
pub mod t060_stateful_conditional;
pub mod t061_integrated_only;
pub mod t062_deprecated_overload;
pub mod t063_calibration_only;
pub mod t064_arg_count;
pub mod t070_when_exhaustive;
pub mod t083_static_local_init;
pub mod t084_expand_bounds;
pub mod t085_user_arg_mismatch;
pub mod t086_unit_mismatch;
pub mod t091_local_case;

pub trait Rule: Send + Sync {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>);
}

pub struct Registry {
    rules: Vec<Box<dyn Rule>>,
}

impl Registry {
    pub fn rules(&self) -> &[Box<dyn Rule>] {
        &self.rules
    }

    /// The opt-in T-codes — rules that run only when explicitly selected (via
    /// `--select`/`[diagnostics] select`), never by default:
    /// - **T064** (a per-node arg-count rule).
    /// - **T089** rate-inversion (`schedule::check`): M1 Build does NOT flag a
    ///   faster script reading a slower-written channel — downsampled reads are
    ///   intentional and accepted — so default-on would diverge from M1 Build.
    ///
    /// T088 (circular dependency = M1 Build 1640), T092 (untagged = M1 Build
    /// 1142/1549), T093/T094 (unassigned/unread = M1 Build 1627/1631),
    /// T095 (invalid display unit = M1 Build 1017), T096 (multiple scheduled
    /// writers = M1 Build 1022), and T097 (user-function call cycle) are now
    /// **default-on**: each mirrors a finding M1 Build itself emits, and the
    /// cross-script checks source the whole project's scripts so they no longer
    /// false-positive under a partial invocation.
    pub fn opt_in_codes() -> &'static [crate::diagnostics::TypeCode] {
        use crate::diagnostics::TypeCode::*;
        &[T064, T089]
    }

    /// The default rule set plus any opt-in rules whose code is in `enabled`. Used
    /// by `check_script_with` so the CLI/LSP can activate an opt-in rule per-run.
    pub fn with_opt_in(enabled: &std::collections::HashSet<String>) -> Self {
        let mut reg = Registry::default();
        if enabled.contains("T064") {
            reg.rules.push(Box::new(t064_arg_count::Rule));
        }
        reg
    }

    /// Every rule, including opt-in ones — for exhaustive tests.
    #[cfg(test)]
    pub fn all() -> Self {
        let mut reg = Registry::default();
        reg.rules.push(Box::new(t064_arg_count::Rule));
        reg
    }
}

impl Default for Registry {
    /// The active rule set used by `check_script` — every always-on rule. Opt-in
    /// rules (see [`Registry::opt_in_codes`]) are excluded; add them with
    /// [`Registry::with_opt_in`].
    fn default() -> Self {
        Registry {
            rules: vec![
                Box::new(t001_unresolved::Rule),
                Box::new(t002_float_eq::Rule),
                Box::new(t003_int_float_mix::Rule),
                Box::new(t004_signed_unsigned_cmp::Rule),
                Box::new(t005_modulo_on_float::Rule),
                Box::new(t020_enum_member::Rule),
                Box::new(t021_enum_numeric_cmp::Rule),
                Box::new(t030_assign_mismatch::Rule),
                Box::new(t042_dbc_signal_range::Rule),
                Box::new(t060_stateful_conditional::Rule),
                Box::new(t061_integrated_only::Rule),
                Box::new(t062_deprecated_overload::Rule),
                Box::new(t063_calibration_only::Rule),
                Box::new(t070_when_exhaustive::Rule),
                Box::new(t083_static_local_init::Rule),
                Box::new(t085_user_arg_mismatch::Rule),
                Box::new(t086_unit_mismatch::Rule),
                Box::new(t084_expand_bounds::Rule),
            ],
        }
    }
}

/// Collect locals (name -> type) declared anywhere in the script. The type is
/// inferred from the declaration's initialiser via the expression typer.
///
/// Locals are typed in **declaration order**, threading an incrementally-built
/// scope of the locals already seen. So a *backward* reference resolves
/// (`local b = a;` after `local a = 1.5;` types `b` as Float), while a *forward*
/// reference stays Unknown (the not-yet-declared name isn't in scope). Declaration
/// order is the safe direction — it can't create the cross-local ordering hazard
/// (a forward dependency typing wrongly) the prior empty-scope guard avoided.
pub(crate) fn collect_locals(
    root: Node,
    project: Option<&Project>,
    group: Option<&str>,
) -> std::collections::HashMap<String, crate::types::ValueType> {
    use crate::types::ValueType;
    use m1_core::Field;
    let mut locals: std::collections::HashMap<String, ValueType> = std::collections::HashMap::new();
    fn walk(
        n: Node,
        locals: &mut std::collections::HashMap<String, ValueType>,
        project: Option<&Project>,
        group: Option<&str>,
    ) {
        // `expand (VAR = start to end) { … }` introduces a compile-time loop
        // variable `VAR` (an integer counter), referenced in the body as
        // `$(VAR)`. It is function-local, not a project symbol, so register it as
        // a local to keep it out of unresolved-reference checks (#109).
        if n.kind() == m1_core::Kind::ExpandStatement
            && let Some(var) = n.child_by_field(Field::Variable)
        {
            locals.insert(var.text().to_string(), ValueType::Integer);
        }
        if n.kind() == m1_core::Kind::LocalDeclaration
            && let Some(name_node) = n.child_by_field(Field::Name)
        {
            let name = name_node.text().to_string();
            // An explicit `local <Type> x = …;` annotation (manual p.34) is
            // authoritative: the declared type wins over initialiser inference
            // (#142 — `local <Float> x = 1;` is Float, not Integer). Unknown
            // annotation names fall back to inference.
            if let Some(declared) = n
                .child_by_field(Field::TypeAnnotation)
                .and_then(|a| a.child_by_field(Field::Type))
                .and_then(|t| crate::types::declared_local_type(t.text()))
            {
                locals.insert(name, declared);
                for c in n.children() {
                    walk(c, locals, project, group);
                }
                return;
            }
            // The `value` field is the initializer expression (the grammar names
            // it; a bare `local x;` has none). Using the field avoids the old
            // kind-exclusion heuristic, which dropped Identifier initialisers
            // (`local b = a;`) and so left every derived local Unknown.
            let t = match n.child_by_field(Field::Value) {
                // A *call* initializer is typed with the project in scope so a
                // user-function call resolves to its inferred return type (#110);
                // built-in calls already resolve project-lessly (intrinsics) so
                // this only adds user-function returns — it does NOT type
                // channel-read locals (`local x = Some Channel;` stays Unknown,
                // which is the separate concern of project-aware local typing).
                // Everything else stays project-less, typed only against the
                // locals declared so far (declaration-order threading), so a
                // backward reference resolves while `local x = x;` stays Unknown.
                Some(init) => {
                    let (g, p) = if init.kind() == m1_core::Kind::CallExpression {
                        (group, project)
                    } else {
                        (None, None)
                    };
                    crate::typer::type_of(
                        init,
                        &Scope {
                            locals: locals.clone(),
                            group: g.map(str::to_string),
                            project: p,
                            fn_symbol: None,
                        },
                    )
                }
                None => ValueType::Unknown,
            };
            locals.insert(name, t);
        }
        for c in n.children() {
            walk(c, locals, project, group);
        }
    }
    walk(root, &mut locals, project, group);
    locals
}

/// Check one script. `script_path` (file name) anchors group-relative resolution.
pub fn check_script(project: &Project, script_path: &Path, source: &str) -> CheckResult {
    run_with(
        &Registry::default(),
        Some(project),
        script_path.file_name().and_then(|s| s.to_str()),
        source,
        &crate::cross_script::ChannelTaints::default(),
    )
}

pub fn check_script_no_project(source: &str) -> CheckResult {
    run_with(
        &Registry::default(),
        None,
        None,
        source,
        &crate::cross_script::ChannelTaints::default(),
    )
}

/// Like [`check_script`] but with opt-in rules activated for the T-codes in
/// `enabled` (e.g. `T064`). The CLI passes its resolved `--select`/`[diagnostics]`
/// set so an opt-in rule runs when, and only when, the caller asks for it.
pub fn check_script_with(
    enabled: &std::collections::HashSet<String>,
    project: Option<&Project>,
    script_path: Option<&Path>,
    source: &str,
) -> CheckResult {
    check_script_with_channels(
        enabled,
        project,
        script_path,
        source,
        &crate::cross_script::ChannelTaints::default(),
    )
}

/// Like [`check_script_with`] but with the project-wide solved channel taints
/// seeded into the invalid-value analysis, so cross-script T080/T081 fire at
/// this file's sinks with their provenance chains (#78 P3). Build `channels`
/// once per run with [`crate::cross_script::solve`].
pub fn check_script_with_channels(
    enabled: &std::collections::HashSet<String>,
    project: Option<&Project>,
    script_path: Option<&Path>,
    source: &str,
    channels: &crate::cross_script::ChannelTaints,
) -> CheckResult {
    run_with(
        &Registry::with_opt_in(enabled),
        project,
        script_path
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str()),
        source,
        channels,
    )
}

/// Run a specific `registry` over `source`.
pub fn run_with(
    registry: &Registry,
    project: Option<&Project>,
    file_name: Option<&str>,
    source: &str,
    channels: &crate::cross_script::ChannelTaints,
) -> CheckResult {
    // A leading UTF-8 BOM (U+FEFF) is metadata, not content: Windows editors add
    // it routinely. Strip one leading BOM here, the single parse choke-point, so
    // column 1 maps to the document's first real character regardless of how the
    // source reached us. The CLI read path already strips it via
    // `m1_workspace::read_text`, and tree-sitter currently consumes a stray BOM as
    // whitespace, but in-process callers (the LSP) pass source strings directly
    // and must not depend on that grammar detail (#213). Done locally, so it needs
    // no m1-workspace release.
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let cst = m1_core::parse(source);
    let syntax_errors = cst.syntax_diagnostics();
    if !syntax_errors.is_empty() {
        return CheckResult {
            diagnostics: vec![],
            syntax_errors,
        };
    }
    // The analysis walkers below (`collect_locals`, `walk`, `flow`, `invalid_value`)
    // recurse over the CST proportional to its nesting depth. A syntactically-valid
    // but pathologically deep expression (tens of thousands of nested parens, a long
    // `a+a+…` chain, …) would overflow the stack and SIGABRT the process. m1-core's
    // `max_depth()` is computed iteratively, so we can probe the depth safely and, if
    // it exceeds the cap that recursion can handle, emit one diagnostic and skip the
    // deep analysis rather than recurse into a crash (#94).
    if cst.root().max_depth() > m1_core::MAX_RECURSION_DEPTH {
        let mut diagnostics = vec![make(
            crate::diagnostics::TypeCode::T090,
            &cst.root(),
            m1_core::Severity::Warning,
            "expression nesting too deep to analyze; this file's type checks were \
             skipped to avoid a stack overflow — reduce the nesting depth"
                .into(),
        )];
        // `@allow(T090)` / a bare `@allow` can still silence it.
        let anns = m1_core::annotations(&cst, &m1_core::Registry::seed());
        suppress_allowed(&anns, source, &mut diagnostics);
        return CheckResult {
            diagnostics,
            syntax_errors,
        };
    }
    let group = match (project, file_name) {
        (Some(p), Some(f)) => p.group_for_script(f),
        _ => None,
    };
    // The backing function symbol anchors `In.<Param>` resolution (#110).
    let fn_symbol = match (project, file_name) {
        (Some(p), Some(f)) => p.function_symbol_for_script(f),
        _ => None,
    };
    let scope = Scope {
        locals: collect_locals(cst.root(), project, group.as_deref()),
        group,
        project,
        fn_symbol,
    };
    let mut diagnostics = Vec::new();
    walk(cst.root(), registry, &scope, &mut diagnostics);
    crate::flow::single_assignment(cst.root(), &scope, &mut diagnostics);
    t091_local_case::check(cst.root(), &scope, &mut diagnostics);

    // Parse `@m1:` annotations once and drive both consumers: the invalid-value
    // (NaN/Inf) provenance analysis (T080/T081, #78) reads the finiteness
    // sinks/sources — seeded with the project-wide solved channel taints so
    // cross-script provenance reaches this file's sinks — then `@allow`
    // suppression filters the final diagnostic set.
    let anns = m1_core::annotations(&cst, &m1_core::Registry::seed());
    if !crate::invalid_value::can_skip(cst.root(), &anns, channels) {
        crate::invalid_value::check_with_channels(
            cst.root(),
            &scope,
            &anns,
            channels,
            None,
            &mut diagnostics,
        );
    }
    suppress_allowed(&anns, source, &mut diagnostics);
    CheckResult {
        diagnostics,
        syntax_errors,
    }
}

/// Drop diagnostics suppressed by an `// @m1:allow(T0xx, …)` annotation
/// (m1-core#33). `@allow(T002)` suppresses only the listed codes; a bare
/// `@allow` suppresses every code on its target construct. Suppression is
/// line-scoped to the annotated construct.
///
/// Project-level diagnostics (T041/T042 — zero byte range, not tied to a source
/// construct) are **not** suppressible this way; quiet those with `--ignore` /
/// the `[diagnostics]` config instead.
///
/// Parsing uses the seed registry so annotations owned by other tools are not
/// treated as unknown here.
fn suppress_allowed(
    anns: &m1_core::Annotations,
    source: &str,
    diagnostics: &mut Vec<TypeDiagnostic>,
) {
    let spans: Vec<(u32, u32, &m1_core::Annotation)> = anns
        .all()
        .iter()
        .filter(|a| a.kind == "allow")
        .filter_map(|a| {
            let t = a.target_byte_range.as_ref()?;
            Some((
                byte_line(source, t.start),
                byte_line(source, t.end.saturating_sub(1)),
                a,
            ))
        })
        .collect();
    if spans.is_empty() {
        return;
    }
    diagnostics.retain(|d| {
        if d.inner.byte_range.is_empty() {
            return true; // project-level diagnostic; not @allow-suppressible
        }
        let line = d.inner.range.start.line;
        let code = d.code.as_str();
        !spans.iter().any(|(start, end, a)| {
            line >= *start && line <= *end && (a.args.is_empty() || a.has_positional(code))
        })
    });
}

/// 0-based line number of `byte` within `source` (count of preceding newlines).
fn byte_line(source: &str, byte: usize) -> u32 {
    let b = byte.min(source.len());
    source.as_bytes()[..b]
        .iter()
        .filter(|&&c| c == b'\n')
        .count() as u32
}

fn walk(node: Node, registry: &Registry, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    for rule in registry.rules() {
        rule.check_node(&node, scope, out);
    }
    for c in node.children() {
        walk(c, registry, scope, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_script_no_project_has_no_diagnostics_for_resolution() {
        // simple int math -> no T002/T003 and no spurious diagnostics
        let r = check_script_no_project("local sum = 1 + 2;\n");
        assert!(r.syntax_errors.is_empty());
        assert!(
            r.diagnostics.is_empty(),
            "expected no diagnostics, got {:?}",
            r.diagnostics
        );
    }

    fn has(r: &CheckResult, code: &str) -> bool {
        r.diagnostics.iter().any(|d| d.code.as_str() == code)
    }

    #[test]
    fn allow_annotation_suppresses_listed_type_code() {
        // `local b = 1.5; local c = 2.5; if (b == c)` fires T002 (float ==).
        let dirty = "local b = 1.5;\nlocal c = 2.5;\nif (b == c) { }\n";
        assert!(has(&check_script_no_project(dirty), "T002"));

        let allowed = "local b = 1.5;\nlocal c = 2.5;\n// @m1:allow(T002)\nif (b == c) { }\n";
        assert!(!has(&check_script_no_project(allowed), "T002"));
    }

    #[test]
    fn allow_annotation_does_not_suppress_other_codes() {
        // Allow an unrelated code; T002 must still fire.
        let src = "local b = 1.5;\nlocal c = 2.5;\n// @m1:allow(T030)\nif (b == c) { }\n";
        assert!(has(&check_script_no_project(src), "T002"));
    }

    #[test]
    fn bare_allow_suppresses_every_code_on_target() {
        let src = "local b = 1.5;\nlocal c = 2.5;\n// @m1:allow\nif (b == c) { }\n";
        assert!(!has(&check_script_no_project(src), "T002"));
    }

    // ── #142: explicit `local <Type>` annotations are authoritative ────────

    #[test]
    fn declared_local_type_wins_over_initialiser_inference() {
        // Both locals are integer-initialised; only the declared <Float> type
        // makes this a float equality (T002).
        let src = "local <Float> b = 1;\nlocal <Float> c = 2;\nif (b == c) { }\n";
        assert!(has(&check_script_no_project(src), "T002"));
    }

    #[test]
    fn declared_integer_local_with_float_initialiser_is_t030() {
        let src = "local <Integer> x = 1.5;\n";
        assert!(has(&check_script_no_project(src), "T030"));
    }

    #[test]
    fn negative_initialiser_on_unsigned_local_is_t030() {
        let src = "local <Unsigned Integer> x = -1;\n";
        assert!(has(&check_script_no_project(src), "T030"));
    }

    #[test]
    fn corpus_annotation_spellings_are_clean() {
        // Real-corpus shapes: lowercase <boolean>, <Unsigned Integer> with hex,
        // <Integer> with a negative literal — all well-typed, no diagnostics.
        let src = "local <boolean> ok = false;\nlocal <Unsigned Integer> h = 0x00;\nlocal <Integer> i = -1;\nlocal <Float> f = 1;\n";
        let r = check_script_no_project(src);
        assert!(
            r.diagnostics.is_empty(),
            "expected clean, got {:?}",
            r.diagnostics
        );
    }
}
