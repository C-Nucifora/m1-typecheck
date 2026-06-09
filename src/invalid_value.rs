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

use crate::cross_script::ChannelTaints;
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::typer::{is_expr, path_text};
use crate::types::ValueType;
use m1_core::{Annotations, Kind, Node, Severity};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The invalid-value state of an expression's result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Taint {
    /// The value can be NaN/Inf.
    pub(crate) invalid: bool,
    /// An explicit finite guard (`Math.IsNaN` fallback / `@sanitizes`) cleared it.
    pub(crate) sanitised: bool,
    /// It flowed through a stateful function that latches NaN — escalate.
    pub(crate) latched: bool,
    /// A short provenance string for the diagnostic message.
    pub(crate) why: Option<String>,
    /// Canonical project-symbol paths this value reads (the cross-script
    /// dependency edges, #78 P3), each flagged true when the read flows through
    /// a latching stateful function on its way to the result.
    pub(crate) deps: BTreeMap<String, bool>,
}

impl Taint {
    fn finite() -> Self {
        Taint::default()
    }

    pub(crate) fn source(why: impl Into<String>) -> Self {
        Taint {
            invalid: true,
            sanitised: false,
            latched: false,
            why: Some(why.into()),
            deps: BTreeMap::new(),
        }
    }

    fn cleared() -> Self {
        Taint {
            invalid: false,
            sanitised: true,
            latched: false,
            why: None,
            deps: BTreeMap::new(),
        }
    }

