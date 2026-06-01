//! T060 `stateful-conditional`: a stateful (purple-icon) library function must
//! run on every execution. It may not be called conditionally (inside an
//! `if`/`when` branch) nor inside a comparison/logical expression, where
//! short-circuiting could skip it. M1 Build itself rejects these.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::typer::path_text;
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::CallExpression {
            return;
        }
        let Some(callee) = node.child_by_field(Field::Function) else {
            return;
        };
        let path = path_text(callee);
        let Resolution::BuiltinFn(overloads) = resolve(&path, scope) else {
            return;
        };
        if !overloads.iter().any(|o| o.stateful) {
            return;
        }
        if let Some(ctx) = conditional_context(node) {
            out.push(make(
                TypeCode::T060,
                node,
                Severity::Error,
                format!(
                    "stateful function `{path}` must be called on every execution; \
                     it cannot be used {ctx}"
                ),
            ));
        }
    }
}

/// If `call` sits inside a construct that can skip or short-circuit it, return a
/// human description of that construct; otherwise `None` (called unconditionally).
fn conditional_context(call: &Node) -> Option<&'static str> {
    let mut cur = call.parent();
    while let Some(n) = cur {
        match n.kind() {
            Kind::IfStatement | Kind::ElseClause => return Some("inside an `if`/`else` branch"),
            // An `is` clause is a conditional arm (only the matching state runs),
            // so a stateful call there is conditional. The enclosing
            // `WhenStatement` itself is NOT: a top-level `when` runs on every
            // tick (it's the state-machine main) and its subject expression is
            // evaluated unconditionally. Keep walking past a `WhenStatement` —
            // if it is a *nested* when, the surrounding `is`/`if` ancestor is
            // caught on the way up; if top-level, the call is unconditional (#21).
            Kind::IsClause => return Some("inside a `when`/`is` branch"),
            Kind::TernaryExpression => return Some("inside a `?:` expression"),
            Kind::BinaryExpression if is_comparison_or_logical(&n) => {
                return Some("inside a comparison/logical expression");
            }
            Kind::UnaryExpression
                if n.children()
                    .iter()
                    .any(|c| matches!(c.kind(), Kind::Not | Kind::Bang)) =>
            {
                return Some("inside a `not` expression");
            }
            Kind::SourceFile => return None,
            _ => {}
        }
        cur = n.parent();
    }
    None
}

fn is_comparison_or_logical(node: &Node) -> bool {
    node.children().iter().any(|c| {
        matches!(
            c.kind(),
            Kind::EqEq
                | Kind::BangEq
                | Kind::Eq
                | Kind::Neq
                | Kind::Lt
                | Kind::Gt
                | Kind::LtEq
                | Kind::GtEq
                | Kind::And
                | Kind::Or
                | Kind::AmpAmp
                | Kind::PipePipe
        )
    })
}

#[cfg(test)]
mod tests {
    use crate::diagnostics::TypeCode;
    use crate::rules::check_script_no_project;

    fn codes(src: &str) -> Vec<TypeCode> {
        check_script_no_project(src)
            .diagnostics
            .into_iter()
            .map(|d| d.code)
            .collect()
    }

    #[test]
    fn top_level_stateful_call_is_ok() {
        // The M1 idiom: call the stateful fn unconditionally into a local, then
        // branch on the local.
        let src = "local x = Delay.Rising(a > 1.0, 5.0);\n";
        assert!(!codes(src).contains(&TypeCode::T060));
    }

    #[test]
    fn stateful_call_inside_if_is_flagged() {
        let src = "if (ready) { x = Delay.Rising(a, 5.0); }\n";
        assert!(codes(src).contains(&TypeCode::T060));
    }

    #[test]
    fn stateful_call_inside_comparison_is_flagged() {
        let src = "x = Delay.Rising(a, 5.0) and ready;\n";
        assert!(codes(src).contains(&TypeCode::T060));
    }

    #[test]
    fn non_stateful_call_inside_if_is_ok() {
        let src = "if (ready) { x = Calculate.Max(a, b); }\n";
        assert!(!codes(src).contains(&TypeCode::T060));
    }

    #[test]
    fn stateful_call_in_when_subject_is_ok() {
        // The `when` subject is evaluated on every tick (the state machine is the
        // unconditional main), so a stateful call there is not conditional (#21).
        let src = "when (Delay.Rising(go, 5.0)) {\n    is (true) {\n        x = 1;\n    }\n}\n";
        assert!(!codes(src).contains(&TypeCode::T060));
    }

    #[test]
    fn stateful_call_inside_is_clause_is_flagged() {
        // An `is` clause only runs when its state matches → conditional.
        let src = "when (state) {\n    is (true) {\n        x = Delay.Rising(go, 5.0);\n    }\n}\n";
        assert!(codes(src).contains(&TypeCode::T060));
    }
}
