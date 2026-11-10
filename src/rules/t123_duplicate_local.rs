//! T123 duplicate-local: a local name already declared in the same block or an
//! enclosing one (M1 Build Error 1312). Names are case-sensitive here, and the
//! same name in sibling blocks (exclusive branches) is accepted.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use m1_core::{Field, Kind, Node, Severity};

pub fn check(root: Node, _scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    walk(root, &mut Vec::new(), out);
}

fn walk<'a>(node: Node<'a>, visible: &mut Vec<&'a str>, out: &mut Vec<TypeDiagnostic>) {
    let depth = visible.len();
    for child in node.children() {
        if child.kind() == Kind::LocalDeclaration
            && let Some(name) = child.child_by_field(Field::Name)
        {
            let text = name.text();
            if !text.contains("$(") && visible.contains(&text) {
                out.push(make(
                    TypeCode::T123,
                    &name,
                    Severity::Error,
                    format!(
                        "local `{text}` is already declared in this scope (M1 Build Error 1312)"
                    ),
                ));
            }
            visible.push(text);
        } else if child.kind() == Kind::Block {
            walk(child, visible, out);
        } else {
            // Blocks nested in if/else/when/expand open their own scope.
            nested_blocks(child, visible, out);
        }
    }
    visible.truncate(depth);
}

fn nested_blocks<'a>(node: Node<'a>, visible: &mut Vec<&'a str>, out: &mut Vec<TypeDiagnostic>) {
    for child in node.children() {
        if child.kind() == Kind::Block {
            walk(child, visible, out);
        } else {
            nested_blocks(child, visible, out);
        }
    }
}
