//! Invalid-value (NaN/Inf) provenance tracing — T080 (#78).
//!
//! M1 floats are IEEE 754-2008 binary32, and M1 has no "no-data" sentinel: an
//! invalid result is a genuine NaN/±Inf that either propagates as-is or, when a
//! float is scaled to an integer for logging/CAN, surfaces as an extreme
//! "legitimate" value. The type lattice can't see this (NaN is a runtime
//! bit-pattern, not a static type), so we track a *can-be-invalid* bit alongside
//! the type via a dedicated backward taint analysis.
//!
//! The analysis is **annotation-scoped**: it only reports when a value is marked
//! `@m1:requires-finite` or `@m1:safety-critical` (a *sink* that must stay
//! finite), so it is zero-noise unless a team opts a channel in. From each sink
//! it walks the contributing expression/locals and flags an unsanitised path
//! from an invalid-value source.
//!
//! ## Model (deliberately conservative — see #78)
//!
//! - **Sources** (`can-be-invalid`): division (denominator may be 0),
//!   domain-restricted math (`Calculate.FastSquareRoot`, `Math.Log`,
//!   `Math.Log10`, `Calculate.InverseSin`, `Calculate.InverseCos`), explicit
//!   `Calculate.Infinity()`, and reads of a channel/local declared
//!   `@m1:source`/`@m1:external` (a sensor that can drop out).
//! - **Sanitiser** that clears the bit: an assignment annotated
//!   `@m1:sanitizes`/`@m1:clears` (an author-declared taint barrier). Note
//!   `Math.IsNaN()` is **calibration-only** per the M1 Development Manual (its
//!   only manual example is inside a Tune calibration method alongside
//!   `UI.PromptOK`), so it cannot appear in an ECU `.m1scr` — T063 already flags
//!   it — and is therefore *not* the in-script guard. **Clamps do _not_
//!   sanitise** either — NaN compares false against everything, so
//!   `Limit.Range`/`Calculate.Min`/`Calculate.Max` pass it straight through.
//!   This is the dangerous "false safety" the issue calls out.
//! - **Latching escalation:** a tainted value reaching a stateful function
//!   (`Filter.*`, `Integral.*`, `Derivative.*`) is escalated to an error, because
//!   the filter state stays poisoned after the input recovers.
//!
//! This pass is **intra-script** (P2/P4 of #78). Cross-script `Out`→`In`
//! propagation (the full SBG→torque-vectoring shape, P3) is a follow-up that
//! reuses this taint lattice over the project channel-writer index.

use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::typer::path_text;
use m1_core::{Annotations, Kind, Node, Severity};
use std::collections::{HashMap, HashSet};

/// The invalid-value state of an expression's result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Taint {
    /// The value can be NaN/Inf.
    invalid: bool,
    /// An explicit finite guard (`Math.IsNaN` fallback / `@sanitizes`) cleared it.
    sanitised: bool,
    /// It flowed through a stateful function that latches NaN — escalate.
    latched: bool,
    /// A short provenance string for the diagnostic message.
    why: Option<String>,
}

impl Taint {
    fn finite() -> Self {
        Taint::default()
    }

    fn source(why: impl Into<String>) -> Self {
        Taint {
            invalid: true,
            sanitised: false,
            latched: false,
            why: Some(why.into()),
        }
    }

    fn cleared() -> Self {
        Taint {
            invalid: false,
            sanitised: true,
            latched: false,
            why: None,
        }
    }

    /// Combine two operand taints (the result is invalid if either is).
    fn join(self, other: Taint) -> Taint {
        Taint {
            invalid: self.invalid || other.invalid,
            sanitised: false,
            latched: self.latched || other.latched,
            why: self.why.or(other.why),
        }
    }
}

/// Domain-restricted math whose result is NaN on out-of-domain input.
const DOMAIN_FNS: &[&str] = &[
    "Calculate.FastSquareRoot",
    "Calculate.SquareRoot",
    "Math.Log",
    "Math.Log10",
    "Calculate.InverseSin",
    "Calculate.InverseCos",
];

