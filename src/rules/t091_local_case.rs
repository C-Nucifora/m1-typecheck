//! T091 local-object-case-ambiguity (manual pp.64–65).
//!
//! The manual's *Naming Conventions*: names must be unambiguous — "not to
//! differentiate names only by using uppercase/lowercase characters" — and,
//! under *Naming Local Variables*, "Don't distinguish local variable names from
//! other object names only by uppercase/lowercase writing (problems with
//! maintaining references will occur)".
//!
//! This pass flags a `local` whose name case-insensitively matches the *leaf*
//! segment of a project symbol referenced **in the same script** — the scope
//! where the two spellings actually sit side by side and confuse maintenance.
//! Symbols owned by a package-defined object instance are exempt (their
//! `Value`/`State` leaves are MoTeC's naming, not the user's — the same
//! `crate::audit::ObjectOwnership` rule the project audits apply).
//!
//! Whole-tree pass (like `flow.rs`), not a node-at-a-time rule: it needs the
//! full set of references in the file before it can judge any declaration.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use m1_core::{Field, Kind, Node, Severity};
use std::collections::{BTreeSet, HashMap};

pub fn check(root: Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    if scope.project.is_none() {
        return; // needs a symbol table to compare against
    }
    // 1. Every project symbol referenced in this script, keyed by lower-cased
    //    leaf. Outermost path nodes only (same guard as T001) so `a.b.c` is
    //    resolved once, not per prefix.
    let mut referenced: HashMap<String, BTreeSet<String>> = HashMap::new();
    collect_references(root, scope, &mut referenced);
    if referenced.is_empty() {
        return;
    }
    let ownership = crate::audit::ObjectOwnership::new(scope.project.unwrap().symbols());

    // 2. Compare every local declaration against the referenced leaves.
    let mut decls = Vec::new();
    collect_local_names(root, &mut decls);
    for name_node in decls {
        let name = name_node.text();
        let Some(paths) = referenced.get(&name.to_ascii_lowercase()) else {
            continue;
        };
        for path in paths {
            if ownership.owned_by_object(path) {
                continue;
            }
            let leaf = crate::audit::leaf_of(path);
            let message = if leaf == name {
                format!(
                    "local `{name}` has the same name as the object `{path}` referenced in this script"
                )
            } else {
                format!(
                    "local `{name}` differs from the object `{path}` referenced in this script only by case"
                )
            };
            out.push(make(TypeCode::T091, &name_node, Severity::Warning, message));
        }
    }
}

/// Record, for every outermost identifier/member path that resolves to a
/// project symbol, the symbol's path under its lower-cased leaf segment.
fn collect_references(node: Node, scope: &Scope, out: &mut HashMap<String, BTreeSet<String>>) {
    if matches!(node.kind(), Kind::Identifier | Kind::MemberExpression) {
        let outermost = node
            .parent()
            .is_none_or(|p| p.kind() != Kind::MemberExpression && p.kind() != Kind::TypeAnnotation);
        if outermost && let Resolution::Symbol(sym) = resolve(node.text(), scope) {
            out.entry(crate::audit::leaf_of(&sym.path).to_ascii_lowercase())
                .or_default()
                .insert(sym.path.clone());
        }
    }
    for c in node.children() {
        collect_references(c, scope, out);
    }
}

/// Collect the `name` node of every `local` declaration, in document order.
fn collect_local_names<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
    if node.kind() == Kind::LocalDeclaration
        && let Some(name) = node.child_by_field(Field::Name)
    {
        out.push(name);
    }
    for c in node.children() {
        collect_local_names(c, out);
    }
}
