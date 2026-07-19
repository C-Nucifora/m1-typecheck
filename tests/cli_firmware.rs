//! CLI surface for firmware-keyed intrinsic catalogues (#260): `--firmware`
//! validates the requested target against the embedded catalogue, and the
//! active target surfaces in the completeness report. End-to-end via the built
//! binary; all identifiers are synthetic.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_m1-typecheck")
}

const SCRIPT: &str = "local x = 1 + 2;\n";

fn setup(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let s = dir.join("loose.m1scr");
    fs::write(&s, SCRIPT).unwrap();
    s
}

fn run(args: &[&str]) -> Output {
    Command::new(bin()).args(args).output().unwrap()
}
fn out_of(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err_of(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn firmware_default_target_is_accepted() {
    let s = setup("fw_default");
    let o = run(&["--firmware", "m1-build-2026-06", s.to_str().unwrap()]);
    assert!(
        o.status.success(),
        "the embedded target is accepted: {}",
        err_of(&o)
    );
}

#[test]
fn firmware_unknown_target_fails_loud() {
    let s = setup("fw_unknown");
    let o = run(&["--firmware", "m1-build-2099-01", s.to_str().unwrap()]);
    // An unknown target must fail loud (exit 2), never silently fall back.
    assert_eq!(o.status.code(), Some(2), "unknown target exits 2");
    let err = err_of(&o);
    assert!(
        err.contains("unknown firmware target") && err.contains("m1-build-2099-01"),
        "names the bad target:\n{err}"
    );
    assert!(
        err.contains("m1-build-2026-06"),
        "lists the known target(s):\n{err}"
    );
}

#[test]
fn completeness_reports_the_catalogue_target() {
    let s = setup("fw_completeness_human");
    let o = run(&["--completeness", s.to_str().unwrap()]);
    assert!(o.status.success());
    assert!(
        out_of(&o).contains("catalogue:") && out_of(&o).contains("m1-build-2026-06"),
        "human report names the target:\n{}",
        out_of(&o)
    );
}

#[test]
fn completeness_json_includes_catalogue_target() {
    let s = setup("fw_completeness_json");
    let o = run(&["--format", "json", "--completeness", s.to_str().unwrap()]);
    assert!(o.status.success());
    assert!(
        out_of(&o).contains("\"catalogue_target\":\"m1-build-2026-06\""),
        "json includes the target:\n{}",
        out_of(&o)
    );
}
