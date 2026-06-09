//! T084 — expand-bounds. Manual p.33: expand's `[start]` and `[end]` "must be
//! literals or constants, and represent integer values that are 0 or
//! positive." The body is text-substituted at compile time, so a runtime
//! value or a negative bound cannot work.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use crate::typer::path_text;
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::ExpandStatement {
            return;
        }
        for field in [Field::Start, Field::End] {
            let Some(bound) = node.child_by_field(field) else {
                continue;
            };
            check_bound(&bound, scope, out);
        }
    }
}

fn check_bound(bound: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    match bound.kind() {
        Kind::Number => {
            let text = bound.text();
            let t = text.trim().trim_end_matches(['u', 'U']);
            // A float literal is not an integer bound; hex parses via u64.
            let ok = if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                u64::from_str_radix(hex, 16).is_ok()
            } else {
                t.parse::<u64>().is_ok()
            };
            if !ok {
                out.push(make(
                    TypeCode::T084,
                    bound,
                    Severity::Error,
                    format!(
                        "expand bound `{}` must be a non-negative integer literal or constant",
                        text.trim()
                    ),
                ));
            }
        }
        // `-N`: a negative literal bound.
        Kind::UnaryExpression => {
            let negative = bound.children().iter().any(|c| c.kind() == Kind::Minus);
            if negative {
                out.push(make(
                    TypeCode::T084,
                    bound,
                    Severity::Error,
                    format!(
                        "expand bound `{}` must be 0 or positive",
                        bound.text().trim()
                    ),
                ));
            }
        }
        Kind::Identifier | Kind::MemberExpression => {
            // A constant is allowed; a channel/parameter read is a runtime
            // value the compile-time expansion cannot use. Unresolved names
            // stay silent (T001's concern).
            if let Resolution::Symbol(s) = resolve(&path_text(*bound), scope)
                && matches!(s.kind, SymbolKind::Channel | SymbolKind::Parameter)
            {
                out.push(make(
                    TypeCode::T084,
                    bound,
                    Severity::Error,
                    format!(
                        "expand bound `{}` is a runtime value; bounds must be literals or constants",
                        bound.text().trim()
                    ),
                ));
            }
        }
        Kind::ParenthesizedExpression => {
            for c in bound.named_children() {
                if crate::typer::is_expr(c.kind()) {
                    check_bound(&c, scope, out);
                }
            }
        }
        // Anything else (calls, arithmetic) is not a literal/constant bound.
        _ => {
            out.push(make(
                TypeCode::T084,
                bound,
                Severity::Error,
                format!(
                    "expand bound `{}` must be a literal or constant",
                    bound.text().trim()
                ),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::check_script_no_project;

    fn has_t084(src: &str) -> bool {
        check_script_no_project(src)
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "T084")
    }

    #[test]
    fn literal_bounds_are_fine() {
        assert!(!has_t084("expand (I = 0 to 7)\n{\n\ty = 1;\n}\n"));
        assert!(!has_t084("expand (I = 1 to 12)\n{\n\ty = 1;\n}\n"));
    }

    #[test]
    fn negative_bound_flags() {
        assert!(has_t084("expand (I = -1 to 7)\n{\n\ty = 1;\n}\n"));
        assert!(has_t084("expand (I = 0 to -3)\n{\n\ty = 1;\n}\n"));
    }

    #[test]
    fn float_bound_flags() {
        assert!(has_t084("expand (I = 0 to 7.5)\n{\n\ty = 1;\n}\n"));
    }

    #[test]
    fn call_bound_flags() {
        assert!(has_t084(
            "expand (I = 0 to Calculate.Max(1, 2))\n{\n\ty = 1;\n}\n"
        ));
    }

    #[test]
    fn unresolved_name_bound_stays_silent() {
        // Could be a constant — without a project we can't tell.
        assert!(!has_t084("expand (I = 0 to COUNT)\n{\n\ty = 1;\n}\n"));
    }
}
