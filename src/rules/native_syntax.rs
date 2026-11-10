//! Small CST helpers shared by the rules that mirror M1 Build's own script
//! validation (T118–T130).
use m1_core::{Field, Kind, Node};

/// Operator family of a binary expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    And,
    Or,
    Equality,
    Relational,
    Other,
}

pub(crate) fn binary_op(node: &Node) -> Option<BinaryOp> {
    if node.kind() != Kind::BinaryExpression {
        return None;
    }
    let op = node.child_by_field(Field::Operator)?;
    Some(match op.kind() {
        Kind::And | Kind::AmpAmp => BinaryOp::And,
        Kind::Or | Kind::PipePipe => BinaryOp::Or,
        Kind::EqEq | Kind::BangEq | Kind::Eq | Kind::Neq => BinaryOp::Equality,
        Kind::Lt | Kind::Gt | Kind::LtEq | Kind::GtEq => BinaryOp::Relational,
        _ => BinaryOp::Other,
    })
}

/// The operator token text of a binary expression (`and`, `>`, …).
pub(crate) fn operator_text<'a>(node: &Node<'a>) -> &'a str {
    node.child_by_field(Field::Operator)
        .map(|op| op.text())
        .unwrap_or("")
}

pub(crate) fn operands<'a>(node: &Node<'a>) -> Option<(Node<'a>, Node<'a>)> {
    Some((
        node.child_by_field(Field::Left)?,
        node.child_by_field(Field::Right)?,
    ))
}

/// The expression inside any number of parentheses.
pub(crate) fn unparenthesized(node: Node<'_>) -> Node<'_> {
    let mut node = node;
    while node.kind() == Kind::ParenthesizedExpression {
        match node
            .named_children()
            .into_iter()
            .find(|child| crate::typer::is_expr(child.kind()))
        {
            Some(inner) => node = inner,
            None => break,
        }
    }
    node
}

/// A numeric or Boolean literal, possibly negated or parenthesized.
pub(crate) fn is_literal(node: Node<'_>) -> bool {
    let node = unparenthesized(node);
    match node.kind() {
        Kind::Number | Kind::Boolean => !node.text().contains("$("),
        Kind::UnaryExpression => {
            node.children().iter().any(|c| c.kind() == Kind::Minus)
                && node
                    .named_children()
                    .into_iter()
                    .find(|c| crate::typer::is_expr(c.kind()))
                    .is_some_and(|operand| unparenthesized(operand).kind() == Kind::Number)
        }
        _ => false,
    }
}

/// True for a `not`/`!` unary expression.
pub(crate) fn is_logical_not(node: &Node) -> bool {
    node.kind() == Kind::UnaryExpression
        && node
            .children()
            .iter()
            .any(|c| matches!(c.kind(), Kind::Not | Kind::Bang))
}

/// The single operand of a unary expression.
pub(crate) fn unary_operand<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    node.named_children()
        .into_iter()
        .find(|c| crate::typer::is_expr(c.kind()))
}

/// True when `node` is the target of a plain `=` assignment.
pub(crate) fn is_plain_assignment_target(node: &Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    parent.kind() == Kind::AssignmentStatement
        && parent
            .child_by_field(Field::Target)
            .is_some_and(|target| target.byte_range() == node.byte_range())
        && parent
            .child_by_field(Field::Operator)
            .is_some_and(|op| op.kind() == Kind::Assign)
}

/// True for the outermost node of an identifier or member path.
pub(crate) fn is_outermost_path(node: &Node) -> bool {
    matches!(node.kind(), Kind::Identifier | Kind::MemberExpression)
        && node
            .parent()
            .is_none_or(|p| p.kind() != Kind::MemberExpression && p.kind() != Kind::TypeAnnotation)
}
