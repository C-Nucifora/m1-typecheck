use crate::diagnostics::TypeDiagnostic;
use crate::resolve::Scope;
use m1_core::Node;

pub struct Rule;
impl super::Rule for Rule {
    fn check_node(&self, _node: &Node, _scope: &Scope, _out: &mut Vec<TypeDiagnostic>) {}
}
