//! Best-effort expression typing over the m1-core CST.
use crate::resolve::{Resolution, Scope, resolve};
use crate::types::{ValueType, numeric_join, type_of_number_literal};
use m1_core::{Kind, Node};

/// Type the first expression found in a parsed source string. Test/utility entry.
pub fn type_of_source_expr(src: &str, scope: &Scope) -> ValueType {
    let cst = m1_core::parse(src);
    // Find the first expression-bearing statement and type its expression child.
    fn first_expr(n: Node<'_>) -> Option<Node<'_>> {
        if is_expr(n.kind()) {
            return Some(n);
        }
        for c in n.children() {
            if let Some(e) = first_expr(c) {
                return Some(e);
            }
        }
        None
    }
    match first_expr(cst.root()) {
        Some(e) => type_of(e, scope),
        None => ValueType::Unknown,
    }
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

pub fn type_of(node: Node, scope: &Scope) -> ValueType {
    match node.kind() {
        Kind::Number => type_of_number_literal(node.text()),
        Kind::Boolean => ValueType::Boolean,
        Kind::String => ValueType::String,
        Kind::ParenthesizedExpression => child_expr(node)
            .map(|c| type_of(c, scope))
            .unwrap_or(ValueType::Unknown),
        Kind::Identifier | Kind::MemberExpression => {
            let path = path_text(node);
            // Typed enum-member path: `<EnumTypeName>.<Member>` -> Enum(id).
            if let Some(p) = scope.project
                && let Some((head, member)) = path.rsplit_once('.')
                && let Some(id) = p.symbols().enum_by_name(head)
                && p.symbols().enum_has_member(id, member)
            {
                return ValueType::Enum(id);
            }
            match resolve(&path, scope) {
                Resolution::Local(t) => t,
                Resolution::Symbol(s) => s.value_type,
                // A library object or a function reference used as a bare value
                // has no value type; a call's return type is handled at
                // CallExpression (v1: still Unknown).
                Resolution::BuiltinObject(_) | Resolution::BuiltinFn(_) => ValueType::Unknown,
                Resolution::Opaque | Resolution::Unresolved => ValueType::Unknown,
            }
        }
        Kind::CallExpression => type_of_call(node, scope),
        Kind::UnaryExpression => {
            // `not`/`!` -> Boolean; `-x` -> type of x.
            let op_is_not = node
                .children()
                .iter()
                .any(|c| matches!(c.kind(), Kind::Not | Kind::Bang));
            if op_is_not {
                ValueType::Boolean
            } else {
                child_expr(node)
                    .map(|c| type_of(c, scope))
                    .unwrap_or(ValueType::Unknown)
            }
        }
        Kind::TernaryExpression => {
            let exprs: Vec<_> = node
                .named_children()
                .into_iter()
                .filter(|c| is_expr(c.kind()))
                .collect();
            // condition, consequence, alternative -> join the two branches
            match (exprs.get(1), exprs.get(2)) {
                (Some(a), Some(b)) => {
                    let (ta, tb) = (type_of(*a, scope), type_of(*b, scope));
                    if ta == tb { ta } else { numeric_join(ta, tb) }
                }
                _ => ValueType::Unknown,
            }
        }
        Kind::BinaryExpression => type_of_binary(node, scope),
        _ => ValueType::Unknown,
    }
}

/// Type a call from the declared `returns` of its resolved built-in overload(s).
/// User-function calls and unresolved callees stay `Unknown` (no signature in the
/// symbol model yet). When overloads disagree on the mapped type we return
/// `Unknown` rather than guess.
fn type_of_call(node: Node, scope: &Scope) -> ValueType {
    let Some(callee) = node
        .named_children()
        .into_iter()
        .find(|c| matches!(c.kind(), Kind::Identifier | Kind::MemberExpression))
    else {
        return ValueType::Unknown;
    };
    match resolve(&path_text(callee), scope) {
        Resolution::BuiltinFn(overloads) => {
            let arg_types = call_arg_types(node, scope);
            let mut result: Option<ValueType> = None;
            for ov in &overloads {
                let t = returns_to_type(&ov.returns, &arg_types);
                match result {
                    None => result = Some(t),
                    Some(prev) if prev == t => {}
                    Some(_) => return ValueType::Unknown,
                }
            }
            result.unwrap_or(ValueType::Unknown)
        }
        _ => ValueType::Unknown,
    }
}

/// Types of a call's positional argument expressions, in order.
fn call_arg_types(node: Node, scope: &Scope) -> Vec<ValueType> {
    node.children()
        .into_iter()
        .find(|c| c.kind() == Kind::ArgumentList)
        .map(|al| {
            al.named_children()
                .into_iter()
                .filter(|c| is_expr(c.kind()))
                .map(|c| type_of(c, scope))
                .collect()
        })
        .unwrap_or_default()
}

/// Map an intrinsic overload's declared `returns` to a [`ValueType`]. The
/// `Integer|FloatingPoint` union (Min/Max/Abs/…) follows the argument types.
fn returns_to_type(returns: &str, arg_types: &[ValueType]) -> ValueType {
    match returns {
        "FloatingPoint" | "FixedPoint7dps" => ValueType::Float,
        "Integer" => ValueType::Integer,
        "UnsignedInteger" => ValueType::Unsigned,
        "Boolean" => ValueType::Boolean,
        "String" => ValueType::String,
        "Integer|FloatingPoint" => arg_types
            .iter()
            .copied()
            .filter(|t| {
                matches!(
                    t,
                    ValueType::Integer | ValueType::Unsigned | ValueType::Float
                )
            })
            .reduce(numeric_join)
            .unwrap_or(ValueType::Unknown),
        // Void, Handle, Enumeration (unknown which enum), Bit: not usefully typed.
        _ => ValueType::Unknown,
    }
}

fn type_of_binary(node: Node, scope: &Scope) -> ValueType {
    let operands: Vec<_> = node
        .named_children()
        .into_iter()
        .filter(|c| is_expr(c.kind()))
        .collect();
    let (l, r) = match (operands.first(), operands.get(1)) {
        (Some(l), Some(r)) => (type_of(*l, scope), type_of(*r, scope)),
        _ => return ValueType::Unknown,
    };
    match binary_op_kind(node) {
        OpClass::Comparison | OpClass::Logical => ValueType::Boolean,
        OpClass::Arithmetic => numeric_join(l, r),
        OpClass::Bitwise => l, // result follows the (integer) left operand
        OpClass::Unknown => ValueType::Unknown,
    }
}

enum OpClass {
    Arithmetic,
    Comparison,
    Logical,
    Bitwise,
    Unknown,
}

fn binary_op_kind(node: Node) -> OpClass {
    for c in node.children() {
        match c.kind() {
            Kind::Plus | Kind::Minus | Kind::Star | Kind::Slash | Kind::Percent => {
                return OpClass::Arithmetic;
            }
            Kind::EqEq
            | Kind::BangEq
            | Kind::Eq
            | Kind::Neq
            | Kind::Lt
            | Kind::Gt
            | Kind::LtEq
            | Kind::GtEq => return OpClass::Comparison,
            Kind::And | Kind::Or | Kind::AmpAmp | Kind::PipePipe => return OpClass::Logical,
            Kind::Amp | Kind::Pipe | Kind::Caret | Kind::LtLt | Kind::GtGt => {
                return OpClass::Bitwise;
            }
            _ => {}
        }
    }
    OpClass::Unknown
}

fn child_expr(node: Node) -> Option<Node> {
    node.named_children()
        .into_iter()
        .find(|c| is_expr(c.kind()))
}

/// The dotted source text of an identifier / member-expression path.
pub fn path_text(node: Node) -> String {
    node.text().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn type_of_first_call(src: &str) -> ValueType {
        let cst = m1_core::parse(src);
        fn find_call(n: Node) -> Option<Node> {
            if n.kind() == Kind::CallExpression {
                return Some(n);
            }
            n.children().into_iter().find_map(find_call)
        }
        let scope = Scope {
            locals: HashMap::new(),
            group: None,
            project: None,
        };
        type_of(find_call(cst.root()).expect("a call expression"), &scope)
    }

    #[test]
    fn builtin_return_types_map_to_value_types() {
        // Calculate.Bias -> FloatingPoint; Calculate.Between -> Boolean;
        // CanComms.GetID -> Integer; CanComms.GetTicks -> UnsignedInteger.
        assert_eq!(
            type_of_first_call("local x = Calculate.Bias(1.0, 2.0, 3.0);\n"),
            ValueType::Float
        );
        assert_eq!(
            type_of_first_call("local x = Calculate.Between(1.0, 0.0, 2.0);\n"),
            ValueType::Boolean
        );
        assert_eq!(
            type_of_first_call("local x = CanComms.GetID(h);\n"),
            ValueType::Integer
        );
        assert_eq!(
            type_of_first_call("local x = CanComms.GetTicks();\n"),
            ValueType::Unsigned
        );
    }

    #[test]
    fn numeric_union_return_follows_argument_types() {
        // Calculate.Absolute -> Integer|FloatingPoint: result follows the argument.
        assert_eq!(
            type_of_first_call("local x = Calculate.Absolute(1.0);\n"),
            ValueType::Float
        );
        assert_eq!(
            type_of_first_call("local x = Calculate.Absolute(2);\n"),
            ValueType::Integer
        );
    }

    #[test]
    fn unknown_callee_is_unknown() {
        // A user/unresolved function has no signature in the model yet.
        assert_eq!(
            type_of_first_call("local x = SomeGroup.UserFunc(1);\n"),
            ValueType::Unknown
        );
    }
}
