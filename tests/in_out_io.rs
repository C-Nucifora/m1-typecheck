//! #233 — M1 In/Out function I/O model. M1 has no `return` keyword: function
//! arguments are read via `In.<param>` and the return value is set by assigning
//! `Out = <expr>`. These checks mirror M1 Build "Validate Project" errors on a
//! `FuncUserParam` / user method with a non-empty `<Signature>`:
//! T100 (bare param ref / 1338), T101 (return statement / 1338),
//! T098 (unused argument / 1355), T099 (return value not assigned / 1353).
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;
use std::path::Path;

fn proj() -> Project {
    Project::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantics.m1prj"))
        .unwrap()
}

fn codes(file: &str, src: &str) -> Vec<(TypeCode, m1_core::Severity, String)> {
    let p = proj();
    check_script(&p, Path::new(file), src)
        .diagnostics
        .into_iter()
        .map(|d| (d.code, d.inner.severity, d.inner.message))
        .collect()
}

fn has(v: &[(TypeCode, m1_core::Severity, String)], c: TypeCode) -> bool {
    v.iter().any(|(k, _, _)| *k == c)
}

// Root.Ctrl.Scale (Ctrl.Scale.m1scr): params Input(f32), Gain(s32); ReturnType f32.
// Root.Ctrl.Helper (Ctrl.Helper.m1scr): param Input(f32); ReturnType f32.
// Root.Ctrl.Alpha (Ctrl.Alpha.m1scr): FuncUser, no <Signature>.

// ── T100 bare-parameter-reference (M1 Build 1338) ─────────────────────────

#[test]
fn bare_param_reference_is_flagged_t100() {
    // `Input` read by its bare name instead of `In.Input`.
    let got = codes(
        "Ctrl.Scale.m1scr",
        "Out = In.Input + In.Gain;\nlocal r = Input;\n",
    );
    assert!(has(&got, TypeCode::T100), "bare param ref flagged: {got:?}");
    assert!(
        got.iter()
            .any(|(c, s, _)| *c == TypeCode::T100 && *s == m1_core::Severity::Error),
        "T100 is an Error (M1 Build parity): {got:?}"
    );
}

#[test]
fn bare_param_reference_supersedes_t001() {
    // The bare ref previously surfaced only as a generic T001 warning; T100
    // owns it now, so T001 must not double-report at the same location.
    let got = codes(
        "Ctrl.Scale.m1scr",
        "Out = In.Input + In.Gain;\nlocal r = Input;\n",
    );
    assert!(has(&got, TypeCode::T100));
    assert!(!has(&got, TypeCode::T001), "T100 supersedes T001: {got:?}");
}

#[test]
fn in_param_read_is_not_flagged() {
    // The correct M1 form reads every arg via `In.<name>` and assigns `Out`.
    let got = codes("Ctrl.Scale.m1scr", "Out = In.Input + In.Gain;\n");
    assert!(
        !has(&got, TypeCode::T100),
        "In.<param> not flagged: {got:?}"
    );
    assert!(!has(&got, TypeCode::T098), "all params read: {got:?}");
    assert!(!has(&got, TypeCode::T099), "Out assigned: {got:?}");
    assert!(!has(&got, TypeCode::T101));
}

// ── T101 return-statement (M1 Build 1338; M1 has no `return`) ──────────────

#[test]
fn return_statement_is_flagged_t101() {
    let got = codes("Ctrl.Helper.m1scr", "Out = In.Input;\nreturn result;\n");
    assert!(
        has(&got, TypeCode::T101),
        "return statement flagged: {got:?}"
    );
    assert!(
        got.iter()
            .any(|(c, s, _)| *c == TypeCode::T101 && *s == m1_core::Severity::Error),
        "T101 is an Error: {got:?}"
    );
    assert!(!has(&got, TypeCode::T001), "T101 supersedes T001: {got:?}");
}

// ── T098 unused-argument (M1 Build 1355) ──────────────────────────────────

#[test]
fn unused_declared_param_is_flagged_t098() {
    // Gain is declared but only Input is read → Gain is an unused argument.
    let got = codes("Ctrl.Scale.m1scr", "Out = In.Input;\n");
    assert!(
        got.iter()
            .any(|(c, _, m)| *c == TypeCode::T098 && m.contains("Gain")),
        "unused arg Gain flagged: {got:?}"
    );
    assert!(
        got.iter()
            .any(|(c, s, _)| *c == TypeCode::T098 && *s == m1_core::Severity::Error),
        "T098 is an Error: {got:?}"
    );
}

// ── T099 return-value-not-assigned (M1 Build 1353) ────────────────────────

#[test]
fn missing_out_with_declared_return_type_is_flagged_t099() {
    // Helper declares ReturnType=f32 but never assigns Out.
    let got = codes("Ctrl.Helper.m1scr", "local r = In.Input;\n");
    assert!(has(&got, TypeCode::T099), "missing Out flagged: {got:?}");
    assert!(
        got.iter()
            .any(|(c, s, _)| *c == TypeCode::T099 && *s == m1_core::Severity::Error),
        "T099 is an Error: {got:?}"
    );
}

#[test]
fn out_member_assignment_satisfies_return_sink() {
    // `Out.Foo = …` still writes the return object — not a missing return value.
    let got = codes("Ctrl.Helper.m1scr", "Out.Value = In.Input;\n");
    assert!(
        !has(&got, TypeCode::T099),
        "Out.* counts as a return: {got:?}"
    );
}

// ── gating: checks apply only to signatured user functions ────────────────

#[test]
fn non_signatured_function_is_unaffected() {
    // Ctrl.Alpha is a FuncUser with no <Signature>: the In/Out parity checks do
    // not apply. A bare miss there stays a generic T001 (unchanged behaviour).
    let got = codes("Ctrl.Alpha.m1scr", "local r = foo;\n");
    assert!(
        !has(&got, TypeCode::T100),
        "no T100 without a signature: {got:?}"
    );
    assert!(!has(&got, TypeCode::T098));
    assert!(!has(&got, TypeCode::T099));
    assert!(
        has(&got, TypeCode::T001),
        "generic miss still T001: {got:?}"
    );
}