/// Stateful library functions that latch NaN in their internal accumulator.
const STATEFUL_PREFIXES: &[&str] = &["Filter.", "Integral.", "Derivative."];

/// Run the invalid-value analysis, appending T080 diagnostics.
///
/// `_scope` is unused intra-script but kept on the signature: the cross-script
/// pass (P3) needs it to resolve `Out`→`In` channel references project-wide.
pub fn check(root: Node, _scope: &Scope, anns: &Annotations, out: &mut Vec<TypeDiagnostic>) {
    // Cheap exit: nothing to do unless a finiteness sink is present.
    let has_sink = anns
        .all()
        .iter()
        .any(|a| a.kind == "requires-finite" || a.kind == "safety-critical");
    if !has_sink {
        return;
    }
    let sources = collect_sources(root, anns);
    let mut env: HashMap<String, Taint> = HashMap::new();
    process(root, anns, &sources, &mut env, out);
}

/// Names (`path_text`) declared an invalid-value source via `@source`/`@external`.
fn collect_sources(root: Node, anns: &Annotations) -> HashSet<String> {
    let mut sources = HashSet::new();
    visit_statements(root, &mut |stmt| {
        let here = anns.for_target_start(stmt.byte_range().start);
        if here
            .iter()
            .any(|a| a.kind == "source" || a.kind == "external")
            && let Some(name) = defined_name(stmt)
        {
            sources.insert(name);
        }
    });
    sources
}

/// Walk statements in document order, threading the taint environment.
fn process(
    node: Node,
    anns: &Annotations,
    sources: &HashSet<String>,
    env: &mut HashMap<String, Taint>,
    out: &mut Vec<TypeDiagnostic>,
) {
    for stmt in node.children() {
        match stmt.kind() {
            Kind::AssignmentStatement | Kind::LocalDeclaration => {
                let here = anns.for_target_start(stmt.byte_range().start);
                let is_sanitiser = here
                    .iter()
                    .any(|a| a.kind == "sanitizes" || a.kind == "clears");
                let is_source = here
                    .iter()
                    .any(|a| a.kind == "source" || a.kind == "external");

                let name = defined_name(stmt);
                let value = value_expr(stmt);

                let taint = if is_sanitiser {
                    Taint::cleared()
                } else if is_source {
                    Taint::source(format!(
                        "`{}` (declared a volatile invalid-value source)",
                        name.as_deref().unwrap_or("source")
                    ))
                } else {
                    value
                        .map(|e| expr_taint(e, env, sources))
                        .unwrap_or_default()
                };

                // Sink check: does an unsanitised invalid value reach a value
                // that must stay finite?
                if let Some(base) = sink_severity(&here)
                    && taint.invalid
                    && !taint.sanitised
                {
                    let severity = if taint.latched {
                        Severity::Error // latched NaN — highest severity
                    } else {
                        base
                    };
                    let at = value.unwrap_or(stmt);
                    out.push(make(TypeCode::T080, &at, severity, message(&taint)));
                }

                if let Some(name) = name {
                    env.insert(name, taint);
                }
            }
            _ => process(stmt, anns, sources, env, out),
        }
    }
}

/// The severity floor for a sink: error for `@safety-critical`, warning for
/// `@requires-finite`, `None` if neither is present.
fn sink_severity(here: &[&m1_core::Annotation]) -> Option<Severity> {
    if here.iter().any(|a| a.kind == "safety-critical") {
        Some(Severity::Error)
    } else if here.iter().any(|a| a.kind == "requires-finite") {
        Some(Severity::Warning)
    } else {
        None
    }
}

fn message(t: &Taint) -> String {
    let mut m = String::from("a value required to be finite can be NaN/Inf");
    if let Some(why) = &t.why {
        m.push_str(&format!(": {why}"));
    }
    if t.latched {
        m.push_str(
            "; it reaches a stateful function (Filter/Integral/Derivative) that latches NaN",
        );
    }
    m.push_str(" — guard it with Math.IsNaN() or a finite fallback");
    m
}

