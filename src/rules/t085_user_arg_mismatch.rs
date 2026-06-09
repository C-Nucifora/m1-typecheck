//! T085 — user-argument-mismatch (#110). A call to a user function or method
//! whose `.m1prj` `<Signature>` declares parameters is checked for arity
//! (Error — M1 Build rejects the build) and per-position argument type
//! compatibility (Warning). Functions without a declared signature stay
//! unchecked — T064 covers the library-function arity case.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use crate::typer::{is_expr, path_text, type_of};
use crate::types::ValueType;
use m1_core::{Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::CallExpression {
            return;
        }
        let Some(callee) = node
            .named_children()
            .into_iter()
            .find(|c| matches!(c.kind(), Kind::Identifier | Kind::MemberExpression))
        else {
            return;
        };
        let callee_path = path_text(callee);
        let Resolution::Symbol(sym) = resolve(&callee_path, scope) else {
            return;
        };
        if !matches!(sym.kind, SymbolKind::Function | SymbolKind::Method) {
            return;
        }
        let Some(params) = &sym.in_params else {
            return; // no declared signature — unchecked
        };
        let args: Vec<Node> = node
            .child_nodes()
            .find(|c| c.kind() == Kind::ArgumentList)
            .map(|al| {
                al.named_children()
                    .into_iter()
                    .filter(|c| is_expr(c.kind()))
                    .collect()
            })
            .unwrap_or_default();

        if args.len() != params.len() {
            out.push(make(
                TypeCode::T085,
                node,
                Severity::Error,
                format!(
                    "`{}` takes {} argument{}, but {} {} supplied",
                    callee_path,
                    params.len(),
                    if params.len() == 1 { "" } else { "s" },
                    args.len(),
                    if args.len() == 1 { "is" } else { "are" },
                ),
            ));
            return;
        }
        for (arg, (pname, pty)) in args.iter().zip(params) {
            let aty = type_of(*arg, scope);
            if !aty.is_known() || !pty.is_known() {
                continue;
            }
            if !arg_compatible(*pty, aty) {
                out.push(make(
                    TypeCode::T085,
                    arg,
                    Severity::Warning,
                    format!(
                        "argument `{}` of `{}` is declared {pty:?}, but a {aty:?} is passed",
                        pname, callee_path
                    ),
                ));
            }
        }
    }
}

/// Argument-passing compatibility — same lattice as assignment (T030):
/// integral widths interchange, integers widen into a float parameter, but a
/// float into an integral parameter (or any enum/numeric mix) flags.
fn arg_compatible(param: ValueType, arg: ValueType) -> bool {
    use ValueType::*;
    match (param, arg) {
        (a, b) if a == b => true,
        (Integer, Unsigned) | (Unsigned, Integer) => true,
        (Float, Integer) | (Float, Unsigned) => true,
        (Enum(a), Enum(b)) => a == b,
        _ => false,
    }
}
