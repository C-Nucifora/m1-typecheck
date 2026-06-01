use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::typer::type_of;
use crate::types::ValueType;
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        match node.kind() {
            Kind::BinaryExpression => self.check_binary(node, scope, out),
            Kind::AssignmentStatement => self.check_compound_assign(node, scope, out),
            _ => {}
        }
    }
}

impl Rule {
    fn check_binary(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let is_arith = node.children().iter().any(|c| {
            matches!(
                c.kind(),
                Kind::Plus | Kind::Minus | Kind::Star | Kind::Slash | Kind::Percent
            )
        });
        if !is_arith {
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
        if mixes_int_and_float(type_of(*l, scope), type_of(*r, scope)) {
            push_t003(node, out);
        }
    }

    /// Compound assignments (`+=`, `-=`, `*=`, `/=`) are arithmetic too: the
    /// target and value operands must not mix integer and float (#22). Plain
    /// `=` is handled by T030 (assignment type mismatch), not here.
    fn check_compound_assign(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let Some(op) = node.child_by_field(Field::Operator) else {
            return;
        };
        if !matches!(
            op.kind(),
            Kind::PlusEq | Kind::MinusEq | Kind::StarEq | Kind::SlashEq
        ) {
            return;
        }
        let (Some(target), Some(value)) = (
            node.child_by_field(Field::Target),
            node.child_by_field(Field::Value),
        ) else {
            return;
        };
        if mixes_int_and_float(type_of(target, scope), type_of(value, scope)) {
            push_t003(node, out);
        }
    }
}

fn mixes_int_and_float(lt: ValueType, rt: ValueType) -> bool {
    (lt == ValueType::Float && rt.is_integral()) || (rt == ValueType::Float && lt.is_integral())
}

fn push_t003(node: &Node, out: &mut Vec<TypeDiagnostic>) {
    out.push(make(
        TypeCode::T003,
        node,
        Severity::Warning,
        "mixing integer and float in arithmetic; use Convert.ToFloat/ToInteger".into(),
    ));
}

fn is_expr(k: Kind) -> bool {
    matches!(
        k,
        Kind::Identifier
            | Kind::MemberExpression
            | Kind::CallExpression
            | Kind::UnaryExpression
            | Kind::BinaryExpression
            | Kind::TernaryExpression
            | Kind::ParenthesizedExpression
            | Kind::Number
            | Kind::Boolean
            | Kind::String
    )
}