/// Taint of an expression's result.
fn expr_taint(node: Node, env: &HashMap<String, Taint>, sources: &HashSet<String>) -> Taint {
    match node.kind() {
        Kind::Number | Kind::Boolean | Kind::String => Taint::finite(),
        Kind::Identifier | Kind::MemberExpression => {
            let p = path_text(node);
            if sources.contains(&p) {
                Taint::source(format!("`{p}` (declared a volatile invalid-value source)"))
            } else {
                env.get(&p).cloned().unwrap_or_default()
            }
        }
        Kind::ParenthesizedExpression | Kind::UnaryExpression => node
            .named_children()
            .into_iter()
            .find(|c| is_expr(c.kind()))
            .map(|c| expr_taint(c, env, sources))
            .unwrap_or_default(),
        Kind::BinaryExpression => binary_taint(node, env, sources),
        Kind::TernaryExpression => ternary_taint(node, env, sources),
        Kind::CallExpression => call_taint(node, env, sources),
        _ => Taint::finite(),
    }
}

fn binary_taint(node: Node, env: &HashMap<String, Taint>, sources: &HashSet<String>) -> Taint {
    let operands: Vec<Node> = node
        .named_children()
        .into_iter()
        .filter(|c| is_expr(c.kind()))
        .collect();
    let op = node
        .children()
        .into_iter()
        .find(|c| is_operator(c.kind()))
        .map(|c| c.text().to_string())
        .unwrap_or_default();

    // Comparison / logical operators yield a boolean — always finite.
    if is_boolean_operator(&op) {
        return Taint::finite();
    }
    let mut t = operands
        .into_iter()
        .map(|c| expr_taint(c, env, sources))
        .fold(Taint::finite(), Taint::join);
    if op == "/" || op == "%" {
        t.invalid = true;
        t.why
            .get_or_insert_with(|| "division (the denominator may be 0)".to_string());
    }
    t
}

fn ternary_taint(node: Node, env: &HashMap<String, Taint>, sources: &HashSet<String>) -> Taint {
    // `condition ? consequence : alternative`. The condition is a boolean; the
    // result is whichever branch is selected, so its taint is the join of the
    // two branches. We deliberately do NOT treat any condition as a sanitiser:
    // the manual's NaN predicate `Math.IsNaN` is calibration-only and cannot
    // appear in an ECU script, and a plain finite-range comparison does not
    // catch NaN (NaN fails every comparison). The in-script taint barrier is an
    // explicit `@m1:sanitizes`/`@m1:clears` annotation.
    node.named_children()
        .into_iter()
        .filter(|c| is_expr(c.kind()))
        .skip(1) // drop the condition; keep the two branches
        .map(|c| expr_taint(c, env, sources))
        .fold(Taint::finite(), Taint::join)
}

fn call_taint(node: Node, env: &HashMap<String, Taint>, sources: &HashSet<String>) -> Taint {
    let Some(callee) = callee_path(node) else {
        return Taint::finite();
    };
    // Explicit infinity.
    if callee == "Calculate.Infinity" {
        return Taint::source("`Calculate.Infinity()` (explicit +Inf)");
    }
    let args: Vec<Taint> = call_args(node)
        .into_iter()
        .map(|a| expr_taint(a, env, sources))
        .collect();
    let args_taint = args.into_iter().fold(Taint::finite(), Taint::join);

    // Domain-restricted math: NaN on out-of-domain input, regardless of args.
    if DOMAIN_FNS.contains(&callee.as_str()) {
        let mut t = Taint::source(format!("`{callee}` can return NaN for out-of-domain input"));
        t.latched = args_taint.latched;
        return t;
    }
    // Stateful function: propagate, and escalate (latch) if the input is tainted.
    if STATEFUL_PREFIXES.iter().any(|p| callee.starts_with(p)) {
        let mut t = args_taint;
        if t.invalid {
            t.latched = true;
        }
        return t;
    }
    // Everything else (including clamps like Limit.Range / Calculate.Min/Max)
    // simply propagates argument taint WITHOUT clearing it.
    args_taint
}

