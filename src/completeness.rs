//! Analysis-completeness telemetry (issue #259).
//!
//! `Unknown` is deliberately absorbing and silent (see AGENTS.md): a rule fires
//! only when every type/enum it needs is *known*, so gaps in the symbol data
//! produce silence rather than a guess. That is the right default, but it means
//! a clean run ("no diagnostics") can be mistaken for a *complete* analysis when
//! it is really a *silent* one — the checker simply couldn't see enough.
//!
//! This module measures that silent surface without changing any diagnostic. It
//! re-runs the same typing/resolution machinery the rules use ([`type_of`],
//! [`resolve`]) purely to *count* how much of the project it could pin down:
//! how many expressions typed, how many references resolved vs. stayed
//! opaque/unresolved, how many intrinsic calls hit the catalogue, and how much
//! enum membership it could verify. The result is an opt-in report (`--completeness`),
//! never a gate — it reports coverage, it does not add findings.
//!
//! Everything here reads existing data structures; it introduces no
//! instrumentation into the type checker itself.

use crate::intrinsics;
use crate::parsed::ParsedScript;
use crate::project::Project;
use crate::resolve::{Resolution, Scope, resolve};
use crate::typer::{is_expr, path_text, type_of};
use crate::types::ValueType;
use m1_core::{Field, Kind, Node};

/// Coverage telemetry for one `m1-typecheck` run: how much of the analysed
/// scripts the checker could actually pin down to a known type / resolved
/// symbol / catalogued intrinsic, versus the silent `Unknown`/opaque surface.
///
/// Every field is a plain count derived by replaying [`type_of`]/[`resolve`]
/// over the parsed scripts — deterministic for a given input, so it is safe to
/// assert on in tests and to diff across runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompletenessReport {
    /// Scripts considered (the whole project set the run was given).
    pub scripts_total: usize,
    /// Scripts skipped because they carry syntax errors (no semantic analysis
    /// runs on them, exactly as in a normal check).
    pub scripts_with_syntax_errors: usize,
    /// Scripts skipped because their nesting exceeds
    /// [`m1_core::MAX_RECURSION_DEPTH`] — the same depth cap `run_with` uses to
    /// avoid a stack overflow (T090). Their expressions are *not* counted below.
    pub scripts_skipped_deep: usize,

    /// Total expression nodes across the analysed scripts.
    pub expressions_total: usize,
    /// Expression nodes the typer gave a *known* (non-`Unknown`) type.
    pub expressions_typed: usize,

    /// Maximal identifier / member-expression paths (references) encountered.
    pub references_total: usize,
    /// References that resolved to a local, project symbol, or built-in
    /// object/function.
    pub references_resolved: usize,
    /// References that resolved to an *opaque* root (a firmware/library/unknown
    /// root the model deliberately never flags — but also never verifies).
    pub references_opaque: usize,
    /// References rooted in a *known* project group whose leaf is absent — a
    /// genuine miss (what T001 reports).
    pub references_unresolved: usize,

    /// Calls of the form `<LibraryObject>.<method>(…)` (Calculate, CanComms, …).
    pub intrinsic_calls_total: usize,
    /// …of those, calls whose method has no catalogue overload — the intrinsic
    /// object is known but the specific method is unmodelled.
    pub intrinsic_calls_unmodelled: usize,

    /// `when … is` statements encountered.
    pub when_subjects_total: usize,
    /// …of those, whose enum membership could not be fully verified: the
    /// subject typed `Unknown`, or resolved to an *open* firmware enum whose
    /// member set the catalogue does not fully know.
    pub when_subjects_incomplete: usize,

    /// Whether a `parameters.m1cfg` was loaded (T041 calibration coverage runs).
    pub cfg_loaded: bool,
    /// Whether any `.m1dbc` was loaded (T042/T108–T110 CAN checks run).
    pub dbc_loaded: bool,
}

impl CompletenessReport {
    /// Percentage of expression nodes the typer gave a known type, 0–100.
    /// `100.0` when there are no expressions (nothing unknown).
    pub fn typed_percent(&self) -> f64 {
        percent(self.expressions_typed, self.expressions_total)
    }

    /// Percentage of references that resolved (to a local/symbol/built-in),
    /// 0–100. `100.0` when there are no references.
    pub fn resolved_percent(&self) -> f64 {
        percent(self.references_resolved, self.references_total)
    }

