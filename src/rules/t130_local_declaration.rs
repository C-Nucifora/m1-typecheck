//! T130 local-declaration: declarations M1 Build rejects.
//!
//! - A local without an initial value fails to parse (Error 1025).
//! - `<Bool>` and `<Float>` are not M1 type names (Error 1313); M1 spells them
//!   `<Boolean>` and `<Floating Point>`. Other unknown names stay silent:
//!   module enumerations are valid but not all are modelled.
//! - Initialising from a user function that declares no return value is Error
//!   1314; from a library function that returns nothing, Error 1348.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

const INVALID_TYPE_ALIASES: &[(&str, &str)] = &[("bool", "Boolean"), ("float", "Floating Point")];

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::LocalDeclaration {
            return;
        }
        let name = node
            .child_by_field(Field::Name)
            .map(|n| n.text())
            .unwrap_or("");
        if let Some(ty) = node
            .child_by_field(Field::TypeAnnotation)
            .and_then(|a| a.child_by_field(Field::Type))
        {
            let spelled = ty.text().trim();
            if let Some((_, m1)) = INVALID_TYPE_ALIASES
                .iter()
                .find(|(alias, _)| alias.eq_ignore_ascii_case(spelled))
            {
                out.push(make(
                    TypeCode::T130,
                    &ty,
                    Severity::Error,
                    format!(
                        "`<{spelled}>` is not an M1 data type; use `<{m1}>` (M1 Build Error 1313)"
                    ),
                ));
            }
        }
        let Some(value) = node.child_by_field(Field::Value) else {
            out.push(make(
                TypeCode::T130,
                node,
                Severity::Error,
                format!(
                    "local `{name}` needs an initial value; M1 Build cannot parse the script (M1 Build Error 1025)"
                ),
            ));
            return;
        };
        if value.kind() != Kind::CallExpression {
            return;
        }
        let Some(callee) = value.child_by_field(Field::Function) else {
            return;
        };
        match resolve(callee.text(), scope) {
            Resolution::Symbol(function)
                if matches!(function.kind, SymbolKind::Function | SymbolKind::Method)
                    && function.in_params.is_some()
                    && function.declared_returns.is_empty() =>
            {
                out.push(make(
                    TypeCode::T130,
                    &value,
                    Severity::Error,
                    format!(
                        "local `{name}` is initialised from `{}`, which returns no value (M1 Build Error 1314)",
                        function.path
                    ),
                ));
            }
            Resolution::BuiltinFn(overloads)
                if !overloads.is_empty() && overloads.iter().all(|o| o.returns == "Void") =>
            {
                out.push(make(
                    TypeCode::T130,
                    &value,
                    Severity::Error,
                    format!(
                        "local `{name}` is initialised from `{}`, which returns no value (M1 Build Error 1348)",
                        callee.text()
                    ),
                ));
            }
            _ => {}
        }
    }
}
