//! T125 return-value-use: `Out` can only be assigned. Reading it (or one of
//! its named results) is M1 Build Error 1356. In a function whose signature
//! declares no return value, `Out` does not exist (Error 1338). Functions
//! without a `<Signature>` stay silent.
use super::native_syntax::{is_outermost_path, is_plain_assignment_target};
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use m1_core::{Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if !is_outermost_path(node) {
            return;
        }
        let text = node.text();
        if text != "Out" && !text.starts_with("Out.") {
            return;
        }
        let Some(function) = scope
            .project
            .zip(scope.fn_symbol.as_ref())
            .and_then(|(project, path)| project.symbols().get(path))
        else {
            return;
        };
        if function.in_params.is_none() {
            return; // no <Signature>: return contract unknown
        }
        if function.declared_returns.is_empty() {
            out.push(make(
                TypeCode::T125,
                node,
                Severity::Error,
                format!(
                    "`{}` declares no return value, so `Out` does not exist (M1 Build Error 1338)",
                    function.path
                ),
            ));
        } else if !is_plain_assignment_target(node) {
            out.push(make(
                TypeCode::T125,
                node,
                Severity::Error,
                format!(
                    "return value `{text}` can only be assigned, not read (M1 Build Error 1356)"
                ),
            ));
        }
    }
}
