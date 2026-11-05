//! Diagnostics whose severity and trigger follow M1 Build's own validation.
//! Each test names the native error or warning it mirrors.
use m1_core::Severity;
use m1_typecheck::{diagnostics::TypeCode, project::Project, rules::check_script};
use std::path::Path;

fn project() -> Project {
    Project::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native_parity.m1prj"))
        .unwrap()
}

/// Findings for a body of the scheduled `Root.Parity.Update` function.
fn findings(source: &str) -> Vec<(TypeCode, Severity)> {
    check_script(&project(), Path::new("Parity.Update.m1scr"), source)
        .diagnostics
        .into_iter()
        .map(|d| (d.code, d.inner.severity))
        .collect()
}

fn has(source: &str, code: TypeCode, severity: Severity) -> bool {
    findings(source).contains(&(code, severity))
}

#[test]
fn unresolved_reads_are_errors_1338() {
    assert!(has(
        "local x = Parity.Missing;\nSystem.Unused(x);\n",
        TypeCode::T001,
        Severity::Error
    ));
}

#[test]
fn incompatible_assignments_are_errors_1320() {
    for source in ["B = PF;\n", "E = 3;\n", "I = PF;\n"] {
        assert!(has(source, TypeCode::T030, Severity::Error), "{source}");
    }
}

#[test]
fn integer_widening_and_signedness_are_accepted() {
    for source in ["F = PI;\n", "U = PI;\n", "I = PU;\n"] {
        assert!(
            !findings(source)
                .iter()
                .any(|(code, _)| *code == TypeCode::T030),
            "{source}"
        );
    }
}

#[test]
fn incompatible_typed_local_initialisers_are_errors_1316() {
    assert!(has(
        "local <Integer> x = 1.5;\nSystem.Unused(x);\n",
        TypeCode::T030,
        Severity::Error
    ));
}

#[test]
fn signed_unsigned_ordering_comparisons_are_errors_1367() {
    assert!(has(
        "local z = PI < PU;\nSystem.Unused(z);\n",
        TypeCode::T004,
        Severity::Error
    ));
}

#[test]
fn user_argument_type_mismatches_are_errors_1301() {
    assert!(has(
        "local r = This.Compute(PF);\nSystem.Unused(r);\n",
        TypeCode::T085,
        Severity::Error
    ));
}
