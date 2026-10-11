//! Call-site checks for project user functions with known scalar signatures.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::typer::path_text;
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
        let path = path_text(callee);
        let Resolution::Symbol(symbol) = resolve(&path, scope) else {
            return;
        };
        if symbol.classname.as_deref() != Some("BuiltIn.FuncUserParam")
            || symbol.declared_returns.len() != 1
            || !symbol.declared_returns[0].value_type.is_known()
        {
            return;
        }
        let mut parent = node.parent();
        let mut direct_result = true;
        while let Some(p) = parent {
            match p.kind() {
                Kind::ExpressionStatement if direct_result => {
                    out.push(make(
                        TypeCode::T114,
                        node,
                        Severity::Error,
                        format!(
                            "return value of function `{path}` is discarded (M1 Build Error 1349)"
                        ),
                    ));
                    return;
                }
                Kind::IfStatement | Kind::ArgumentList => {
                    out.push(make(
                        TypeCode::T115,
                        node,
                        Severity::Error,
                        format!("function `{path}` cannot be called inside an `if` condition or function argument; assign its result to a local first (M1 Build Error 1372)"),
                    ));
                    return;
                }
                Kind::LocalDeclaration
                | Kind::AssignmentStatement
                | Kind::ExpressionStatement
                | Kind::Block
                | Kind::ElseClause
                | Kind::SourceFile => return,
                Kind::ParenthesizedExpression => {}
                // An operator consumes the value, but its containing
                // condition or argument can still reject the user call.
                _ => direct_result = false,
            }
            parent = p.parent();
        }
    }
}
