//! T124 read-only-assignment: device functions may assign only channels and
//! locals. Assigning a parameter or constant is M1 Build Error 1318, and
//! assigning a function argument (`In.<name>`) is Error 1357. Tune methods
//! (`BuiltIn.CalFuncUser`) may assign parameters and are not checked.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::AssignmentStatement {
            return;
        }
        let Some(target) = node.child_by_field(Field::Target) else {
            return;
        };
        let function = scope
            .project
            .zip(scope.fn_symbol.as_ref())
            .and_then(|(project, path)| project.symbols().get(path));
        if function.is_some_and(|f| f.classname.as_deref() == Some("BuiltIn.CalFuncUser")) {
            return;
        }
        let text = target.text();
        if let Some(argument) = text.strip_prefix("In.")
            && function
                .and_then(|f| f.in_params.as_ref())
                .is_some_and(|params| params.iter().any(|(name, _)| name == argument))
        {
            out.push(make(
                TypeCode::T124,
                &target,
                Severity::Error,
                format!("argument `{argument}` can only be read; it cannot be assigned (M1 Build Error 1357)"),
            ));
            return;
        }
        if let Resolution::Symbol(symbol) = resolve(text, scope) {
            let what = match symbol.kind {
                SymbolKind::Parameter => "parameter",
                SymbolKind::Constant => "constant",
                _ => return,
            };
            out.push(make(
                TypeCode::T124,
                &target,
                Severity::Error,
                format!(
                    "{what} `{}` is read-only; only channels and locals can be assigned (M1 Build Error 1318)",
                    symbol.path
                ),
            ));
        }
    }
}
