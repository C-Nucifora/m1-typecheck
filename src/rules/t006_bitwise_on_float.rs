//! T006 — a bitwise operator (`& | ^ ~ << >>`) applied to a floating-point
//! operand.
//!
//! The M1 Development Manual lists these operators under a table titled
//! **Integer Bitwise** (p.37): Bitwise NOT `~a`, AND `a & b`, OR `a | b`, XOR
//! `a ^ b`, left shift `a << b`, right shift `a >> b`. They are integer-domain
//! operators — applying one to a floating-point operand is not valid M1. The
//! manual's "Compound Assignment" table (p.38) adds the bitwise-assign forms
//! `a &= b`, `a |= b`, `a ^= b`, `a <<= b`, `a >>= b`, which carry the same
//! integer-only domain.
//!
//! This mirrors the other operator-domain rules (T002 float-equality, T003
//! float→int narrowing, T004 signed/unsigned ordering, T005 modulo-on-float):
//! a focused diagnostic for an illegal-operand case the generic expression
//! typer would otherwise type silently — a binary bitwise op just returns the
//! left operand's type, and unary `~` returns the operand type unchanged, so
//! `floatCh & 0xFF`, `~floatVal` and `floatVal << 2` produced no finding.
//!
//! The guard is `is_float`-only (silent on Integer/Unsigned/Unknown), so the
//! corpus's integer bit-manipulation (`flags & 0xFF`, `1 << bit`) stays clean —
//! `Unknown` is absorbing here as everywhere.
//!
//! Three forms are covered: the bare binary operator (`a & b`, a
//! `BinaryExpression`), the unary bitwise NOT (`~a`, a `UnaryExpression`), and
//! the compound assignments (`a &= b`, …, an `AssignmentStatement`).
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::typer::{is_expr, type_of};
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

const MSG: &str = "bitwise operators are integer-only (manual p.37 lists `~ & | ^ << >>` under \
                   Integer Bitwise); operand is floating point — convert with \
                   Convert.ToInteger / Convert.ToUnsignedInteger first";

/// The binary bitwise operator token kinds (manual p.37 "Integer Bitwise").
fn is_binary_bitwise(k: Kind) -> bool {
    matches!(
        k,
        Kind::Amp | Kind::Pipe | Kind::Caret | Kind::LtLt | Kind::GtGt
    )
}

/// The compound bitwise-assignment token kinds (manual p.38 "Compound
/// Assignment").
fn is_compound_bitwise(k: Kind) -> bool {
    matches!(
        k,
        Kind::AmpEq | Kind::PipeEq | Kind::CaretEq | Kind::LtLtEq | Kind::GtGtEq
    )
}

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        match node.kind() {
            Kind::BinaryExpression => self.check_binary(node, scope, out),
            Kind::UnaryExpression => self.check_unary(node, scope, out),
            Kind::AssignmentStatement => self.check_compound_assign(node, scope, out),
            _ => {}
        }
    }
}

impl Rule {
    /// A bare `a & b` / `a | b` / `a ^ b` / `a << b` / `a >> b` with a float
    /// operand on either side.
    fn check_binary(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let is_bitwise = node.children().iter().any(|c| is_binary_bitwise(c.kind()));
        if !is_bitwise {
            return;
        }
        let any_float = node
            .named_children()
            .into_iter()
            .filter(|c| is_expr(c.kind()))
            .any(|c| type_of(c, scope).is_float());
        if any_float {
            out.push(make(TypeCode::T006, node, Severity::Warning, MSG.into()));
        }
    }

    /// Bitwise NOT `~a` with a float operand. (Logical `not`/`!` are typed
    /// Boolean by the typer and never reach here as a float.)
    fn check_unary(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let is_tilde = node.children().iter().any(|c| c.kind() == Kind::Tilde);
        if !is_tilde {
            return;
        }
        let operand_is_float = node
            .named_children()
            .into_iter()
            .filter(|c| is_expr(c.kind()))
            .any(|c| type_of(c, scope).is_float());
        if operand_is_float {
            out.push(make(TypeCode::T006, node, Severity::Warning, MSG.into()));
        }
    }

