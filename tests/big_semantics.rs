//! Wave-2b semantics: declared signatures + In-params (#110 / T085), unit
//! mismatch (T086, #144) and scheduling (T088/T089, #145).
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;
use std::path::Path;

fn proj() -> Project {
    Project::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantics.m1prj"))
        .unwrap()
}

fn codes(p: &Project, file: &str, src: &str) -> Vec<(TypeCode, String)> {
    check_script(p, Path::new(file), src)
        .diagnostics
        .into_iter()
        .map(|d| (d.code, d.inner.message))
        .collect()
}

fn has(codes: &[(TypeCode, String)], c: TypeCode) -> bool {
    codes.iter().any(|(k, _)| *k == c)
}

// ---- #110: declared signatures --------------------------------------------

#[test]
fn signature_params_are_parsed() {
    let p = proj();
    let sym = p.symbols().get("Root.Ctrl.Scale").expect("Scale symbol");
    let params = sym.in_params.as_ref().expect("declared params");
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].0, "Input");
    assert_eq!(params[1].0, "Gain");
    assert!(sym.return_type.is_some(), "declared ReturnType=f32 stored");
}

#[test]
fn in_param_reads_are_typed_in_the_callee() {
    let p = proj();
    // Inside Ctrl.Scale.m1scr, `In.Input` is a declared f32 — equality on it
    // is float equality (T002).
    let got = codes(&p, "Ctrl.Scale.m1scr", "if (In.Input == 1.0) { }\n");
    assert!(has(&got, TypeCode::T002), "In.Input typed Float: {got:?}");
    // An unknown In member stays opaque — no T001/T002 noise.
    let opaque = codes(&p, "Ctrl.Scale.m1scr", "local x = In.Mystery;\n");
    assert!(
        !has(&opaque, TypeCode::T001),
        "unknown In member stays opaque: {opaque:?}"
    );
}

#[test]
fn t085_flags_wrong_arity() {
    let p = proj();
    let got = codes(&p, "Ctrl.Alpha.m1scr", "A Out = Scale(1.0);\n");
    assert!(has(&got, TypeCode::T085), "1 arg for 2 params: {got:?}");
    let ok = codes(&p, "Ctrl.Alpha.m1scr", "A Out = Scale(1.0, 2);\n");
    assert!(!has(&ok, TypeCode::T085), "correct call clean: {ok:?}");
}

#[test]
fn t085_flags_wrong_argument_type() {
    let p = proj();
    // Gain is declared s32; passing a float literal flags position 2.
    let got = codes(&p, "Ctrl.Alpha.m1scr", "A Out = Scale(1.0, 2.5);\n");
    assert!(
        got.iter()
            .any(|(c, m)| *c == TypeCode::T085 && m.contains("Gain")),
        "float into s32 param flags: {got:?}"
    );
}

// ---- #144: T086 unit mismatch ----------------------------------------------

#[test]
fn t086_flags_cross_quantity_copy() {
    let p = proj();
    let got = codes(&p, "Ctrl.Alpha.m1scr", "Pressure = Yaw Rate;\n");
    assert!(
        got.iter()
            .any(|(c, m)| *c == TypeCode::T086 && m.contains("Pa") && m.contains("deg/s")),
        "Pa = rad/s copy flags with both base units: {got:?}"
    );
}

#[test]
fn t086_same_quantity_copy_is_clean() {
    let p = proj();
    let got = codes(&p, "Ctrl.Alpha.m1scr", "Other Rate = Yaw Rate;\n");
    assert!(!has(&got, TypeCode::T086), "same quantity clean: {got:?}");
}

#[test]
fn t086_computed_rhs_is_unchecked() {
    let p = proj();
    // Arithmetic legitimately changes dimension — stays silent.
    let got = codes(&p, "Ctrl.Alpha.m1scr", "Pressure = Yaw Rate * 2.0;\n");
    assert!(
        !has(&got, TypeCode::T086),
        "computed RHS unchecked: {got:?}"
    );
}

#[test]
fn t086_unitless_side_is_clean() {
    let p = proj();
    let got = codes(&p, "Ctrl.Alpha.m1scr", "A Out = Yaw Rate;\n");
    assert!(!has(&got, TypeCode::T086), "unitless target clean: {got:?}");
}

