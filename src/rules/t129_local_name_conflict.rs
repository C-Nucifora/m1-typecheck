//! T129 local-name-conflict: a local whose name matches, ignoring case, an
//! object visible by bare name from the function — a child of the function's
//! group or of any enclosing group up to `Root` (M1 Build Error 1311).
//! Library object names and objects deeper in the tree do not conflict.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use m1_core::{Field, Kind, Node, Severity};

pub fn check(root: Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    let (Some(project), Some(group)) = (scope.project, scope.group.as_deref()) else {
        return;
    };
    let table = project.symbols();
    let mut scopes = vec![group.to_string()];
    let mut current = group;
    while let Some((parent, _)) = current.rsplit_once('.') {
        scopes.push(parent.to_string());
        current = parent;
    }
    let mut declarations = Vec::new();
    collect(root, &mut declarations);
    for name in declarations {
        let text = name.text();
        if text.contains("$(") {
            continue;
        }
        let conflict = scopes.iter().find_map(|scope_path| {
            table
                .immediate_children(scope_path)
                .into_iter()
                .find(|child| crate::audit::leaf_of(&child.path).eq_ignore_ascii_case(text))
        });
        if let Some(object) = conflict {
            out.push(make(
                TypeCode::T129,
                &name,
                Severity::Error,
                format!(
                    "local `{text}` conflicts with the object `{}` of the same name (M1 Build Error 1311)",
                    object.path
                ),
            ));
        }
    }
}

fn collect<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
    if node.kind() == Kind::LocalDeclaration
        && let Some(name) = node.child_by_field(Field::Name)
    {
        out.push(name);
    }
    for child in node.children() {
        collect(child, out);
    }
}