    /// Combine two operand taints (the result is invalid if either is).
    fn join(self, other: Taint) -> Taint {
        let mut deps = self.deps;
        for (path, latched) in other.deps {
            let e = deps.entry(path).or_insert(false);
            *e = *e || latched;
        }
        Taint {
            invalid: self.invalid || other.invalid,
            sanitised: false,
            latched: self.latched || other.latched,
            why: self.why.or(other.why),
            deps,
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

/// Everything the statement/expression walkers need, bundled so the recursion
/// threads one reference instead of five parameters.
struct Ctx<'a, 'p> {
    scope: &'a Scope<'p>,
    anns: &'a Annotations,
    /// Names declared `@source`/`@external` (spelled paths).
    sources: &'a HashSet<String>,
    /// Project-wide solved channel taints, seeded into project-symbol reads
    /// (empty for the pure intra-script run).
    channels: &'a ChannelTaints,
    /// Canonical path of the function symbol this script backs — the channel
    /// an `Out =` assignment writes (None when the script backs no function).
    fn_symbol: Option<&'a str>,
}

/// Run the intra-script invalid-value analysis, appending T080 diagnostics.
pub fn check(root: Node, scope: &Scope, anns: &Annotations, out: &mut Vec<TypeDiagnostic>) {
    let empty = ChannelTaints::default();
    if can_skip(root, anns, &empty) {
        return;
    }
    check_with_channels(root, scope, anns, &empty, None, out);
}

/// Cheap exit: a possibly-NaN value can only arise from a division (or a
/// domain-restricted call / `@source`), and only matters if it can reach a
/// sink — an annotated finiteness sink, or the implicit integer-conversion
/// sink (#120). With cross-script taints seeded the walk must always run: a
/// plain channel copy can carry a remote invalid value into an integer sink.
pub(crate) fn can_skip(root: Node, anns: &Annotations, channels: &ChannelTaints) -> bool {
    anns.all().is_empty() && !contains_invalid_source(root) && channels.is_empty()
}

/// Run the analysis with the project-wide solved `channels` taints seeded into
/// project-symbol reads (#78 P3). Returns the script's *write summary*: for
/// each assignment targeting a project symbol (canonical path; `Out` maps to
/// `fn_symbol`), the taint of the written value including its `deps` edges —
/// the input [`crate::cross_script::solve`] builds the channel graph from.
pub(crate) fn check_with_channels(
    root: Node,
    scope: &Scope,
    anns: &Annotations,
    channels: &ChannelTaints,
    fn_symbol: Option<&str>,
    out: &mut Vec<TypeDiagnostic>,
) -> Vec<(String, Taint)> {
    let sources = collect_sources(root, anns);
    let ctx = Ctx {
        scope,
        anns,
        sources: &sources,
        channels,
        fn_symbol,
    };
    let mut env: HashMap<String, Taint> = HashMap::new();
    let mut writes = Vec::new();
    process(root, &ctx, &mut env, &mut writes, out);
    writes
}

/// True if the tree contains a potential NaN/Inf source the analysis cares about:
/// a division/modulo, a domain-restricted math call, or `Calculate.Infinity()`.
/// Used to skip the walk entirely on the (common) scripts that can't produce one.
fn contains_invalid_source(root: Node) -> bool {
    root.descendants().any(|n| match n.kind() {
        Kind::BinaryExpression => n
            .child_nodes()
            .any(|c| matches!(c.kind(), Kind::Slash | Kind::Percent)),
        Kind::CallExpression => callee_path(n)
            .map(|c| c == "Calculate.Infinity" || DOMAIN_FNS.contains(&c.as_str()))
            .unwrap_or(false),
        _ => false,
    })
}

/// True if `path` resolves to an integer-typed channel/parameter — the target of
/// an implicit NaN→garbage-integer conversion when a non-finite float is stored.
fn is_integer_target(path: &str, scope: &Scope) -> bool {
    matches!(
        resolve(path, scope),
        Resolution::Symbol(s) if matches!(s.value_type, ValueType::Integer | ValueType::Unsigned)
    )
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

/// Walk statements in document order, threading the taint environment and
/// collecting the cross-script write summary.
fn process(
    node: Node,
    ctx: &Ctx,
    env: &mut HashMap<String, Taint>,
    writes: &mut Vec<(String, Taint)>,
    out: &mut Vec<TypeDiagnostic>,
) {
    for stmt in node.child_nodes() {
        match stmt.kind() {
            Kind::AssignmentStatement | Kind::LocalDeclaration => {
                let here = ctx.anns.for_target_start(stmt.byte_range().start);
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
                    value.map(|e| expr_taint(e, ctx, env)).unwrap_or_default()
                };

                // Sink check: does an unsanitised invalid value reach a value
                // that must stay finite?
                let annotated_sink = sink_severity(&here);
                if let Some(base) = annotated_sink
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

                // Implicit integer-conversion sink (#120): storing a possibly
                // NaN/Inf float into an integer-typed channel reinterprets the
                // float bit pattern as a garbage (often huge) integer. Flag it
                // even without an annotation — but not twice if T080 already fired.
                if annotated_sink.is_none()
                    && taint.invalid
                    && !taint.sanitised
                    && stmt.kind() == Kind::AssignmentStatement
                    && let Some(target) = &name
                    && is_integer_target(target, ctx.scope)
                {
                    let at = value.unwrap_or(stmt);
                    out.push(make(
                        TypeCode::T081,
                        &at,
                        Severity::Warning,
                        nan_int_message(&taint, target),
                    ));
                }

                // Cross-script channel graph (#78 P3): record the canonical
                // symbol this assignment writes (an `Out =` writes the
                // script's backing function symbol).
                if stmt.kind() == Kind::AssignmentStatement
                    && let Some(target) = &name
                {
                    let canonical = if target == "Out" {
                        ctx.fn_symbol.map(str::to_string)
                    } else if let Resolution::Symbol(s) = resolve(target, ctx.scope) {
                        Some(s.path.clone())
                    } else {
                        None
                    };
                    if let Some(c) = canonical {
                        writes.push((c, taint.clone()));
                    }
                }

                if let Some(name) = name {
                    env.insert(name, taint);
                }
            }
            _ => process(stmt, ctx, env, writes, out),
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

fn nan_int_message(t: &Taint, target: &str) -> String {
    let mut m = format!(
        "a possibly NaN/Inf value is stored to integer channel `{target}` — a non-finite float converts to a garbage (often huge) integer"
    );
    if let Some(why) = &t.why {
        m.push_str(&format!("; the value can be invalid via {why}"));
    }
    m.push_str(" — guard the divisor or clamp to a finite range before storing");
    m
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
fn expr_taint(node: Node, ctx: &Ctx, env: &HashMap<String, Taint>) -> Taint {
    match node.kind() {
        Kind::Number | Kind::Boolean | Kind::String => Taint::finite(),
        Kind::Identifier | Kind::MemberExpression => {
            let p = path_text(node);
            if ctx.sources.contains(&p) {
                return Taint::source(format!("`{p}` (declared a volatile invalid-value source)"));
            }
            if let Some(t) = env.get(&p) {
                return t.clone();
            }
            // A project-symbol read is a cross-script edge: seed from the
            // solved channel taints and record the dependency for the solver.
            if let Resolution::Symbol(s) = resolve(&p, ctx.scope) {
                let mut t = ctx.channels.read_taint(&s.path);
                t.deps.insert(s.path.clone(), false);
                return t;
            }
            Taint::default()
        }
        Kind::ParenthesizedExpression | Kind::UnaryExpression => node
            .named_children()
            .into_iter()
            .find(|c| is_expr(c.kind()))
            .map(|c| expr_taint(c, ctx, env))
            .unwrap_or_default(),
        Kind::BinaryExpression => binary_taint(node, ctx, env),
        Kind::TernaryExpression => ternary_taint(node, ctx, env),
        Kind::CallExpression => call_taint(node, ctx, env),
        _ => Taint::finite(),
    }
}

fn binary_taint(node: Node, ctx: &Ctx, env: &HashMap<String, Taint>) -> Taint {
    let operands: Vec<Node> = node
        .named_children()
        .into_iter()
        .filter(|c| is_expr(c.kind()))
        .collect();
    let op = node
        .child_nodes()
        .find(|c| is_operator(c.kind()))
        .map(|c| c.text().to_string())
        .unwrap_or_default();

    // Comparison / logical operators yield a boolean — always finite.
    if is_boolean_operator(&op) {
        return Taint::finite();
    }
    let mut t = operands
        .into_iter()
        .map(|c| expr_taint(c, ctx, env))
        .fold(Taint::finite(), Taint::join);
    if op == "/" || op == "%" {
        t.invalid = true;
        t.why
            .get_or_insert_with(|| "division (the denominator may be 0)".to_string());
    }
    t
}

fn ternary_taint(node: Node, ctx: &Ctx, env: &HashMap<String, Taint>) -> Taint {
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
        .map(|c| expr_taint(c, ctx, env))
        .fold(Taint::finite(), Taint::join)
}

fn call_taint(node: Node, ctx: &Ctx, env: &HashMap<String, Taint>) -> Taint {
    let Some(callee) = callee_path(node) else {
        return Taint::finite();
    };
    // Explicit infinity.
    if callee == "Calculate.Infinity" {
        return Taint::source("`Calculate.Infinity()` (explicit +Inf)");
    }
    let args: Vec<Taint> = call_args(node)
        .into_iter()
        .map(|a| expr_taint(a, ctx, env))
        .collect();
    let args_taint = args.into_iter().fold(Taint::finite(), Taint::join);

    // Domain-restricted math: NaN on out-of-domain input, regardless of args.
    // The argument edges stay on the result for the solver's bookkeeping.
    if DOMAIN_FNS.contains(&callee.as_str()) {
        let mut t = Taint::source(format!("`{callee}` can return NaN for out-of-domain input"));
        t.latched = args_taint.latched;
        t.deps = args_taint.deps;
        return t;
    }
    // Stateful function: propagate, and escalate (latch) if the input is
    // tainted. Any upstream channel feeding this call is latched *here* even
    // when nothing is invalid yet — the solver may taint it later (#78 P3).
    if STATEFUL_PREFIXES.iter().any(|p| callee.starts_with(p)) {
        let mut t = args_taint;
        if t.invalid {
            t.latched = true;
        }
        for latched in t.deps.values_mut() {
            *latched = true;
        }
        return t;
    }
    // A user-function/method call returns the callee's `Out` value — a
    // cross-script edge keyed by the callee's symbol path, like a channel read.
    if let Resolution::Symbol(s) = resolve(&callee, ctx.scope) {
        let mut t = args_taint.join(ctx.channels.read_taint(&s.path));
        t.deps.insert(s.path.clone(), false);
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
    call.child_nodes()
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
    for child in node.child_nodes() {
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
