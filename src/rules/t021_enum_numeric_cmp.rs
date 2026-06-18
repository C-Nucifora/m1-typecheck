use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::typer::{is_expr, type_of};
use crate::types::ValueType;
use m1_core::{Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::BinaryExpression {
            return;
        }
        let is_cmp = node.children().iter().any(|c| {
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
            )
        });
        if !is_cmp {
            return;
        }
        let ops: Vec<_> = node
            .named_children()
            .into_iter()
            .filter(|c| is_expr(c.kind()))
            .collect();
        let (Some(l), Some(r)) = (ops.first(), ops.get(1)) else {
            return;
        };
        let (lt, rt) = (type_of(*l, scope), type_of(*r, scope));
        let enum_vs_num = (lt.is_enum() && is_numeric(rt)) || (rt.is_enum() && is_numeric(lt));
        if enum_vs_num {
            // M1 Build rejects the comparison outright (Error 1329
            // "Incompatible data types for binary operation"), so this is an
            // error: the build fails, not a style nit (#173).
            out.push(make(
                TypeCode::T021,
                node,
                Severity::Error,
                "comparing an enum to a number; compare against an enum member instead (M1 Build Error 1329: incompatible data types)".into(),
            ));
            return;
        }
        // The Manual's "Enumeration Comparison" table (p.36-37) defines ONLY
        // `eq`/`neq` (==/!=) for enumerated types. Ordering (`<`/`>`/`<=`/`>=`)
        // is not defined, so two enum operands joined by an ordering operator
        // are rejected by M1 Build with the same Error 1329. Guard on BOTH
        // sides being known enums so an Unknown operand never trips it.
        let is_ordering = node
            .children()
            .iter()
            .any(|c| matches!(c.kind(), Kind::Lt | Kind::Gt | Kind::LtEq | Kind::GtEq));
        if is_ordering && lt.is_enum() && rt.is_enum() {
            out.push(make(
                TypeCode::T021,
                node,
                Severity::Error,
                "enumerated types support only eq/neq (==/!=), not ordering comparisons (M1 Build Error 1329: incompatible data types for binary operation)".into(),
            ));
            return;
        }
        // Two operands of DIFFERENT enum types (even with eq/neq): M1 Build
        // rejects comparing distinct enumerated types outright (Error 1329).
        // Only same-enum comparison is valid — the same `(Enum(a), Enum(b)) =>
        // a == b` constraint already encoded for assignment (T030) and user
        // arguments (T085). Stay silent unless BOTH ids are known *and* the
        // enums are closed (project) types: an open firmware enum's identity is
        // a placeholder we cannot prove distinct, so comparing against it must
        // not false-positive (matches the ordering guard's conservatism).
        if let (ValueType::Enum(a), ValueType::Enum(b)) = (lt, rt)
            && a != b
            && let Some(project) = scope.project
        {
            let syms = project.symbols();
            if !syms.enum_is_open(a) && !syms.enum_is_open(b) {
                let an = syms.enum_type(a).name.clone();
                let bn = syms.enum_type(b).name.clone();
                out.push(make(
                    TypeCode::T021,
                    node,
                    Severity::Error,
                    format!(
                        "comparing enum `{an}` with enum `{bn}`; only same-enum comparison is valid (M1 Build Error 1329: incompatible data types for binary operation)"
                    ),
                ));
            }
        }
    }
}

fn is_numeric(t: ValueType) -> bool {
    matches!(
        t,
        ValueType::Integer | ValueType::Unsigned | ValueType::Float
    )
}
