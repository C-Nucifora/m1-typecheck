use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        // Only check a member-expression or bare identifier that is NOT part of a
        // larger member-expression (avoid double-checking every prefix). We
        // evaluate each full path once at its outermost node.
        if !matches!(node.kind(), Kind::Identifier | Kind::MemberExpression) {
            return;
        }
        if let Some(parent) = node.parent() {
            if parent.kind() == Kind::MemberExpression {
                return; // part of a longer path; the outer node carries the full path
            }
            // Skip the type name inside `<Type>` annotations.
            if parent.kind() == Kind::TypeAnnotation {
                return;
            }
        }
        // The `state` of a `when…is (State)` clause is always an *enumerator* of
        // the subject's enum (which may be a firmware enum whose members we don't
        // model), never a project reference — its membership is T020/T070's job,
        // not an unresolved-reference miss.
        if in_is_clause_state(node) {
            return;
        }
        if !matches!(resolve(node.text(), scope), Resolution::Unresolved) {
            return;
        }
        // A miss in *write* position — the target of a plain `=` assignment — is
        // not an unresolved *reference* (a read): it is a write to a target the
        // project doesn't know about. Reporting it as "unresolved reference" is
        // misleading (issue #19), so it gets the distinct T031 code with a
        // clearer message. A compound assignment (`+=`, `-=`, …) reads the
        // target before writing, so an unresolved compound target IS a genuine
        // unresolved read and stays T001.
        if is_plain_assignment_target(node) {
            // M1 Build rejects a write to a non-existent object outright
            // (Errors 1338/1352 "does not exist"), so this fails the build —
            // an error, not a warning (#174).
            out.push(make(
                TypeCode::T031,
                node,
                Severity::Error,
                format!(
                    "assignment to unknown target `{}` (M1 Build Error 1338: \"does not exist\")",
                    node.text()
                ),
            ));
        } else {
            out.push(make(
                TypeCode::T001,
                node,
                Severity::Warning,
                format!("unresolved reference `{}`", node.text()),
            ));
        }
    }
}

/// True when `node` sits in the `state` position of a `when…is (State)` clause
/// (walking up we reach the `IsClause` before entering its body `Block`). Such
/// names are enumerators, not project references.
fn in_is_clause_state(node: &Node) -> bool {
    let mut cur = *node;
    while let Some(p) = cur.parent() {
        match p.kind() {
            Kind::IsClause => return true,
            Kind::Block => return false, // inside the is-clause body, not its state
            _ => cur = p,
        }
    }
    false
}

/// True when `node` is the `target` of a plain `=` assignment (a pure write).
/// Compound assignments (`+=`, `-=`, `*=`, `/=`) read the target first, so they
/// are deliberately excluded.
fn is_plain_assignment_target(node: &Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != Kind::AssignmentStatement {
        return false;
    }
    // Plain `=` only (a compound op implies a read of the target).
    if !parent.children().iter().any(|c| c.kind() == Kind::Assign) {
        return false;
    }
    parent
        .child_by_field(Field::Target)
        .is_some_and(|t| t.byte_range() == node.byte_range())
}
