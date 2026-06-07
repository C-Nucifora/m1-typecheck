//! CLI surface (#88): `--select` / `--ignore` / `--format json` / `--rules`, plus
//! the cfg/dbc visibility notes (#87 part 2). End-to-end via the built binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_m1-typecheck")
}

// A project declaring a parameter (`Root.Foo.Gain.Value`). Paired with a cfg that
// loads but omits that parameter (`EMPTY_CONFIG`), the T041 project audit emits a
// warning, giving us a stable T-code to filter on.
const PROJECT: &str = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="Demo" TargetHardware="ecu120">
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Foo"/>
    <Component Classname="BuiltIn.Parameter" Name="Root.Foo.Gain.Value"><Props/></Component>
    <Component Classname="BuiltIn.FuncUser" Filename="Foo Update.m1scr" Name="Root.Foo.Update"/>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

const CONFIG: &str = r#"<?xml version="1.0"?>
<Configuration>
 <Group Name="">
  <Parameter Name="Root.Foo.Gain.Value"><Cell Type="u16" Unit="ratio"><![CDATA[1]]></Cell></Parameter>
 </Group>
</Configuration>
"#;

// A cfg that loads but does NOT cover `Root.Foo.Gain.Value` — so the T041
// project audit fires (the parameter is declared in the .m1prj but absent here).
const EMPTY_CONFIG: &str = r#"<?xml version="1.0"?>
<Configuration>
 <Group Name=""/>
</Configuration>
"#;

// A float compared with == (T002) plus a wrong-arity library call (opt-in T064).
const SCRIPT: &str = "local f = 1.5; if (f == 2.5) { }\nlocal x = Calculate.Max(1, 2, 3);\n";

fn setup(name: &str, cfg: Option<&str>) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Project.m1prj"), PROJECT).unwrap();
    if let Some(c) = cfg {
        fs::write(dir.join("parameters.m1cfg"), c).unwrap();
    }
    let script = dir.join("Foo Update.m1scr");
    fs::write(&script, SCRIPT).unwrap();
    script
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
fn rules_lists_every_t_code() {
    let o = run(&["--rules"]);
    let s = out_of(&o);
    assert!(s.contains("T002  float-equality"), "got:\n{s}");
    assert!(s.contains("T041  missing-cfg-parameter"));
    assert!(s.contains("T064  wrong-argument-count"));
}

#[test]
fn rules_json_is_valid_and_covers_codes() {
    let o = run(&["--rules", "--format", "json"]);
    let s = out_of(&o);
    let v: serde_json::Value = serde_json::from_str(&s).expect("valid JSON");
    let rules = v["rules"].as_array().unwrap();
    let codes: Vec<&str> = rules.iter().map(|r| r["code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"T001"));
    assert!(codes.contains(&"T064"));
}

#[test]
fn format_json_emits_valid_json() {
    let script = setup("cli_json", Some(EMPTY_CONFIG));
    let o = run(&["--format", "json", script.to_str().unwrap()]);
    let s = out_of(&o);
    let v: serde_json::Value = serde_json::from_str(&s).expect("valid JSON document");
    assert_eq!(v["version"], 1);
    // The T002 float-equality diagnostic is present in the file's diagnostics.
    let diags = v["files"][0]["diagnostics"].as_array().unwrap();
    assert!(
        diags.iter().any(|d| d["code"] == "T002"),
        "expected T002 in JSON, got:\n{s}"
    );
    // The T041 project audit lands in the project array (cfg omits the param).
    let proj = v["project"].as_array().unwrap();
    assert!(
        proj.iter().any(|d| d["code"] == "T041"),
        "expected T041 audit"
    );
}

#[test]
fn ignore_suppresses_a_code() {
    let script = setup("cli_ignore", Some(EMPTY_CONFIG));
    // Without --ignore, T041 is reported.
    let base = out_of(&run(&[script.to_str().unwrap()]));
    assert!(base.contains("T041"), "baseline should show T041:\n{base}");
    // --ignore T041 removes it (but keeps the others, e.g. T002).
    let filtered = out_of(&run(&["--ignore", "T041", script.to_str().unwrap()]));
    assert!(
        !filtered.contains("T041"),
        "T041 should be gone:\n{filtered}"
    );
    assert!(filtered.contains("T002"), "T002 should remain:\n{filtered}");
}

#[test]
fn select_whitelists_only_named_codes() {
    let script = setup("cli_select", Some(EMPTY_CONFIG));
    let s = out_of(&run(&["--select", "T002", script.to_str().unwrap()]));
    assert!(s.contains("T002"), "T002 should be present:\n{s}");
    assert!(!s.contains("T041"), "T041 should be filtered out:\n{s}");
}

#[test]
fn select_activates_opt_in_t064() {
    let script = setup("cli_optin", None);
    // T064 is opt-in: silent by default...
    let off = out_of(&run(&[script.to_str().unwrap()]));
    assert!(!off.contains("T064"), "T064 must be off by default:\n{off}");
    // ...and on only when explicitly selected.
    let on = out_of(&run(&["--select", "T064", script.to_str().unwrap()]));
    assert!(on.contains("T064"), "T064 should fire when selected:\n{on}");
}

#[test]
fn note_when_no_cfg_or_dbc_discovered() {
    // #87 part 2: a project that loads but finds no cfg/dbc must announce the
    // skipped audits on stderr, so a green CI run isn't mistaken for "all clean".
    let script = setup("cli_note_absent", None);
    let e = err_of(&run(&[script.to_str().unwrap()]));
    assert!(
        e.contains("no parameters.m1cfg found") && e.contains("T041"),
        "expected the cfg-skipped note, got:\n{e}"
    );
    assert!(
        e.contains("no .m1dbc found") && e.contains("T042"),
        "expected the dbc-skipped note, got:\n{e}"
    );
}

#[test]
fn no_cfg_note_when_cfg_present() {
    // With a cfg present, the cfg-skipped note must NOT appear (the dbc note may,
    // since this fixture ships no .m1dbc).
    let script = setup("cli_note_present", Some(CONFIG));
    let e = err_of(&run(&[script.to_str().unwrap()]));
    assert!(
        !e.contains("no parameters.m1cfg found"),
        "cfg note must be absent when a cfg is present, got:\n{e}"
    );
}

#[test]
fn unknown_filter_code_is_rejected() {
    // #111: a bogus --ignore/--select code must be rejected, not silently
    // accepted (which hid typos that disabled nothing).
    let o = run(&["--ignore", "NOTACODE"]);
    assert_ne!(
        o.status.code(),
        Some(0),
        "an unknown --ignore code must exit non-zero; stderr:\n{}",
        err_of(&o)
    );
    let e = err_of(&o).to_lowercase();
    assert!(
        e.contains("notacode") || e.contains("unknown"),
        "stderr should name the bad code:\n{}",
        err_of(&o)
    );
    // A valid code is still accepted (no spurious rejection).
    let ok = run(&["--ignore", "T041"]);
    assert_eq!(ok.status.code(), Some(0), "a valid code must be accepted");
}

#[test]
fn version_flag_prints_version() {
    // #112: report the version like the other CLIs.
    let o = run(&["--version"]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", err_of(&o));
    assert!(
        out_of(&o).contains(env!("CARGO_PKG_VERSION")),
        "stdout: {}",
        out_of(&o)
    );
}
