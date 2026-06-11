use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::typer::path_text;
use m1_core::{Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let Some(project) = scope.project else { return };
        let table = project.symbols();
        // `<EnumTypeName>.<Member>` with a head that names an enum type but a
        // member the enum does not declare.
        //
        // NOTE: the plan also specified an `LHS is (Member)` trigger, but the
        // tree-sitter-m1 grammar treats `is` as a syntax error (and the idiom
        // never appears in the m1-example corpus), so it is unreachable; only the
        // typed-member-path idiom is checked here.
        if node.kind() != Kind::MemberExpression {
            return;
        }
        if matches!(
            node.parent().map(|p| p.kind()),
            Some(Kind::MemberExpression)
        ) {
            return; // only the outermost path
        }
        let path = path_text(*node);
        if let Some((head, member)) = path.rsplit_once('.')
            && let Some(id) = table.enum_by_name(head)
        {
            let enum_name = table.enum_type(id).name.clone();
            let mut member_is_error = false;
            // Skip firmware-supplied (open) enums: their member list is not in
            // the project, so an unlisted name is not provably a non-member.
            if !table.enum_is_open(id) && !table.enum_has_member(id, member) {
                // M1 Build resolves names case-insensitively (manual pp.64-65 —
                // the same behaviour T091 guards), so a case-variant of a real
                // member builds fine and is only a style warning; a name with no
                // member under any casing is M1 Build Error 1352 and fails the
                // build.
                let case_variant = table
                    .enum_type(id)
                    .members
                    .iter()
                    .find(|(m, _)| m.eq_ignore_ascii_case(member));
                let (severity, message) = match case_variant {
                    Some((actual, _)) => (
                        Severity::Warning,
                        format!(
                            "`{member}` does not match the case of enum `{head}` member `{actual}` (M1 Build resolves it case-insensitively)"
                        ),
                    ),
                    None => {
                        member_is_error = true;
                        (
                            Severity::Error,
                            format!(
                                "`{member}` is not a member of enum `{enum_name}` (M1 Build Error 1352: \"does not exist\")"
                            ),
                        )
                    }
                };
                out.push(make(TypeCode::T020, node, severity, message));
            }
            // The head itself resolved as a case-variant of the enum's name
            // (`universal Switch State.On`): M1 Build accepts it, so it gets the
            // same style Warning as a member case-variant. Suppressed when the
            // member is a hard Error — the build-breaker is the headline. (#183)
            if enum_name != head && !member_is_error {
                out.push(make(
                    TypeCode::T020,
                    node,
                    Severity::Warning,
                    format!(
                        "`{head}` does not match the case of enum `{enum_name}` (M1 Build resolves it case-insensitively)"
                    ),
                ));
            }
        }
    }
}
