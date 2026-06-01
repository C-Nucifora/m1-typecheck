use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::rules::check_script_no_project;

fn codes(src: &str) -> Vec<TypeCode> {
    check_script_no_project(src)
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn t002_flags_float_literal_equality() {
    // fValue is Float (Hungarian); compared with == to a float literal
    assert!(codes("local fValue = 1.0;\nlocal bX = fValue == 1.0;\n").contains(&TypeCode::T002));
}

#[test]
fn t002_no_false_positive_on_integers() {
    assert!(!codes("local iA = 1;\nlocal bX = iA == 2;\n").contains(&TypeCode::T002));
}

#[test]
fn t003_allows_int_float_widening_in_arithmetic() {
    // M1 Build implicitly widens the integer to float; this is not flagged (#17).
    assert!(
        !codes("local fX = 1.0;\nlocal iY = 2;\nlocal fZ = fX + iY;\n").contains(&TypeCode::T003)
    );
}

#[test]
fn t003_no_flag_when_converted() {
    // Convert.ToFloat(iY) returns FloatingPoint -> float + float, no narrowing.
    let src = "local fX = 1.0;\nlocal iY = 2;\nlocal fZ = fX + Convert.ToFloat(iY);\n";
    assert!(!codes(src).contains(&TypeCode::T003));
}

#[test]
fn t003_no_flag_pure_float() {
    assert!(
        !codes("local fX = 1.0;\nlocal fY = 2.0;\nlocal fZ = fX + fY;\n").contains(&TypeCode::T003)
    );
}

#[test]
fn t003_flags_float_to_integer_narrowing_compound_assign() {
    // iX is an integer target; `+=`/`*=` of a float narrows on store (precision loss).
    assert!(codes("local iX = 1;\niX += 2.0;\n").contains(&TypeCode::T003));
    assert!(codes("local iX = 1;\niX *= 3.5;\n").contains(&TypeCode::T003));
}

#[test]
fn t003_no_flag_compound_assign_widening_into_float_target() {
    // Float target, integer value: widening, safe.
    assert!(!codes("local fX = 1.0;\nfX += 2;\n").contains(&TypeCode::T003));
}

#[test]
fn t003_no_flag_compound_assign_pure_float() {
    assert!(!codes("local fX = 1.0;\nfX += 2.0;\n").contains(&TypeCode::T003));
}

#[test]
fn t003_no_flag_compound_assign_pure_int() {
    assert!(!codes("local iX = 1;\niX += 2;\n").contains(&TypeCode::T003));
}
