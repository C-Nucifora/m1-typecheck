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
fn t002_fires_through_a_derived_local_chain() {
    // #89(a): a local initialised from another local must inherit its type
    // (declaration-order threading), so comparing two float-derived locals flags
    // T002 exactly as comparing two float literals does.
    let derived = "local a = 1.5; local b = a; local c = a; if (b == c) { }\n";
    let direct = "local b = 1.5; local c = 2.5; if (b == c) { }\n";
    assert!(
        codes(direct).contains(&TypeCode::T002),
        "baseline: literal float equality should flag T002"
    );
    assert!(
        codes(derived).contains(&TypeCode::T002),
        "derived-local float equality must now flag T002 too, got {:?}",
        codes(derived)
    );
}

#[test]
fn forward_local_reference_stays_unknown() {
    // A forward reference (a local initialised from one declared LATER) must not
    // resolve — only backward references are typed. Here `b = c` precedes `c`'s
    // float declaration, so `b` stays Unknown. Compared against an integer literal,
    // an Unknown `b` yields no T002; a (wrongly) Float `b` would. The comparison
    // uses only `b` and an int literal so the later float `c` can't itself trip it.
    let src = "local b = c; local c = 1.5; if (b == 5) { }\n";
    assert!(
        !codes(src).contains(&TypeCode::T002),
        "forward reference must stay Unknown (no T002 from `b`), got {:?}",
        codes(src)
    );
}

#[test]
fn t003_allows_int_float_widening_in_arithmetic() {
    // M1 Build implicitly widens the integer to float; this is not flagged (#17).
    assert!(
        !codes("local fX = 1.0;\nlocal iY = 2;\nlocal fZ = fX + iY;\n").contains(&TypeCode::T003)
    );
}

#[test]
fn t003_no_flag_with_convert_call() {
    // `Convert.ToInteger` returns Integer; Float + Integer is implicit widening,
    // so no T003. (There is deliberately no `Convert.ToFloat` in M1: int->float
    // widening is implicit, so only the narrowing direction has a Convert fn.)
    let src = "local fX = 1.0;\nlocal iY = 2;\nlocal fZ = fX + Convert.ToInteger(iY);\n";
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
