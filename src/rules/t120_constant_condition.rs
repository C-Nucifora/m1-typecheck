//! T120 constant-condition: an `if` condition (M1 Build Error 1343) or an
//! inline `?:` condition (Error 1344) that is a Boolean literal.
use super::native_syntax::unparenthesized;
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, _scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let native = match node.kind() {
            Kind::IfStatement => "1343",
            Kind::TernaryExpression => "1344",
            _ => return,
        };
        let Some(condition) = node.child_by_field(Field::Condition) else {
            return;
        };
        if unparenthesized(condition).kind() == Kind::Boolean {
            out.push(make(
                TypeCode::T120,
                &condition,
                Severity::Error,
                format!(
                    "condition `{}` is a constant value (M1 Build Error {native})",
                    condition.text()
                ),
            ));
        }
    }
}
