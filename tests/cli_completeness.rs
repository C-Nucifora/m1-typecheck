//! CLI surface for the analysis-completeness report (#259): `--completeness`
//! prints coverage telemetry (not findings) and exits 0, in both human and JSON
//! form. End-to-end via the built binary; all identifiers are synthetic.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_m1-typecheck")
}

const PROJECT: &str = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="Demo" TargetHardware="ecu120">
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Sensors"/>
    <Component Classname="BuiltIn.Channel" Name="Root.Sensors.Yaw"><Props Type="f32"/></Component>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

// A clean script: a typed local, an intrinsic call, and a resolved channel read.
const SCRIPT: &str = "local x = Calculate.Max(1, 2);\nSensors.Yaw = x + 1.0;\n";

fn setup(name: &str) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let prj = dir.join("Project.m1prj");
    fs::write(&prj, PROJECT).unwrap();
    let s = dir.join("Sensors.Update.m1scr");
    fs::write(&s, SCRIPT).unwrap();
    (prj, s)
}

fn run(args: &[&str]) -> Output {
    Command::new(bin()).args(args).output().unwrap()
}
fn out_of(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn completeness_human_prints_report_and_exits_zero() {
    let (prj, s) = setup("completeness_human");
    let o = run(&[
        "--project",
        prj.to_str().unwrap(),
        "--completeness",
        s.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "completeness exits 0");
    let out = out_of(&o);
    assert!(
        out.contains("analysis completeness report"),
        "human header present:\n{out}"
    );
    assert!(out.contains("expressions:"), "expression line:\n{out}");
    assert!(out.contains("references:"), "reference line:\n{out}");
    assert!(
        out.contains("intrinsic calls:"),
        "intrinsic-call line:\n{out}"
    );
    // No .m1cfg present, so the degraded-input note appears.
    assert!(out.contains(".m1cfg absent"), "cfg-absent note:\n{out}");
}

#[test]
fn completeness_json_emits_coverage_document() {
    let (prj, s) = setup("completeness_json");
    let o = run(&[
        "--project",
        prj.to_str().unwrap(),
        "--format",
        "json",
        "--completeness",
        s.to_str().unwrap(),
    ]);
    assert!(o.status.success());
    let out = out_of(&o);
    // Shape: a single completeness document with the nested coverage sections.
    assert!(
        out.contains("\"completeness\":{"),
        "completeness object:\n{out}"
    );
    for key in [
        "\"scripts\":{",
        "\"expressions\":{",
        "\"typed_percent\":",
        "\"references\":{",
        "\"intrinsic_calls\":{",
        "\"when_subjects\":{",
        "\"inputs\":{",
        "\"cfg_loaded\":false",
        "\"dbc_loaded\":false",
    ] {
        assert!(out.contains(key), "missing {key} in:\n{out}");
    }
    // The intrinsic call (Calculate.Max) is catalogued, so none unmodelled.
    assert!(
        out.contains("\"intrinsic_calls\":{\"total\":1,\"unmodelled\":0}"),
        "one modelled intrinsic call:\n{out}"
    );
}

#[test]
fn completeness_reports_unmodelled_intrinsic_method() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("completeness_unmodelled");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let prj = dir.join("Project.m1prj");
    fs::write(&prj, PROJECT).unwrap();
    let s = dir.join("Sensors.Update.m1scr");
    // Calculate is a catalogued object, but NoSuchMethod is not modelled.
    fs::write(&s, "local x = Calculate.NoSuchMethod(1);\n").unwrap();

    let o = run(&[
        "--project",
        prj.to_str().unwrap(),
        "--format",
        "json",
        "--completeness",
        s.to_str().unwrap(),
    ]);
    assert!(o.status.success());
    let out = out_of(&o);
    assert!(
        out.contains("\"intrinsic_calls\":{\"total\":1,\"unmodelled\":1}"),
        "the unmodelled method is counted:\n{out}"
    );
}

#[test]
fn completeness_without_project_still_runs() {
    // Project-less mode: the report should still work over the raw script.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("completeness_no_project");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let s = dir.join("loose.m1scr");
    fs::write(&s, "local x = 1 + 2;\n").unwrap();

    let o = run(&["--completeness", s.to_str().unwrap()]);
    assert!(o.status.success(), "project-less completeness exits 0");
    assert!(
        out_of(&o).contains("analysis completeness report"),
        "report still prints"
    );
}
