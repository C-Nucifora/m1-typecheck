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
        let is_cmp = node.children().iter().any(|c| {
            matches!(
                c.kind(),
                Kind::EqEq
                    | Kind::BangEq
                    | Kind::Eq
                    | Kind::Neq
                    | Kind::Lt
                    | Kind::Gt
                    | Kind::LtEq
                    | Kind::GtEq
            )
        });
        if !is_cmp {
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
        let (lt, rt) = (type_of(*l, scope), type_of(*r, scope));
        let enum_vs_num = (lt.is_enum() && is_numeric(rt)) || (rt.is_enum() && is_numeric(lt));
        if enum_vs_num {
            // M1 Build rejects the comparison outright (Error 1329
            // "Incompatible data types for binary operation"), so this is an
            // error: the build fails, not a style nit (#173).
            out.push(make(
                TypeCode::T021,
                node,
                Severity::Error,
                "comparing an enum to a number; compare against an enum member instead (M1 Build Error 1329: incompatible data types)".into(),
            ));
        }
    }
}

fn is_numeric(t: ValueType) -> bool {
    matches!(
        t,
        ValueType::Integer | ValueType::Unsigned | ValueType::Float
    )
}
