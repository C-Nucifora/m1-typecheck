//! T122 discarded-logic-result: a logical (`and`/`or`) or comparison
//! operation used as a statement has no effect (M1 Build Error 1347).
//! Arithmetic and `not` statements are accepted by M1 Build.
use super::native_syntax::{BinaryOp, binary_op, unparenthesized};
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use m1_core::{Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, _scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::ExpressionStatement {
            return;
        }
        let Some(expression) = node
            .named_children()
            .into_iter()
            .find(|child| crate::typer::is_expr(child.kind()))
        else {
            return;
        };
        if matches!(
            binary_op(&unparenthesized(expression)),
            Some(BinaryOp::And | BinaryOp::Or | BinaryOp::Equality | BinaryOp::Relational)
        ) {
            out.push(make(
                TypeCode::T122,
                node,
                Severity::Error,
                "result of logic operation is not assigned; the statement has no effect (M1 Build Error 1347)"
                    .into(),
            ));
        }
    }
}
