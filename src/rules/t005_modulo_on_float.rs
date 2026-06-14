//! T005 — modulo (`%`) applied to a floating-point operand.
//!
//! The M1 Development Manual lists Modulo (`a % b`) only under **Integer
//! Arithmetic** (p.35) and deliberately omits it from the **Floating Point
//! Arithmetic** table on the same page (which has only Assignment, Add,
//! Subtract, Negate, Multiply, Divide). In the M1 language the `%` *operator*
//! is integer-only; applying it to a floating-point operand is not valid.
//!
//! This mirrors the other operator-domain rules (T002 float-equality, T003
//! float→int narrowing, T004 signed/unsigned ordering): a focused diagnostic
//! for an illegal-operand case the generic expression typer would otherwise
//! type silently (it lumps `%` in with `+ - * /` as arithmetic).
//!
//! Floating-point remainder *is* available in the language — just not via the
//! operator: the manual (p.47) lists `Calculate.Modulo` as a library function
//! that accepts and returns "Integer or Floating Point". So the message points
//! the user there rather than claiming M1 lacks float modulo entirely.
//!
//! The guard is `is_float`-only (silent on Integer/Unsigned/Unknown), so the
//! corpus's integer-counter modulo (`i % N`) stays clean — `Unknown` is
//! absorbing here as everywhere.
//!
//! Both forms are covered: the bare operator (`a % b`, a `BinaryExpression`)
//! and the compound assignment (`a %= b`, manual p.37 "Modulo assignment").
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::typer::{is_expr, type_of};
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

const MSG: &str = "the modulo operator `%` is integer-only (manual p.35 lists it under Integer \
                   Arithmetic, not Floating Point); use Calculate.Modulo for floating-point \
                   remainder";

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        match node.kind() {
            Kind::BinaryExpression => self.check_binary(node, scope, out),
            Kind::AssignmentStatement => self.check_compound_assign(node, scope, out),
            _ => {}
        }
    }
}

impl Rule {
    /// A bare `a % b` binary expression with a float operand.
    fn check_binary(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let is_modulo = node.children().iter().any(|c| c.kind() == Kind::Percent);
        if !is_modulo {
            return;
        }
        let any_float = node
            .named_children()
            .into_iter()
            .filter(|c| is_expr(c.kind()))
            .any(|c| type_of(c, scope).is_float());
        if any_float {
            out.push(make(TypeCode::T005, node, Severity::Error, MSG.into()));
        }
    }

    /// The compound form `a %= b` (manual p.37) with a float operand on either
    /// side.
    fn check_compound_assign(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let Some(op) = node.child_by_field(Field::Operator) else {
            return;
        };
        if op.kind() != Kind::PercentEq {
            return;
        }
        let float_operand = [
            node.child_by_field(Field::Target),
            node.child_by_field(Field::Value),
        ]
        .into_iter()
        .flatten()
        .any(|c| type_of(c, scope).is_float());
        if float_operand {
            out.push(make(TypeCode::T005, node, Severity::Error, MSG.into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::Scope;
    use crate::rules::Rule as _;
    use crate::types::ValueType;

    fn diags(src: &str, locals: &[(&str, ValueType)]) -> Vec<TypeDiagnostic> {
        let cst = m1_core::parse(src);
        let scope = Scope {
            locals: locals.iter().map(|(n, t)| (n.to_string(), *t)).collect(),
            group: None,
            project: None,
            fn_symbol: None,
        };
        let mut out = Vec::new();
        walk(cst.root(), &scope, &mut out);
        out
    }

    fn walk(node: Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        Rule.check_node(&node, scope, out);
        for c in node.children() {
            walk(c, scope, out);
        }
    }

    #[test]
    fn flags_modulo_on_float_local() {
        let out = diags(
            "local x = f % g;\n",
            &[("f", ValueType::Float), ("g", ValueType::Float)],
        );
        assert_eq!(out.len(), 1, "expected one T005, got {out:?}");
        assert_eq!(out[0].code, TypeCode::T005);
        // Float modulo is invalid per the manual (p.35), not a style nit: it is
        // an Error so it blocks m1-ci (which doesn't fail on warnings by default)
        // and shows as an error in editors.
        assert_eq!(out[0].inner.severity, Severity::Error);
    }

    #[test]
    fn flags_modulo_float_against_integer_literal() {
        // `floatA % 2` — only one operand is float, still illegal for `%`.
        let out = diags("local x = f % 2;\n", &[("f", ValueType::Float)]);
        assert_eq!(out.len(), 1, "expected one T005, got {out:?}");
        assert_eq!(out[0].code, TypeCode::T005);
    }

    #[test]
    fn flags_modulo_compound_assign_on_float() {
        // `f %= 2;` — compound modulo-assignment with a float target (manual p.37).
        let out = diags("f %= 2;\n", &[("f", ValueType::Float)]);
        assert_eq!(out.len(), 1, "expected one T005, got {out:?}");
        assert_eq!(out[0].code, TypeCode::T005);
    }

    #[test]
    fn flags_modulo_compound_assign_with_float_value() {
        // Integer target, float value via `%=` — still a float operand on `%=`.
        let out = diags(
            "i %= f;\n",
            &[("i", ValueType::Integer), ("f", ValueType::Float)],
        );
        assert_eq!(out.len(), 1, "expected one T005, got {out:?}");
        assert_eq!(out[0].code, TypeCode::T005);
    }

    #[test]
    fn integer_modulo_is_clean() {
        // Integer-counter modulo `i % N` is the legitimate corpus shape — silent.
        let out = diags("local x = i % 8;\n", &[("i", ValueType::Integer)]);
        assert!(out.is_empty(), "integer modulo must be clean: {out:?}");
    }

    #[test]
    fn unsigned_and_unknown_modulo_are_clean() {
        // Unsigned is integral; Unknown is absorbing — neither fires.
        let unsigned = diags("local x = u % 8;\n", &[("u", ValueType::Unsigned)]);
        assert!(
            unsigned.is_empty(),
            "unsigned modulo must be clean: {unsigned:?}"
        );
        let unknown = diags("local x = z % 8;\n", &[]);
        assert!(
            unknown.is_empty(),
            "unknown modulo must be silent: {unknown:?}"
        );
    }

    #[test]
    fn other_arithmetic_on_float_is_not_t005() {
        // `+ - * /` on floats are valid Floating Point Arithmetic — not our concern.
        for op in ["+", "-", "*", "/"] {
            let src = format!("local x = f {op} g;\n");
            let out = diags(&src, &[("f", ValueType::Float), ("g", ValueType::Float)]);
            assert!(
                out.is_empty(),
                "`{op}` on floats must not fire T005: {out:?}"
            );
        }
    }

    #[test]
    fn integer_compound_modulo_assign_is_clean() {
        let out = diags("i %= 2;\n", &[("i", ValueType::Integer)]);
        assert!(out.is_empty(), "integer `%=` must be clean: {out:?}");
    }
}
