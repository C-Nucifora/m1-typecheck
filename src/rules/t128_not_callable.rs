//! T128 not-callable: call syntax applied to a channel, parameter or constant
//! (M1 Build Error 1336). Only functions and methods can be executed.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::CallExpression {
            return;
        }
        let Some(callee) = node.child_by_field(Field::Function) else {
            return;
        };
        let Resolution::Symbol(symbol) = resolve(callee.text(), scope) else {
            return;
        };
        let what = match symbol.kind {
            SymbolKind::Channel => "channel",
            SymbolKind::Parameter => "parameter",
            SymbolKind::Constant => "constant",
            _ => return,
        };
        out.push(make(
            TypeCode::T128,
            &callee,
            Severity::Error,
            format!(
                "{what} `{}` is not an executable function (M1 Build Error 1336)",
                symbol.path
            ),
        ));
    }
}
