//! T083 — static-local-initialiser. Manual p.34: "The initial value of a
//! static local variable must be a literal value, enumerator or constant
//! value." A runtime read (`static local x = Some.Channel;`) or a call as the
//! initialiser only executes on the very first pass, which is almost never
//! what the author intended — M1 Build rejects it.
//!
//! Conservative classification: literals (including negated ones) and
//! constant-foldable literal arithmetic are fine; identifier/member paths are
//! fine unless they resolve to a Channel/Parameter (a runtime read); calls
//! are never constant. Unresolvable names stay silent (T001's concern).
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use crate::typer::{is_expr, path_text};
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::LocalDeclaration {
            return;
        }
        let is_static = node.children().iter().any(|c| c.kind() == Kind::Static);
        if !is_static {
            return;
        }
        let Some(init) = node.child_by_field(Field::Value) else {
            return; // bare `static local x;` — nothing to validate here
        };
        if !is_constant_expr(&init, scope) {
            out.push(make(
                TypeCode::T083,
                node,
                Severity::Error,
                "a static local's initial value must be a literal, enumerator or constant \
                 (it is evaluated once, not every pass)"
                    .into(),
            ));
        }
    }
}

fn is_constant_expr(n: &Node, scope: &Scope) -> bool {
    match n.kind() {
        Kind::Number | Kind::Boolean | Kind::String => true,
        Kind::ParenthesizedExpression | Kind::UnaryExpression => n
            .named_children()
            .into_iter()
            .filter(|c| is_expr(c.kind()))
            .all(|c| is_constant_expr(&c, scope)),
        // Constant-foldable literal arithmetic (`-1`, `2 * 60`) is a constant
        // value in the manual's sense.
        Kind::BinaryExpression => n
            .named_children()
            .into_iter()
            .filter(|c| is_expr(c.kind()))
            .all(|c| is_constant_expr(&c, scope)),
        Kind::Identifier | Kind::MemberExpression => {
            match resolve(&path_text(*n), scope) {
                // The runtime-read case the manual forbids.
                Resolution::Symbol(s) => {
                    !matches!(s.kind, SymbolKind::Channel | SymbolKind::Parameter)
                }
                Resolution::Local(_) => false, // another local: also runtime state
                // Enum members, constants, opaque/unresolved names: stay silent
                // (unresolved is T001's concern, not T083's).
                _ => true,
            }
        }
        Kind::CallExpression => false,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::check_script_no_project;

    fn has_t083(src: &str) -> bool {
        check_script_no_project(src)
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "T083")
    }

    #[test]
    fn literal_initialisers_are_fine() {
        assert!(!has_t083("static local x = 5;\n"));
        assert!(!has_t083("static local ok = false;\n"));
        assert!(!has_t083("static local <Integer> i = -1;\n")); // real corpus shape
        assert!(!has_t083("static local h = 0x00;\n"));
        assert!(!has_t083("static local period = 2 * 60;\n")); // constant-foldable
    }

    #[test]
    fn local_read_initialiser_flags() {
        assert!(has_t083(
            "local seed = 1;\nstatic local x = seed;\n" // another local = runtime state
        ));
    }

    #[test]
    fn call_initialiser_flags() {
        assert!(has_t083("static local x = Calculate.Max(1, 2);\n"));
    }

    #[test]
    fn non_static_local_is_not_checked() {
        assert!(!has_t083("local x = Calculate.Max(1, 2);\n"));
    }

    #[test]
    fn unresolved_identifier_stays_silent() {
        // No project loaded: a bare name can't be classified — T001 territory.
        assert!(!has_t083("static local x = Maybe Constant;\n"));
    }
}
