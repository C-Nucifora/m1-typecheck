//! T118 unused-local: a local (or static local) that is never read is M1
//! Build Error 1350. Assignments, including compound ones (`x += 1`), do not
//! count as reads; passing the local to `System.Unused` does. Locals whose
//! names come from `$(…)` expansion are skipped.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use m1_core::{Field, Kind, Node, Severity};
use std::collections::HashSet;

pub fn check(root: Node, _scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    let mut declarations = Vec::new();
    let mut reads = HashSet::new();
    collect(root, &mut declarations, &mut reads);
    for name in declarations {
        let text = name.text();
        if text.contains("$(") || reads.contains(text) {
            continue;
        }
        out.push(make(
            TypeCode::T118,
            &name,
            Severity::Error,
            format!("local `{text}` is declared but never read (M1 Build Error 1350)"),
        ));
    }
}

fn collect<'a>(node: Node<'a>, declarations: &mut Vec<Node<'a>>, reads: &mut HashSet<&'a str>) {
    if node.kind() == Kind::LocalDeclaration
        && let Some(name) = node.child_by_field(Field::Name)
    {
        declarations.push(name);
    }
    if node.kind() == Kind::Identifier && is_read(&node) {
        reads.insert(node.text());
    }
    for child in node.children() {
        collect(child, declarations, reads);
    }
}

/// An identifier in value position: not a declaration name, a type name, a
/// member's property segment, or any assignment target.
fn is_read(node: &Node) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };
    let same = |field: Field| {
        parent
            .child_by_field(field)
            .is_some_and(|child| child.byte_range() == node.byte_range())
    };
    match parent.kind() {
        Kind::LocalDeclaration => !same(Field::Name),
        Kind::TypeAnnotation => false,
        Kind::MemberExpression => !same(Field::Property),
        Kind::AssignmentStatement => !same(Field::Target),
        _ => true,
    }
}
