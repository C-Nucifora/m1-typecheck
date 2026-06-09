use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::typer::type_of;
use crate::types::ValueType;
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::WhenStatement {
            return;
        }
        let Some(subject) = node.child_by_field(Field::Subject) else {
            return;
        };
        let subject_ty = type_of(subject, scope);
        let ValueType::Enum(id) = subject_ty else {
            // Manual p.32: "The [argument] used in the when statement must be of
            // an enumerated data type." A subject of a KNOWN non-enum type is
            // T082; an Unknown subject stays silent (conservative, like the
            // other rules).
            if subject_ty.is_known() {
                out.push(make(
                    TypeCode::T082,
                    &subject,
                    Severity::Error,
                    format!(
                        "`when` subject must be an enumerated type; `{}` is {subject_ty:?}",
                        subject.text().trim()
                    ),
                ));
            }
            return;
        };
        let Some(project) = scope.project else { return };
        let enum_ty = project.symbols().enum_type(id);

        // Collect the enumerators covered across every `is` arm (one arm may list
        // several via `or`). If an arm names something that is NOT a member of the
        // enum, treat it as a catch-all/default and consider the `when` exhaustive.
        let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
        for arm in node.named_children() {
            if arm.kind() != Kind::IsClause {
                continue;
            }
            let Some(state) = arm.child_by_field(Field::State) else {
                continue;
            };
            let names = arm_enumerators(state);
            // An arm we cannot decompose into enumerators (empty), or one naming a
            // label that is not a member of the enum, acts as a catch-all/default:
            // bail out conservatively and emit nothing.
            if names.is_empty() {
                return;
            }
            for name in names {
                if enum_ty.members.iter().any(|(m, _)| *m == name) {
                    covered.insert(name);
                } else {
                    return;
                }
            }
        }

        let missing: Vec<&str> = enum_ty
            .members
            .iter()
            .map(|(m, _)| m.as_str())
            .filter(|m| !covered.contains(*m))
            .collect();
        if missing.is_empty() {
            return;
        }
        let list = missing
            .iter()
            .map(|m| format!("`{m}`"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push(make(
            TypeCode::T070,
            node,
            Severity::Error,
            format!(
                "`when` on enum `{}` is not exhaustive; missing {}",
                enum_ty.name, list
            ),
        ));
    }
}

/// The enumerator name(s) named by an `is (...)` state expression. A single
/// `Identifier` names one enumerator; a `BinaryExpression` joined by `or` lists
/// several (`A or B or C`).
fn arm_enumerators(state: Node) -> Vec<String> {
    match state.kind() {
        Kind::Identifier => vec![state.text().to_string()],
        Kind::ParenthesizedExpression => state
            .named_children()
            .into_iter()
            .flat_map(arm_enumerators)
            .collect(),
        Kind::BinaryExpression => {
            // Only `or`-joined identifier lists name enumerators; any other
            // operator yields no recognised enumerators (and thus no coverage).
            let is_or = state.children().iter().any(|c| c.kind() == Kind::Or);
            if !is_or {
                return Vec::new();
            }
            state
                .named_children()
                .into_iter()
                .flat_map(arm_enumerators)
                .collect()
        }
        _ => Vec::new(),
    }
}
