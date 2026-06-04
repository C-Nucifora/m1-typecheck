//! Rule trait, registry, and the single-walk runner.
use crate::diagnostics::{CheckResult, TypeDiagnostic};
use crate::project::Project;
use crate::resolve::Scope;
use m1_core::Node;
use std::path::Path;

pub mod t001_unresolved;
pub mod t002_float_eq;
pub mod t003_int_float_mix;
pub mod t004_signed_unsigned_cmp;
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
    /// `--select`/`[diagnostics] select`), never by default. Currently just T064.
    pub fn opt_in_codes() -> &'static [crate::diagnostics::TypeCode] {
        use crate::diagnostics::TypeCode::*;
        &[T064]
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
                Box::new(t020_enum_member::Rule),
                Box::new(t021_enum_numeric_cmp::Rule),
                Box::new(t030_assign_mismatch::Rule),
                Box::new(t042_dbc_signal_range::Rule),
                Box::new(t060_stateful_conditional::Rule),
                Box::new(t061_integrated_only::Rule),
                Box::new(t062_deprecated_overload::Rule),
                Box::new(t063_calibration_only::Rule),
                Box::new(t070_when_exhaustive::Rule),
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
fn collect_locals(root: Node) -> std::collections::HashMap<String, crate::types::ValueType> {
    use crate::types::ValueType;
    use m1_core::Field;
    let mut locals: std::collections::HashMap<String, ValueType> = std::collections::HashMap::new();
    fn walk(n: Node, locals: &mut std::collections::HashMap<String, ValueType>) {
        if n.kind() == m1_core::Kind::LocalDeclaration
            && let Some(name_node) = n.child_by_field(Field::Name)
        {
            let name = name_node.text().to_string();
            // The `value` field is the initializer expression (the grammar names
            // it; a bare `local x;` has none). Using the field avoids the old
            // kind-exclusion heuristic, which dropped Identifier initialisers
            // (`local b = a;`) and so left every derived local Unknown.
            let t = match n.child_by_field(Field::Value) {
                // Type against the locals declared so far (declaration-order
                // threading) so a backward reference resolves; the name being
                // declared is not yet inserted, so `local x = x;` stays Unknown.
                Some(init) => crate::typer::type_of(
                    init,
                    &Scope {
                        locals: locals.clone(),
                        group: None,
                        project: None,
                    },
                ),
                None => ValueType::Unknown,
            };
            locals.insert(name, t);
        }
        for c in n.children() {
            walk(c, locals);
        }
    }
    walk(root, &mut locals);
    locals
}

/// Check one script. `script_path` (file name) anchors group-relative resolution.
pub fn check_script(project: &Project, script_path: &Path, source: &str) -> CheckResult {
    run_with(
        &Registry::default(),
        Some(project),
        script_path.file_name().and_then(|s| s.to_str()),
        source,
    )
}

pub fn check_script_no_project(source: &str) -> CheckResult {
    run_with(&Registry::default(), None, None, source)
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
    run_with(
        &Registry::with_opt_in(enabled),
        project,
        script_path
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str()),
        source,
    )
}

/// Run a specific `registry` over `source`.
pub fn run_with(
    registry: &Registry,
    project: Option<&Project>,
    file_name: Option<&str>,
    source: &str,
) -> CheckResult {
    let cst = m1_core::parse(source);
    let syntax_errors = cst.syntax_diagnostics();
    if !syntax_errors.is_empty() {
        return CheckResult {
            diagnostics: vec![],
            syntax_errors,
        };
    }
    let group = match (project, file_name) {
        (Some(p), Some(f)) => p.group_for_script(f),
        _ => None,
    };
    let scope = Scope {
        locals: collect_locals(cst.root()),
        group,
        project,
    };
    let mut diagnostics = Vec::new();
    walk(cst.root(), registry, &scope, &mut diagnostics);
    crate::flow::single_assignment(cst.root(), &scope, &mut diagnostics);
    suppress_allowed(source, &cst, &mut diagnostics);
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
fn suppress_allowed(source: &str, cst: &m1_core::Cst, diagnostics: &mut Vec<TypeDiagnostic>) {
    let anns = m1_core::annotations(cst, &m1_core::Registry::seed());
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
}
