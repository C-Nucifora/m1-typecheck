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
        let table = project.symbols();
        let enum_open = table.enum_is_open(id);
        let enum_ty = table.enum_type(id);

        // Collect the enumerators covered across every `is` arm (one arm may list
        // several via `or`). A label that is NOT a member of the enum is handled
        // by enum-openness: for an OPEN firmware enum the member list is not fully
        // known, so an unlisted label may be a real enumerator we cannot see —
        // bail conservatively (no T020, no T070). For a CLOSED project enum the
        // member list is exhaustive, M1's `when…is` has no catch-all syntax, and
        // M1 Build rejects an unknown is-label (Error 1352): emit T020 and keep
        // checking exhaustiveness so a typo cannot mask a missing enumerator (#212).
        let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut nonmember_buf: Vec<TypeDiagnostic> = Vec::new();
        for arm in node.named_children() {
            if arm.kind() != Kind::IsClause {
                continue;
            }
            let Some(state) = arm.child_by_field(Field::State) else {
                continue;
            };
            let names = arm_enumerators(state);
            // An arm we cannot decompose into enumerators (an unrecognisable
            // pattern) carries no information either way — bail conservatively.
            if names.is_empty() {
                return;
            }
            for name in names {
                if enum_ty.members.iter().any(|(m, _)| *m == name) {
                    covered.insert(name);
                } else if enum_open {
                    // Open firmware enum: the unlisted label may be a real member
                    // we cannot see, so treat the arm as a catch-all and bail.
                    return;
                } else {
                    nonmember_buf.push(make(
                        TypeCode::T020,
                        &state,
                        Severity::Error,
                        format!(
                            "`{name}` is not a member of enum `{}` (M1 Build Error 1352: \"does not exist\")",
                            enum_ty.name
                        ),
                    ));
                }
            }
        }
        out.append(&mut nonmember_buf);

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
/// `Identifier` names one enumerator; an `IsPatternList` (grammar ≥0.5.0,
/// `A or B or C`) lists several. Returns empty when any pattern is
/// unrecognisable — the caller then treats the arm as a catch-all and stays
/// silent rather than risk a false "not exhaustive".
fn arm_enumerators(state: Node) -> Vec<String> {
    match state.kind() {
        Kind::Identifier => vec![state.text().to_string()],
        // `State.A`: the enumerator is the leaf segment.
        Kind::MemberExpression => state
            .named_children()
            .into_iter()
            .rfind(|c| c.kind() == Kind::Identifier)
            .map(|leaf| vec![leaf.text().to_string()])
            .unwrap_or_default(),
        Kind::IsPatternList => {
            let mut names = Vec::new();
            for pattern in state.named_children() {
                let got = arm_enumerators(pattern);
                if got.is_empty() {
                    return Vec::new(); // unrecognisable pattern → whole arm bails
                }
                names.extend(got);
            }
            names
        }
        Kind::ParenthesizedExpression => state
            .named_children()
            .into_iter()
            .flat_map(arm_enumerators)
            .collect(),
        // Pre-0.5.0 grammars parsed `A or B` as a binary or-expression; keep
        // accepting that shape for one transition release.
        Kind::BinaryExpression => {
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
