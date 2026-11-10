//! T126 ambiguous-logical-operators: `and` and `or` mixed without parentheses
//! (`a and b or c`) is rejected by M1 Build (Error 1365) even though the
//! grammar gives `and` the higher precedence.
use super::native_syntax::{BinaryOp, binary_op, operands};
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use m1_core::{Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, _scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let other = match binary_op(node) {
            Some(BinaryOp::And) => BinaryOp::Or,
            Some(BinaryOp::Or) => BinaryOp::And,
            _ => return,
        };
        let Some((left, right)) = operands(node) else {
            return;
        };
        if binary_op(&left) == Some(other) || binary_op(&right) == Some(other) {
            out.push(make(
                TypeCode::T126,
                node,
                Severity::Error,
                "`and` and `or` are mixed without parentheses; add parentheses to define the order (M1 Build Error 1365)"
                    .into(),
            ));
        }
    }
}
