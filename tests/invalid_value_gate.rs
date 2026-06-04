//! End-to-end CI gate for invalid-value (NaN/Inf) provenance tracing (T080, #78).
//!
//! A value annotated `@m1:safety-critical` reached by an unsanitised invalid
//! value must fail the build (a T080 *error* → exit 1). Guarding it with
//! `Math.IsNaN()` must clear it (exit 0). This is the m1-ci gate: m1-typecheck
//! already exits non-zero on any error-severity diagnostic, so T080 needs no
//! separate gate wiring.

use std::fs;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_m1-typecheck")
}

fn write_script(name: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("m1tc_t080_{name}"));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Check.m1scr");
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn unsanitised_safety_critical_fails_the_build() {
    // A division (denominator may be 0) reaching a @safety-critical value, no guard.
    let script = write_script(
        "unguarded",
        "// @m1:safety-critical\nFront Torque = Yaw Moment / Track Width;\n",
    );
    let out = Command::new(bin()).arg(&script).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("T080"),
        "expected a T080 diagnostic, got: {stdout}"
    );
    assert!(
        stdout.contains("error[T080]"),
        "safety-critical must be an error, got: {stdout}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "an unsanitised safety-critical value must fail the build"
    );
}

#[test]
fn sanitised_value_passes() {
    // The in-script taint barrier is an author-declared `@m1:sanitizes`
    // (Math.IsNaN is calibration-only and cannot guard an ECU script).
    let script = write_script(
        "guarded",
        "// @m1:sanitizes\nSafe Moment = Yaw Moment / Track Width;\n\
         // @m1:safety-critical\nFront Torque = Safe Moment;\n",
    );
    let out = Command::new(bin()).arg(&script).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("T080"),
        "a @m1:sanitizes barrier must clear T080, got: {stdout}"
    );
    assert_eq!(out.status.code(), Some(0));
}
