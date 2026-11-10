//! T121 constant-logic-result: a logical or comparison operation whose
//! operands are both literals always gives the same result (M1 Build warning
//! 1342), e.g. `true and false` or `1 > 0`.
use super::native_syntax::{BinaryOp, binary_op, is_literal, operands, operator_text};
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use m1_core::{Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, _scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if !matches!(
            binary_op(node),
            Some(BinaryOp::And | BinaryOp::Or | BinaryOp::Equality | BinaryOp::Relational)
        ) {
            return;
        }
        let Some((left, right)) = operands(node) else {
            return;
        };
        if is_literal(left) && is_literal(right) {
            out.push(make(
                TypeCode::T121,
                node,
                Severity::Warning,
                format!(
                    "result of logic operation `{}` is a constant value (M1 Build warning 1342)",
                    operator_text(node)
                ),
            ));
        }
    }
}