    /// The compound form `a &= b` / `a |= b` / `a ^= b` / `a <<= b` / `a >>= b`
    /// (manual p.38) with a float operand on either side.
    fn check_compound_assign(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let Some(op) = node.child_by_field(Field::Operator) else {
            return;
        };
        if !is_compound_bitwise(op.kind()) {
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
            out.push(make(TypeCode::T006, node, Severity::Warning, MSG.into()));
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
    fn flags_each_binary_bitwise_operator_on_float() {
        // Every member of the "Integer Bitwise" table fires on a float operand.
        for op in ["&", "|", "^", "<<", ">>"] {
            let src = format!("local x = f {op} g;\n");
            let out = diags(&src, &[("f", ValueType::Float), ("g", ValueType::Float)]);
            assert_eq!(
                out.len(),
                1,
                "`{op}` on floats: expected one T006, got {out:?}"
            );
            assert_eq!(out[0].code, TypeCode::T006);
        }
    }

    #[test]
    fn flags_bitwise_against_integer_literal() {
        // `floatCh & 0xFF` — only one operand is float, still illegal for `&`.
        let out = diags("local x = f & 0xFF;\n", &[("f", ValueType::Float)]);
        assert_eq!(out.len(), 1, "expected one T006, got {out:?}");
        assert_eq!(out[0].code, TypeCode::T006);
    }

    #[test]
    fn flags_unary_bitwise_not_on_float() {
        // `~floatVal` — the typer used to return the operand type (Float) silently.
        let out = diags("local x = ~f;\n", &[("f", ValueType::Float)]);
        assert_eq!(out.len(), 1, "expected one T006, got {out:?}");
        assert_eq!(out[0].code, TypeCode::T006);
    }

    #[test]
    fn flags_each_compound_bitwise_assign_on_float() {
        // Every member of the bitwise "Compound Assignment" table (manual p.38).
        for op in ["&=", "|=", "^=", "<<=", ">>="] {
            let src = format!("f {op} 1;\n");
            let out = diags(&src, &[("f", ValueType::Float)]);
            assert_eq!(
                out.len(),
                1,
                "`{op}` on float target: expected one T006, got {out:?}"
            );
            assert_eq!(out[0].code, TypeCode::T006);
        }
    }

    #[test]
    fn flags_compound_assign_with_float_value() {
        // Integer target, float value via `<<=` — still a float operand.
        let out = diags(
            "i <<= f;\n",
            &[("i", ValueType::Integer), ("f", ValueType::Float)],
        );
        assert_eq!(out.len(), 1, "expected one T006, got {out:?}");
        assert_eq!(out[0].code, TypeCode::T006);
    }

    #[test]
    fn integer_bitwise_is_clean() {
        // Integer/unsigned bit-manipulation is the legitimate corpus shape.
        let amp = diags("local x = i & 0xFF;\n", &[("i", ValueType::Integer)]);
        assert!(amp.is_empty(), "integer `&` must be clean: {amp:?}");
        let shift = diags("local x = u << 2;\n", &[("u", ValueType::Unsigned)]);
        assert!(shift.is_empty(), "unsigned `<<` must be clean: {shift:?}");
        let not = diags("local x = ~i;\n", &[("i", ValueType::Integer)]);
        assert!(not.is_empty(), "integer `~` must be clean: {not:?}");
    }

    #[test]
    fn unknown_operand_is_silent() {
        // Unknown is absorbing — neither the binary nor the unary form fires.
        let bin = diags("local x = z & 0xFF;\n", &[]);
        assert!(bin.is_empty(), "unknown `&` must be silent: {bin:?}");
        let un = diags("local x = ~z;\n", &[]);
        assert!(un.is_empty(), "unknown `~` must be silent: {un:?}");
    }

    #[test]
    fn arithmetic_and_logical_on_float_are_not_t006() {
        // `+ - * /` are valid Floating Point Arithmetic; `and`/`or` are logical —
        // none are bitwise, so T006 must stay silent (T005 owns `%`).
        for op in ["+", "-", "*", "/", "and", "or"] {
            let src = format!("local x = f {op} g;\n");
            let out = diags(&src, &[("f", ValueType::Float), ("g", ValueType::Float)]);
            assert!(
                out.is_empty(),
                "`{op}` on floats must not fire T006: {out:?}"
            );
        }
    }

    #[test]
    fn integer_compound_bitwise_assign_is_clean() {
        let out = diags("i &= 0x0F;\n", &[("i", ValueType::Integer)]);
        assert!(out.is_empty(), "integer `&=` must be clean: {out:?}");
    }
}
