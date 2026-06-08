use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::typer::{is_expr, type_of};
use crate::types::ValueType;
use m1_core::{Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::BinaryExpression {
            return;
        }
        let is_eq = node
            .children()
            .iter()
            .any(|c| matches!(c.kind(), Kind::EqEq | Kind::BangEq | Kind::Eq | Kind::Neq));
        if !is_eq {
            return;
        }
        let operands: Vec<_> = node
            .named_children()
            .into_iter()
            .filter(|c| is_expr(c.kind()))
            .map(|o| type_of(o, scope))
            .collect();
        // Comparing an enum to anything is not a float-precision concern even
        // when the other side is a float literal (`State eq 1.0`): the value is
        // a discrete enumerator, not a continuous float. That mismatch is T021's
        // job; firing T002 too is a spurious duplicate. Stay silent here.
        if operands.iter().any(|t| t.is_enum()) {
            return;
        }
        let any_float = operands.contains(&ValueType::Float);
        if any_float {
            out.push(make(
                TypeCode::T002,
                node,
                Severity::Error,
                "never compare floats with equality operators; use a tolerance check".into(),
            ));
        }
    }
}
