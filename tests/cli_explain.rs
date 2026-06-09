//! Cross-script CLI surface (#78 P3): the solved taints reach the normal run's
//! diagnostics, and `--explain <channel>` prints a channel's provenance chain.
//! End-to-end via the built binary; all identifiers are synthetic.

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
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Control"/>
    <Component Classname="BuiltIn.Channel" Name="Root.Sensors.Yaw"><Props Type="f32"/></Component>
    <Component Classname="BuiltIn.Channel" Name="Root.Control.Demand"><Props Type="f32"/></Component>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

/// The writer divides (taints `Root.Sensors.Yaw`); the reader stores it into an
/// annotated finiteness sink in a different script.
const WRITER: &str = "Sensors.Yaw = 1 / Control.Demand;\n";
const READER: &str = "// @m1:requires-finite\nControl.Demand = Sensors.Yaw * 2;\n";
const CLEAN_WRITER: &str = "Sensors.Yaw = Control.Demand + 1;\n";

fn setup(name: &str, writer: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let prj = dir.join("Project.m1prj");
    fs::write(&prj, PROJECT).unwrap();
    let w = dir.join("Sensors.Update.m1scr");
    fs::write(&w, writer).unwrap();
    let r = dir.join("Control.Update.m1scr");
    fs::write(&r, READER).unwrap();
    (prj, w, r)
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
fn cross_script_t080_fires_in_a_normal_run() {
    let (prj, w, r) = setup("xs_normal", WRITER);
    let o = run(&[
        "--project",
        prj.to_str().unwrap(),
        w.to_str().unwrap(),
        r.to_str().unwrap(),
    ]);
    let s = out_of(&o);
    assert!(
        s.contains("T080") && s.contains("Control.Update.m1scr"),
        "the reader's sink reports cross-script T080:\n{s}"
    );
    assert!(
        s.contains("Sensors.Update.m1scr"),
        "the message names the writing script:\n{s}"
    );
}

#[test]
fn explain_prints_the_provenance_chain() {
    let (prj, w, r) = setup("xs_explain", WRITER);
    let o = run(&[
        "--project",
        prj.to_str().unwrap(),
        "--explain",
        "Sensors.Yaw",
        w.to_str().unwrap(),
        r.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "explain exits 0: {}", err_of(&o));
    let s = out_of(&o);
    assert!(s.contains("can be NaN/Inf"), "status line:\n{s}");
    assert!(
        s.contains("Sensors.Update.m1scr"),
        "chain names the writer:\n{s}"
    );
}

#[test]
fn explain_reports_a_clean_channel() {
    let (prj, w, r) = setup("xs_clean", CLEAN_WRITER);
    let o = run(&[
        "--project",
        prj.to_str().unwrap(),
        "--explain",
        "Sensors.Yaw",
        w.to_str().unwrap(),
        r.to_str().unwrap(),
    ]);
    assert!(o.status.success());
    assert!(
        out_of(&o).contains("no invalid-value"),
        "clean status:\n{}",
        out_of(&o)
    );
}

#[test]
fn explain_unknown_channel_exits_2() {
    let (prj, w, _) = setup("xs_unknown", WRITER);
    let o = run(&[
        "--project",
        prj.to_str().unwrap(),
        "--explain",
        "Nope.Missing",
        w.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        err_of(&o).contains("unknown channel"),
        "stderr:\n{}",
        err_of(&o)
    );
}

#[test]
fn explain_json_emits_the_chain() {
    let (prj, w, r) = setup("xs_json", WRITER);
    let o = run(&[
        "--project",
        prj.to_str().unwrap(),
        "--format",
        "json",
        "--explain",
        "Sensors.Yaw",
        w.to_str().unwrap(),
        r.to_str().unwrap(),
    ]);
    assert!(o.status.success());
    let s = out_of(&o);
    assert!(
        s.contains("\"explain\"") && s.contains("\"invalid\":true") && s.contains("\"chain\":["),
        "json document:\n{s}"
    );
}
