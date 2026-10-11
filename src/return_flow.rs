//! Path counts for assignments to a declared scalar return sink.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use m1_core::{Field, Kind, Node, Severity};

#[derive(Default)]
struct Counts {
    min: u32,
    max: u32,
    incomplete_join: bool,
}

/// Check scalar `Out` assignments across straight-line and `if`/`else` paths.
/// Callers gate this on a known, single declared return. Unsupported return
/// writes inside `when`/`expand`, or through members, leave this pass silent.
pub(crate) fn check(root: Node, out: &mut Vec<TypeDiagnostic>) {
    let Some(counts) = counts(root) else { return };
    if counts.max >= 2 {
        out.push(make(
            TypeCode::T112,
            &root,
            Severity::Error,
            "return value `Out` is assigned more than once on a single code path (M1 Build Error 1354)".into(),
        ));
    }
    if counts.incomplete_join || (counts.max > 0 && counts.min == 0) {
        out.push(make(
            TypeCode::T113,
            &root,
            Severity::Error,
            "not all code paths assign to return value `Out` (M1 Build Error 1366)".into(),
        ));
    }
}

fn counts(node: Node) -> Option<Counts> {
    match node.kind() {
        Kind::AssignmentStatement => {
            let target = node.child_by_field(Field::Target)?;
            if target.text().starts_with("Out.") {
                return None;
            }
            if target.text() != "Out" {
                return Some(Counts::default());
            }
            if !node.child_nodes().any(|c| c.kind() == Kind::Assign) {
                return None;
            }
            Some(Counts {
                min: 1,
                max: 1,
                incomplete_join: false,
            })
        }
        Kind::IfStatement => {
            let then = node.child_nodes().find(|c| c.kind() == Kind::Block)?;
            let then = counts(then)?;
            let other = match node.child_nodes().find(|c| c.kind() == Kind::ElseClause) {
                Some(other) => counts(other)?,
                None => Counts::default(),
            };
            Some(Counts {
                min: then.min.min(other.min),
                max: then.max.max(other.max),
                incomplete_join: then.incomplete_join
                    || other.incomplete_join
                    || (then.min.min(other.min) == 0 && then.max.max(other.max) > 0),
            })
        }
        Kind::WhenStatement | Kind::ExpandStatement => {
            let writes_out = node.descendants().any(|n| {
                n.kind() == Kind::AssignmentStatement
                    && n.child_by_field(Field::Target)
                        .is_some_and(|t| t.text() == "Out" || t.text().starts_with("Out."))
            });
            if writes_out {
                None
            } else {
                Some(Counts::default())
            }
        }
        Kind::SourceFile | Kind::Block | Kind::ElseClause => {
            let mut result = Counts::default();
            for child in node.child_nodes() {
                let child = counts(child)?;
                // A conditional join must either leave Out untouched or assign
                // it on every branch. Assignments outside that conditional do
                // not repair a branch that omits the assignment.
                result.incomplete_join |= child.incomplete_join;
                result.min = result.min.saturating_add(child.min);
                result.max = result.max.saturating_add(child.max);
            }
            Some(result)
        }
        _ => Some(Counts::default()),
    }
}