// ---- #145: scheduling -------------------------------------------------------

fn schedule(p: &Project, t089: bool, scripts: &[(&str, &str)]) -> Vec<(TypeCode, String)> {
    // T088/T097 always on in these tests; T089 per the flag.
    let owned: Vec<(String, String)> = scripts
        .iter()
        .map(|(f, s)| (f.to_string(), s.to_string()))
        .collect();
    m1_typecheck::schedule::check(p, &owned, true, t089, true)
        .into_iter()
        .map(|d| (d.code, d.inner.message))
        .collect()
}

#[test]
fn t088_flags_same_rate_cycle() {
    let p = proj();
    let got = schedule(
        &p,
        false,
        &[
            ("Ctrl.Alpha.m1scr", "A Out = B Out + 1.0;\n"),
            ("Ctrl.Beta.m1scr", "B Out = A Out + 1.0;\n"),
        ],
    );
    assert!(
        got.iter().any(|(c, _)| *c == TypeCode::T088),
        "100Hz<->100Hz write/read cycle flags: {got:?}"
    );
}

// ---- T097: recursive-call ---------------------------------------------------

#[test]
fn t097_flags_direct_self_recursion() {
    let p = proj();
    let got = schedule(
        &p,
        false,
        &[("Ctrl.Scale.m1scr", "A Out = Scale(In.Input, 2);\n")],
    );
    assert!(
        got.iter()
            .any(|(c, m)| *c == TypeCode::T097 && m.contains("Root.Ctrl.Scale")),
        "a user function calling itself flags T097: {got:?}"
    );
}

#[test]
fn t097_flags_mutual_recursion_once() {
    let p = proj();
    let got = schedule(
        &p,
        false,
        &[
            ("Ctrl.Scale.m1scr", "A Out = Helper(1.0);\n"),
            ("Ctrl.Helper.m1scr", "B Out = Scale(1.0, 2);\n"),
        ],
    );
    let hits: Vec<_> = got.iter().filter(|(c, _)| *c == TypeCode::T097).collect();
    assert_eq!(hits.len(), 1, "one cycle, one report: {got:?}");
    assert!(
        hits[0].1.contains("Root.Ctrl.Helper") && hits[0].1.contains("Root.Ctrl.Scale"),
        "the cycle names both functions: {got:?}"
    );
}

#[test]
fn t097_plain_user_function_call_is_clean() {
    let p = proj();
    let got = schedule(
        &p,
        false,
        &[
            ("Ctrl.Alpha.m1scr", "A Out = Scale(1.0, 2);\n"),
            ("Ctrl.Scale.m1scr", "B Out = In.Input * 2.0;\n"),
        ],
    );
    assert!(
        !got.iter().any(|(c, _)| *c == TypeCode::T097),
        "an acyclic call is not recursion: {got:?}"
    );
}

#[test]
fn cross_rate_feedback_is_not_a_cycle() {
    let p = proj();
    let got = schedule(
        &p,
        false,
        &[
            ("Ctrl.Alpha.m1scr", "A Out = B Out + 1.0;\n"), // 100 Hz
            ("Ctrl.Fast.m1scr", "B Out = A Out + 1.0;\n"),  // 500 Hz
        ],
    );
    assert!(
        !got.iter().any(|(c, _)| *c == TypeCode::T088),
        "cross-rate feedback is a normal control shape: {got:?}"
    );
}

#[test]
fn t089_is_opt_in_and_flags_fast_reader_of_slow_writer() {
    let p = proj();
    let scripts: &[(&str, &str)] = &[
        ("Ctrl.Alpha.m1scr", "Slow Out = 1.0;\n"), // 100 Hz writer
        ("Ctrl.Fast.m1scr", "B Out = Slow Out + 1.0;\n"), // 500 Hz reader
    ];
    let off = schedule(&p, false, scripts);
    assert!(
        !off.iter().any(|(c, _)| *c == TypeCode::T089),
        "T089 must stay opt-in: {off:?}"
    );
    let on = schedule(&p, true, scripts);
    assert!(
        on.iter()
            .any(|(c, m)| *c == TypeCode::T089 && m.contains("Slow Out")),
        "fast reader of slow-only writer flags: {on:?}"
    );
}