/// The dotted callee path of a call expression (`Filter.FirstOrder`).
fn callee_path(call: Node) -> Option<String> {
    call.named_children()
        .into_iter()
        .find(|c| matches!(c.kind(), Kind::Identifier | Kind::MemberExpression))
        .map(path_text)
}

/// The positional argument expressions of a call.
fn call_args(call: Node) -> Vec<Node> {
    call.children()
        .into_iter()
        .find(|c| c.kind() == Kind::ArgumentList)
        .map(|al| {
            al.named_children()
                .into_iter()
                .filter(|c| is_expr(c.kind()))
                .collect()
        })
        .unwrap_or_default()
}

/// The name a statement defines: the assignment target, or the local's name.
fn defined_name(stmt: Node) -> Option<String> {
    match stmt.kind() {
        Kind::AssignmentStatement => stmt
            .named_children()
            .into_iter()
            .find(|c| is_expr(c.kind()))
            .map(path_text),
        Kind::LocalDeclaration => stmt
            .named_children()
            .into_iter()
            .find(|c| c.kind() == Kind::Identifier)
            .map(path_text),
        _ => None,
    }
}

/// The right-hand-side value expression of an assignment or local declaration.
fn value_expr(stmt: Node) -> Option<Node> {
    match stmt.kind() {
        Kind::AssignmentStatement => {
            // The value is the named expression that is not the target (the
            // target is the first; the value is the last expression child).
            let exprs: Vec<Node> = stmt
                .named_children()
                .into_iter()
                .filter(|c| is_expr(c.kind()))
                .collect();
            (exprs.len() >= 2).then(|| exprs[exprs.len() - 1])
        }
        Kind::LocalDeclaration => stmt.named_children().into_iter().find(|c| {
            is_expr(c.kind()) && c.kind() != Kind::Identifier && c.kind() != Kind::TypeAnnotation
        }),
        _ => None,
    }
}

/// Depth-first visit of every statement-ish node.
fn visit_statements(node: Node, f: &mut impl FnMut(Node)) {
    for child in node.children() {
        if matches!(
            child.kind(),
            Kind::AssignmentStatement | Kind::LocalDeclaration
        ) {
            f(child);
        }
        visit_statements(child, f);
    }
}

fn is_operator(k: Kind) -> bool {
    matches!(
        k,
        Kind::Plus
            | Kind::Minus
            | Kind::Star
            | Kind::Slash
            | Kind::Percent
            | Kind::Lt
            | Kind::LtEq
            | Kind::Gt
            | Kind::GtEq
            | Kind::EqEq
            | Kind::BangEq
            | Kind::And
            | Kind::Or
            | Kind::Amp
            | Kind::Pipe
            | Kind::Caret
    )
}

fn is_boolean_operator(op: &str) -> bool {
    matches!(
        op,
        "<" | "<=" | ">" | ">=" | "==" | "!=" | "eq" | "neq" | "and" | "or"
    )
}

