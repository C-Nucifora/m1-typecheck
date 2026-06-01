use m1_typecheck::resolve::Scope;
use m1_typecheck::typer::type_of_source_expr;
use m1_typecheck::types::ValueType;
use std::collections::HashMap;

// Helper: type the single expression in `<expr>;` with the given locals.
fn ty(src_expr: &str, locals: &[(&str, ValueType)]) -> ValueType {
    let mut m = HashMap::new();
    for (k, v) in locals {
        m.insert(k.to_string(), *v);
    }
    let scope = Scope {
        locals: m,
        group: None,
        project: None,
    };
    type_of_source_expr(&format!("{src_expr};\n"), &scope)
}

#[test]
fn literals() {
    assert_eq!(ty("1", &[]), ValueType::Integer);
    assert_eq!(ty("1.0", &[]), ValueType::Float);
    assert_eq!(ty("0u", &[]), ValueType::Unsigned);
    assert_eq!(ty("true", &[]), ValueType::Boolean);
    assert_eq!(ty("\"hi\"", &[]), ValueType::String);
}

#[test]
fn local_and_arithmetic() {
    assert_eq!(
        ty("fGain", &[("fGain", ValueType::Float)]),
        ValueType::Float
    );
    assert_eq!(ty("1 + 2", &[]), ValueType::Integer);
    assert_eq!(
        ty("fGain * 2", &[("fGain", ValueType::Float)]),
        ValueType::Float
    );
}

#[test]
fn comparisons_are_boolean() {
    assert_eq!(ty("1 < 2", &[]), ValueType::Boolean);
    assert_eq!(ty("a eq b", &[]), ValueType::Boolean); // a,b opaque -> still Boolean
}

#[test]
fn calls_resolve_builtin_return_types() {
    // Calculate.Min -> Integer|FloatingPoint, following its argument types (#17).
    assert_eq!(ty("Calculate.Min(1, 2)", &[]), ValueType::Integer);
    assert_eq!(ty("Calculate.Min(1.0, 2.0)", &[]), ValueType::Float);
}

#[test]
fn convert_functions_type_to_their_real_returns() {
    // M1's Convert library has exactly ToInteger and ToUnsignedInteger — and no
    // ToFloat, because int->float widening is implicit. Guard the real names so
    // the phantom `Convert.ToFloat` can't creep back into messages/tests.
    assert_eq!(ty("Convert.ToInteger(1.5)", &[]), ValueType::Integer);
    assert_eq!(
        ty("Convert.ToUnsignedInteger(1.5)", &[]),
        ValueType::Unsigned
    );
    // A non-existent Convert function doesn't resolve, so it types as Unknown.
    assert_eq!(ty("Convert.ToFloat(1)", &[]), ValueType::Unknown);
}
