//! T106 — group-not-a-value-provider (M1 Build Error 1331, #242).
//!
//! A `BuiltIn.GroupCompound` is a *value provider* — usable where a value is
//! expected — only when it declares a Default Value
//! (`<Props UseDefValue="true" DefValue="…">`, captured as
//! [`crate::symbols::Symbol::default_value`]). Passing a bare group with no
//! default value where a value is expected silently passed the CLI but M1 Build
//! rejects it (Error 1331), and the failure cascades into overload-resolution,
//! unknown-type-local and assignment errors (1302/1314/1320). Flagging the bare
//! group at the value site surfaces the root cause.
//!
//! The value sites checked are the ones where a group is unambiguously used *as
//! a value* (never as a namespace prefix): a function/method **call argument**,
//! an **assignment** right-hand side, and a **local declaration** initialiser.
//! A member access like `Group.Channel` resolves to the channel (not the group),
//! so it is never flagged.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make, related_to_def};
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use crate::typer::{is_expr, path_text};
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        match node.kind() {
            Kind::CallExpression => {
                // Every argument expression is a value position.
                if let Some(args) = node.child_nodes().find(|c| c.kind() == Kind::ArgumentList) {
                    for arg in args
                        .named_children()
                        .into_iter()
                        .filter(|c| is_expr(c.kind()))
                    {
                        check_value_expr(&arg, scope, out);
                    }
                }
            }
            // The right-hand side of an assignment / a local initialiser.
            Kind::AssignmentStatement | Kind::LocalDeclaration => {
                if let Some(value) = node.child_by_field(Field::Value) {
                    check_value_expr(&value, scope, out);
                }
            }
            _ => {}
        }
    }
}

/// Flag `expr` when it is a bare reference to a [`SymbolKind::Group`] that
/// declares no Default Value — a group used as a value but not a value provider.
fn check_value_expr(expr: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    if !is_expr(expr.kind()) {
        return;
    }
    let Resolution::Symbol(sym) = resolve(&path_text(*expr), scope) else {
        return;
    };
    if sym.kind != SymbolKind::Group || sym.default_value.is_some() {
        return;
    }
    let mut d = make(
        TypeCode::T106,
        expr,
        Severity::Error,
        format!(
            "group `{}` is not a value provider — it has no Default Value, so it \
             cannot be used where a value is expected (M1 Build Error 1331)",
            sym.path
        ),
    );
    d.related.extend(related_to_def(
        sym,
        format!("group `{}` declared here", sym.path),
    ));
    out.push(d);
}