fn is_expr(k: Kind) -> bool {
    matches!(
        k,
        Kind::Identifier
            | Kind::MemberExpression
            | Kind::CallExpression
            | Kind::UnaryExpression
            | Kind::BinaryExpression
            | Kind::TernaryExpression
            | Kind::ParenthesizedExpression
            | Kind::Number
            | Kind::Boolean
            | Kind::String
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::Scope;
    use std::collections::HashMap;

    fn run(src: &str) -> Vec<TypeDiagnostic> {
        let cst = m1_core::parse(src);
        let anns = m1_core::annotations(&cst, &m1_core::Registry::seed());
        let scope = Scope {
            locals: HashMap::new(),
            group: None,
            project: None,
        };
        let mut out = Vec::new();
        check(cst.root(), &scope, &anns, &mut out);
        out
    }

    fn has_t080(diags: &[TypeDiagnostic]) -> bool {
        diags.iter().any(|d| d.code == TypeCode::T080)
    }

    #[test]
    fn no_sink_means_no_analysis() {
        // A division with no annotation anywhere: silent.
        assert!(run("local x = a / b;\nOut = x;\n").is_empty());
    }

    #[test]
    fn division_reaching_requires_finite_is_flagged() {
        let src = "// @m1:requires-finite\nOut = a / b;\n";
        assert!(has_t080(&run(src)));
    }

    #[test]
    fn domain_math_reaching_sink_is_flagged() {
        let src = "// @m1:requires-finite\nOut = Calculate.FastSquareRoot(x);\n";
        assert!(has_t080(&run(src)));
    }

    #[test]
    fn explicit_infinity_is_flagged() {
        let src = "// @m1:requires-finite\nOut = Calculate.Infinity();\n";
        assert!(has_t080(&run(src)));
    }

    #[test]
    fn finite_value_is_not_flagged() {
        let src = "// @m1:requires-finite\nOut = a + b * 2;\n";
        assert!(!has_t080(&run(src)));
    }

    #[test]
    fn ternary_does_not_sanitise_by_itself() {
        // A bare ternary is not a sanitiser: its condition cannot be a valid
        // in-script NaN guard (Math.IsNaN is calibration-only), so a tainted
        // branch still flags. The in-script barrier is `@m1:sanitizes`.
        let src = "// @m1:requires-finite\nOut = cond ? 0 : a / b;\n";
        assert!(has_t080(&run(src)));
    }

    #[test]
    fn clamp_does_not_sanitise() {
        // The dangerous false-safety case: a range clamp does NOT clear NaN.
        let src = "// @m1:requires-finite\nOut = Limit.Range(a / b, -1, 1);\n";
        assert!(has_t080(&run(src)));
    }

    #[test]
    fn min_does_not_sanitise() {
        let src = "// @m1:requires-finite\nOut = Calculate.Min(a / b, 1);\n";
        assert!(has_t080(&run(src)));
    }

    #[test]
    fn taint_propagates_through_locals() {
        let src = "local t = a / b;\nlocal u = t + 1;\n// @m1:requires-finite\nOut = u;\n";
        assert!(has_t080(&run(src)));
    }

    #[test]
    fn source_annotation_marks_a_volatile_input() {
        let src = "// @m1:source(volatile)\nGyro = Read.Raw();\n// @m1:requires-finite\nOut = Gyro + 1;\n";
        assert!(has_t080(&run(src)));
    }

    #[test]
    fn sanitizes_annotation_clears_the_bit() {
        let src = "// @m1:sanitizes\nClean = a / b;\n// @m1:requires-finite\nOut = Clean;\n";
        assert!(!has_t080(&run(src)));
    }

    #[test]
    fn safety_critical_is_an_error_and_latches_through_a_filter() {
        let src = "// @m1:safety-critical\nOut = Filter.FirstOrder(a / b, 0.1);\n";
        let diags = run(src);
        assert!(has_t080(&diags));
        let d = diags.iter().find(|d| d.code == TypeCode::T080).unwrap();
        assert_eq!(d.inner.severity, Severity::Error);
        assert!(d.inner.message.contains("latches"));
    }

    #[test]
    fn requires_finite_escalates_to_error_when_latched() {
        // Even a (lower-severity) @requires-finite sink escalates to Error when
        // the tainted value reaches a latching stateful function.
        let src = "// @m1:requires-finite\nOut = Integral.Normal(a / b);\n";
        let d = run(src);
        let d = d.iter().find(|d| d.code == TypeCode::T080).unwrap();
        assert_eq!(d.inner.severity, Severity::Error);
    }
}
