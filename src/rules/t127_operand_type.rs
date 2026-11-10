//! T127 operand-type: operands M1 Build rejects for logic and comparison
//! operators. `not` needs a Boolean operand (Error 1324); `and`/`or` need
//! Boolean operands (Error 1329); an ordering comparison between a
//! floating-point and an integer value needs one side converted (Error
//! 1367). A numeric literal may be compared with either kind. Unknown types
//! stay silent; float equality, enum and signedness mixes have their own codes.
use super::native_syntax::{
    BinaryOp, binary_op, is_literal, is_logical_not, operands, operator_text, unary_operand,
};
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::typer::type_of;
use crate::types::ValueType;
use m1_core::{Node, Severity};

pub struct Rule;

fn not_boolean(ty: ValueType) -> bool {
    ty.is_known() && ty != ValueType::Boolean
}

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if is_logical_not(node) {
            if let Some(operand) = unary_operand(node)
                && not_boolean(type_of(operand, scope))
            {
                out.push(make(
                    TypeCode::T127,
                    node,
                    Severity::Error,
                    "unary operator `not` needs a Boolean operand (M1 Build Error 1324)".into(),
                ));
            }
            return;
        }
        let Some(op) = binary_op(node) else {
            return;
        };
        let Some((left, right)) = operands(node) else {
            return;
        };
        let (lt, rt) = (type_of(left, scope), type_of(right, scope));
        match op {
            BinaryOp::And | BinaryOp::Or if not_boolean(lt) || not_boolean(rt) => {
                out.push(make(
                    TypeCode::T127,
                    node,
                    Severity::Error,
                    format!(
                        "logic operator `{}` needs Boolean operands (M1 Build Error 1329)",
                        operator_text(node)
                    ),
                ));
            }
            BinaryOp::Relational
                if !is_literal(left)
                    && !is_literal(right)
                    && ((lt.is_float() && rt.is_integral())
                        || (lt.is_integral() && rt.is_float())) =>
            {
                out.push(make(
                    TypeCode::T127,
                    node,
                    Severity::Error,
                    format!(
                        "comparison `{}` mixes floating-point and integer values; convert one side (M1 Build Error 1367)",
                        operator_text(node)
                    ),
                ));
            }
            _ => {}
        }
    }
}
