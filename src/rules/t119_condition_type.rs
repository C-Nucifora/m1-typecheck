//! T119 condition-not-boolean: an `if` condition (M1 Build Error 1351) or an
//! inline `?:` condition (Error 1359) whose known type is not Boolean. Unknown
//! types stay silent.
use super::native_syntax::unparenthesized;
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::typer::type_of;
use crate::types::ValueType;
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let (native, construct) = match node.kind() {
            Kind::IfStatement => ("1351", "`if` condition"),
            Kind::TernaryExpression => ("1359", "inline `?:` condition"),
            _ => return,
        };
        let Some(condition) = node.child_by_field(Field::Condition) else {
            return;
        };
        let ty = type_of(unparenthesized(condition), scope);
        if ty.is_known() && ty != ValueType::Boolean {
            out.push(make(
                TypeCode::T119,
                &condition,
                Severity::Error,
                format!("{construct} must evaluate to a Boolean value (M1 Build Error {native})"),
            ));
        }
    }
}
