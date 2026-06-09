//! T064 `wrong-argument-count`: a call to a **fully modelled** library method
//! whose argument count matches **none** of that method's overload arities.
//!
//! The intrinsics model arities precisely, and a method's overloads can declare
//! different arities (e.g. `Change.By` = {2, 3}), so the check is union-aware: the
//! call is fine if the actual arg count equals *any* overload's parameter count.
//!
//! Scope guard: this only fires when the callee resolves to `BuiltinFn` (a known
//! library object + known method). It deliberately does **not** flag unknown
//! members — the corpus calls valid firmware functions absent from the intrinsics
//! export (`System.FlashFree()`), which `resolve` returns as `Opaque`, never
//! `BuiltinFn`, so they are never reached here. Opt-in via `--select T064` (or the
//! `[diagnostics] select`); low severity (Warning) by default when selected.
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
        // Fully modelled call only: resolved to a library method's overload set.
        let Resolution::BuiltinFn(overloads) = resolve(&path, scope) else {
            return;
        };
        if overloads.is_empty() {
            return;
        }

        let arg_count = node
            .children()
            .into_iter()
            .find(|c| c.kind() == Kind::ArgumentList)
            .map(|al| {
                al.named_children()
                    .into_iter()
                    .filter(|c| crate::typer::is_expr(c.kind()))
                    .count()
            })
            .unwrap_or(0);

        // Union-aware: fine if the arg count matches ANY overload's arity.
        if overloads.iter().any(|o| o.params.len() == arg_count) {
            return;
        }

        let mut arities: Vec<usize> = overloads.iter().map(|o| o.params.len()).collect();
        arities.sort_unstable();
        arities.dedup();
        let expected = arities
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(" or ");
        out.push(make(
            TypeCode::T064,
            node,
            Severity::Warning,
            format!("`{path}` called with {arg_count} argument(s); expected {expected}"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use crate::diagnostics::TypeCode;
    use crate::rules::{Registry, run_with};

    fn codes(src: &str) -> Vec<TypeCode> {
        run_with(
            &Registry::all(),
            None,
            None,
            src,
            &crate::cross_script::ChannelTaints::default(),
        )
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
    }

    #[test]
    fn opt_in_only_off_by_default() {
        use crate::rules::check_script_no_project;
        // Default run (no opt-in) must NOT emit T064 even for a wrong-arity call.
        let got: Vec<TypeCode> = check_script_no_project("local x = Calculate.Max(1, 2, 3);\n")
            .diagnostics
            .into_iter()
            .map(|d| d.code)
            .collect();
        assert!(!got.contains(&TypeCode::T064));
    }

    #[test]
    fn wrong_arity_is_flagged() {
        // Every Calculate.Max overload declares 2 params.
        assert!(codes("local x = Calculate.Max(1, 2, 3, 4, 5);\n").contains(&TypeCode::T064));
        assert!(codes("local x = Calculate.Max(1);\n").contains(&TypeCode::T064));
    }

    #[test]
    fn valid_arity_is_not_flagged() {
        assert!(!codes("local x = Calculate.Max(1, 2);\n").contains(&TypeCode::T064));
    }

    #[test]
    fn union_arity_accepts_each_overload() {
        // Change.By has overloads of arity {2, 3}; both are valid.
        assert!(!codes("Change.By(a, b);\n").contains(&TypeCode::T064));
        assert!(!codes("Change.By(a, b, c);\n").contains(&TypeCode::T064));
        // ...but 4 args matches neither.
        assert!(codes("Change.By(a, b, c, d);\n").contains(&TypeCode::T064));
    }

    #[test]
    fn unknown_member_is_not_flagged() {
        // System.FlashFree is valid firmware absent from the intrinsics export:
        // it resolves Opaque, never BuiltinFn, so T064 must not fire.
        assert!(!codes("x = System.FlashFree();\n").contains(&TypeCode::T064));
    }
}