    /// Scripts that were actually analysed (not skipped for syntax/depth).
    pub fn scripts_analysed(&self) -> usize {
        self.scripts_total
            .saturating_sub(self.scripts_with_syntax_errors)
            .saturating_sub(self.scripts_skipped_deep)
    }
}

/// Integer-count percentage with a total-zero guard, rounded to one decimal.
fn percent(part: usize, total: usize) -> f64 {
    if total == 0 {
        return 100.0;
    }
    ((part as f64 / total as f64) * 1000.0).round() / 10.0
}

/// Build the completeness report for a run: replay typing/resolution over every
/// parsed script and tally the coverage. `cfg_loaded`/`dbc_loaded` are threaded
/// from the load step (the report cannot re-derive them — they depend on what
/// was discovered on disk).
pub fn analyze(
    project: Option<&Project>,
    scripts: &[ParsedScript],
    cfg_loaded: bool,
    dbc_loaded: bool,
) -> CompletenessReport {
    let mut r = CompletenessReport {
        scripts_total: scripts.len(),
        cfg_loaded,
        dbc_loaded,
        ..Default::default()
    };

    for script in scripts {
        let cst = &script.cst;
        if !cst.syntax_diagnostics().is_empty() {
            r.scripts_with_syntax_errors += 1;
            continue;
        }
        // Mirror run_with's depth guard: an over-deep script is skipped by the
        // real analysis, so it contributes no coverage here either.
        if cst.root().max_depth() > m1_core::MAX_RECURSION_DEPTH {
            r.scripts_skipped_deep += 1;
            continue;
        }
        let group = match project {
            Some(p) => p.group_for_script(&script.name),
            None => None,
        };
        let fn_symbol = match project {
            Some(p) => p.function_symbol_for_script(&script.name),
            None => None,
        };
        let scope = Scope {
            locals: crate::rules::collect_locals(cst.root(), project, group.as_deref()),
            group,
            project,
            fn_symbol,
        };
        tally_script(cst.root(), &scope, &mut r);
    }

    r
}

/// Walk one script's CST, updating the running tallies. Three concerns share
/// the single traversal: expression typing, reference resolution, and the
/// intrinsic-call / `when`-subject counts.
fn tally_script(root: Node, scope: &Scope, r: &mut CompletenessReport) {
    for n in root.descendants() {
        let k = n.kind();

        // Expression typing coverage: every expression node, including nested
        // sub-expressions (a coverage estimate over expression *nodes*).
        if is_expr(k) {
            r.expressions_total += 1;
            if type_of(n, scope).is_known() {
                r.expressions_typed += 1;
            }
        }

        // Reference resolution: only *maximal* identifier/member paths — a node
        // whose parent is itself a member expression is an inner segment of a
        // longer path already counted, so skip it (resolve the whole path once).
        if matches!(k, Kind::Identifier | Kind::MemberExpression)
            && n.parent().map(|p| p.kind()) != Some(Kind::MemberExpression)
        {
            r.references_total += 1;
            match resolve(&path_text(n), scope) {
                Resolution::Local(_)
                | Resolution::Symbol(_)
                | Resolution::BuiltinObject(_)
                | Resolution::BuiltinFn(_) => r.references_resolved += 1,
                Resolution::Opaque => r.references_opaque += 1,
                Resolution::Unresolved => r.references_unresolved += 1,
            }
        }

        // Intrinsic-method calls: `<LibraryObject>.<method>(…)`. The object is
        // catalogued; count whether the specific method is too.
        if k == Kind::CallExpression {
            tally_call(n, scope, r);
        }

        // `when … is` enum membership: can we verify the subject's enum?
        if k == Kind::WhenStatement {
            tally_when(n, scope, r);
        }
    }
}

/// Count an intrinsic library call (`Calculate.Max(…)`, `CanComms.GetID(…)`).
/// Only calls whose callee root is a known library object are intrinsic calls;
/// of those, a callee that does not resolve to a `BuiltinFn` has no catalogue
/// overload for that method (the object is modelled, the method is not).
fn tally_call(call: Node, scope: &Scope, r: &mut CompletenessReport) {
    let Some(callee) = call
        .named_children()
        .into_iter()
        .find(|c| matches!(c.kind(), Kind::Identifier | Kind::MemberExpression))
    else {
        return;
    };
    let path = path_text(callee);
    let Some((root, _method)) = path.split_once('.') else {
        return; // a bare call is a user function, not an intrinsic-object method
    };
    if intrinsics::get().library_object_name(root).is_none() {
        return; // not a library object — a project method or accessor, out of scope
    }
    r.intrinsic_calls_total += 1;
    if !matches!(resolve(&path, scope), Resolution::BuiltinFn(_)) {
        r.intrinsic_calls_unmodelled += 1;
    }
}

