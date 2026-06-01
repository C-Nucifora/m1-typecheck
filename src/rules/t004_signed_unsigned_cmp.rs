//! T004 — signed/unsigned ordering comparison.
//!
//! Ordering an `Unsigned` value against a signed `Integer` (`<`, `>`, `<=`,
//! `>=`) is where signed/unsigned mixing actually bites: the operand is
//! promoted and the comparison can flip versus the programmer's intent (e.g.
//! `unsignedCh < signedOffset` when the signed side is negative wraps). Unlike
//! arithmetic mixing (which M1 Build handles implicitly across the corpus), the
//! ordering result genuinely diverges, so it is worth a warning.
//!
//! Scope is deliberately narrow (#18, Option 1): only ordering comparisons, and
//! only when *both* operands are non-literal. Integer/hex literals carry a sign
//! the compiler knows at compile time, so `unsignedCh < 5` / `signedCh < 0xFF`
//! are allowed — flagging them was the bulk of the false positives a blanket
//! rule produced on the corpus.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::typer::type_of;
use crate::types::ValueType;
use m1_core::{Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::BinaryExpression {
            return;
        }
        let is_ordering = node
            .children()
            .iter()
            .any(|c| matches!(c.kind(), Kind::Lt | Kind::Gt | Kind::LtEq | Kind::GtEq));
        if !is_ordering {
            return;
        }
        let ops: Vec<_> = node
            .named_children()
            .into_iter()
            .filter(|c| is_expr(c.kind()))
            .collect();
        let (Some(l), Some(r)) = (ops.first(), ops.get(1)) else {
            return;
        };
        // Allow literal operands: the compiler knows their sign at compile time.
        if l.kind() == Kind::Number || r.kind() == Kind::Number {
            return;
        }
        let (lt, rt) = (type_of(*l, scope), type_of(*r, scope));
        if signed_unsigned(lt, rt) {
            out.push(make(
                TypeCode::T004,
                node,
                Severity::Warning,
                "ordering comparison mixes signed and unsigned; the result can flip on \
                 wraparound — convert one side with Convert.ToInteger/ToUnsigned"
                    .into(),
            ));
        }
    }
}

fn signed_unsigned(lt: ValueType, rt: ValueType) -> bool {
    (lt == ValueType::Unsigned && rt == ValueType::Integer)
        || (lt == ValueType::Integer && rt == ValueType::Unsigned)
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
    use crate::rules::Rule as _;
    use crate::types::ValueType;

    fn diags(src: &str, locals: &[(&str, ValueType)]) -> Vec<TypeDiagnostic> {
        let cst = m1_core::parse(src);
        let scope = Scope {
            locals: locals.iter().map(|(n, t)| (n.to_string(), *t)).collect(),
            group: None,
            project: None,
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
    fn flags_signed_unsigned_ordering() {
        let out = diags(
            "local x = u < s;\n",
            &[("u", ValueType::Unsigned), ("s", ValueType::Integer)],
        );
        assert_eq!(out.len(), 1, "expected one T004, got {out:?}");
        assert_eq!(out[0].code, TypeCode::T004);
    }

    #[test]
    fn allows_literal_operand() {
        // unsigned vs integer *literal* — sign known at compile time, no warning.
        let out = diags("local x = u < 5;\n", &[("u", ValueType::Unsigned)]);
        assert!(out.is_empty(), "literal operand should be allowed: {out:?}");
    }

    #[test]
    fn ignores_equality_and_same_sign() {
        // equality is signedness-agnostic; same-sign ordering is fine.
        let eq = diags(
            "local x = u == s;\n",
            &[("u", ValueType::Unsigned), ("s", ValueType::Integer)],
        );
        assert!(eq.is_empty(), "equality should not be flagged: {eq:?}");
        let same = diags(
            "local x = a < b;\n",
            &[("a", ValueType::Unsigned), ("b", ValueType::Unsigned)],
        );
        assert!(same.is_empty(), "same-sign ordering is fine: {same:?}");
    }
}
