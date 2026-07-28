//! T065 `intrinsic-argument-type-mismatch`: a fully modelled library call for
//! which no same-arity overload accepts the known argument types.
//!
//! M1 Build's numeric library overloads are stricter than ordinary arithmetic.
//! In particular, repeated `Integer|FloatingPoint` parameters describe one
//! shared choice: `Calculate.Max` is `(Integer, Integer)` OR
//! `(Floating Point, Floating Point)`, not the Cartesian product. A mixed call
//! is M1 Build Error 1302. Unknown arguments stay viable, preserving the
//! checker's false-positive boundary for incomplete project/config models.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::typer::{is_expr, path_text, type_of};
use crate::types::ValueType;
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

        let args: Vec<ValueType> = node
            .children()
            .into_iter()
            .find(|c| c.kind() == Kind::ArgumentList)
            .map(|list| {
                list.named_children()
                    .into_iter()
                    .filter(|c| is_expr(c.kind()))
                    .map(|c| type_of(c, scope))
                    .collect()
            })
            .unwrap_or_default();

        // Arity belongs to T064. If no overload has this arity, do not relabel
        // it as an argument-type error (especially while T064 remains opt-in).
        let same_arity: Vec<_> = overloads
            .into_iter()
            .filter(|ov| ov.params.len() == args.len())
            .collect();
        if same_arity.is_empty()
            || same_arity
                .iter()
                .any(|ov| crate::intrinsics::overload_accepts_args(ov, &args))
        {
            return;
        }

        out.push(make(
            TypeCode::T065,
            node,
            Severity::Error,
            format!(
                "`{path}` has no overload matching argument types ({}){}",
                args.iter()
                    .map(|&ty| render_type(ty))
                    .collect::<Vec<_>>()
                    .join(", "),
                union_explanation(&same_arity),
            ),
        ));
    }
}

fn render_type(ty: ValueType) -> &'static str {
    match ty {
        ValueType::Boolean => "Boolean",
        ValueType::Integer => "Integer",
        ValueType::Unsigned => "Unsigned Integer",
        ValueType::Float => "Floating Point",
        ValueType::Enum(_) => "Enumeration",
        ValueType::String => "String",
        ValueType::Unknown => "Unknown",
    }
}

fn union_explanation(overloads: &[&crate::intrinsics::Overload]) -> &'static str {
    if overloads.iter().any(|ov| {
        ov.params
            .iter()
            .filter(|p| p.ty == "Integer|FloatingPoint")
            .count()
            > 1
    }) {
        "; Integer|FloatingPoint parameters in one signature must all select Integer or all select Floating Point"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::check_script_no_project;

    fn diagnostics(src: &str) -> Vec<crate::diagnostics::TypeDiagnostic> {
        check_script_no_project(src).diagnostics
    }

    fn has_t065(src: &str) -> bool {
        diagnostics(src).iter().any(|d| d.code == TypeCode::T065)
    }

    #[test]
    fn mixed_calculate_max_numeric_families_are_rejected() {
        let src = "local <Unsigned Integer> polePairs = 10;\n\
                   local x = Calculate.Max(1.0, polePairs);\n";
        let d = diagnostics(src)
            .into_iter()
            .find(|d| d.code == TypeCode::T065)
            .expect("mixed Max arguments should be T065");
        assert_eq!(d.inner.severity, Severity::Error);
        assert!(d.inner.message.contains("Floating Point, Unsigned Integer"));
        assert!(d.inner.message.contains("all select Integer"));
    }

    #[test]
    fn same_family_calculate_max_calls_are_valid() {
        assert!(!has_t065(
            "local <Unsigned Integer> n = 10;\nlocal x = Calculate.Max(1, n);\n"
        ));
        assert!(!has_t065(
            "local f = 10.0;\nlocal x = Calculate.Max(1.0, f);\n"
        ));
    }

    #[test]
    fn single_union_and_fixed_float_parameters_are_independent() {
        assert!(!has_t065(
            "local <Unsigned Integer> n = 10;\n\
             local i = Convert.ToInteger(n);\n\
             local ok = Calculate.Stable(n, 0.1);\n"
        ));
    }

    #[test]
    fn concrete_parameter_types_are_checked_conservatively() {
        assert!(has_t065("local x = Calculate.Floor(true);\n"));
        // Integer-to-floating widening is permitted for function arguments.
        assert!(!has_t065("local x = Calculate.Floor(1);\n"));
    }

    #[test]
    fn unknown_argument_keeps_an_overload_viable() {
        assert!(!has_t065("local x = Calculate.Max(1.0, Firmware.Value);\n"));
    }

    #[test]
    fn wrong_arity_is_left_to_t064() {
        assert!(!has_t065("local x = Calculate.Max(1.0);\n"));
    }
}