/// Count a `when … is` subject and whether its enum membership is fully known.
fn tally_when(when: Node, scope: &Scope, r: &mut CompletenessReport) {
    let Some(subject) = when.child_by_field(Field::Subject) else {
        return;
    };
    r.when_subjects_total += 1;
    let incomplete = match type_of(subject, scope) {
        ValueType::Enum(id) => scope
            .project
            .map(|p| p.symbols().enum_is_open(id))
            .unwrap_or(true),
        // A known non-enum subject is a *definite* error (T082), not an
        // incompleteness; only an Unknown subject is a coverage gap.
        t => !t.is_known(),
    };
    if incomplete {
        r.when_subjects_incomplete += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsed::parse_all;

    fn parse_one(src: &str) -> Vec<ParsedScript> {
        parse_all(&[("S.m1scr".to_string(), src.to_string())])
    }

    #[test]
    fn percent_guards_zero_total() {
        assert_eq!(percent(0, 0), 100.0);
        assert_eq!(percent(1, 2), 50.0);
        assert_eq!(percent(1, 3), 33.3);
    }

    #[test]
    fn all_literals_are_fully_typed() {
        // `local x = 1 + 2;` — every expression node (1, 2, 1+2) is a known
        // integer, so typed coverage is 100%.
        let scripts = parse_one("local x = 1 + 2;\n");
        let r = analyze(None, &scripts, false, false);
        assert_eq!(r.scripts_total, 1);
        assert_eq!(r.scripts_analysed(), 1);
        assert!(r.expressions_total >= 3);
        assert_eq!(r.expressions_total, r.expressions_typed);
        assert_eq!(r.typed_percent(), 100.0);
    }

    #[test]
    fn project_less_channel_read_is_opaque_and_untyped() {
        // Project-less: `Engine.Speed` cannot resolve to a symbol, so it is an
        // opaque reference and stays Unknown-typed — the silent surface the
        // report exists to surface.
        let scripts = parse_one("local x = Engine.Speed + 1;\n");
        let r = analyze(None, &scripts, false, false);
        assert!(r.references_total >= 1);
        assert!(
            r.references_opaque >= 1,
            "the channel read is opaque: {r:?}"
        );
        assert_eq!(r.references_unresolved, 0);
        // The Unknown channel read means not every expression is typed.
        assert!(r.expressions_typed < r.expressions_total);
    }

    #[test]
    fn intrinsic_call_counts_modelled_vs_unmodelled() {
        // Calculate.Max is catalogued; Calculate.NoSuchMethod is not. Both are
        // intrinsic-object calls, but only the second is unmodelled.
        let scripts =
            parse_one("local a = Calculate.Max(1, 2);\nlocal b = Calculate.NoSuchMethod(1);\n");
        let r = analyze(None, &scripts, false, false);
        assert_eq!(r.intrinsic_calls_total, 2);
        assert_eq!(r.intrinsic_calls_unmodelled, 1);
    }

    #[test]
    fn syntax_error_script_is_skipped() {
        let scripts = parse_one("local x = ;\n");
        let r = analyze(None, &scripts, false, false);
        assert_eq!(r.scripts_total, 1);
        assert_eq!(r.scripts_with_syntax_errors, 1);
        assert_eq!(r.scripts_analysed(), 0);
        assert_eq!(r.expressions_total, 0);
    }

    #[test]
    fn cfg_and_dbc_flags_are_threaded() {
        let scripts = parse_one("local x = 1;\n");
        let r = analyze(None, &scripts, true, true);
        assert!(r.cfg_loaded && r.dbc_loaded);
    }

    #[test]
    fn when_subject_unknown_enum_is_incomplete() {
        // Project-less: the `when` subject is an opaque channel read, so its
        // enum membership is unknown — an incomplete when-subject.
        let src = "when (Engine.Mode) {\nis (Idle) {\n}\n}\n";
        let scripts = parse_one(src);
        let r = analyze(None, &scripts, false, false);
        assert_eq!(r.when_subjects_total, 1);
        assert_eq!(r.when_subjects_incomplete, 1);
    }
}
