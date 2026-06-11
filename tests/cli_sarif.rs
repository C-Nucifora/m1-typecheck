//! `--format sarif` (#185): SARIF 2.1.0 output for GitHub code scanning,
//! mirroring `m1-lint --format sarif`. End-to-end via the built binary.

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
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Foo"/>
    <Component Classname="BuiltIn.Parameter" Name="Root.Foo.Gain.Value"><Props/></Component>
    <Component Classname="BuiltIn.FuncUser" Filename="Foo Update.m1scr" Name="Root.Foo.Update"/>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

// Loads, but omits `Root.Foo.Gain.Value` — the T041 project audit fires.
const EMPTY_CONFIG: &str = r#"<?xml version="1.0"?>
<Configuration>
 <Group Name=""/>
</Configuration>
"#;

// A float compared with `==` — the T002 script diagnostic, on line 1.
const SCRIPT: &str = "local f = 1.5; if (f == 2.5) { }\n";

fn setup(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Project.m1prj"), PROJECT).unwrap();
    fs::write(dir.join("parameters.m1cfg"), EMPTY_CONFIG).unwrap();
    let script = dir.join("Foo Update.m1scr");
    fs::write(&script, SCRIPT).unwrap();
    script
}

fn run(args: &[&str]) -> Output {
    Command::new(bin()).args(args).output().unwrap()
}

#[test]
fn sarif_document_is_valid_and_carries_script_and_project_findings() {
    let script = setup("cli_sarif");
    let o = run(&["--format", "sarif", script.to_str().unwrap()]);
    let s = String::from_utf8_lossy(&o.stdout);
    let v: serde_json::Value = serde_json::from_str(&s).expect("valid JSON document");

    assert_eq!(v["version"], "2.1.0");
    assert!(
        v["$schema"]
            .as_str()
            .unwrap()
            .contains("sarif-schema-2.1.0"),
        "missing schema URI"
    );

    let driver = &v["runs"][0]["tool"]["driver"];
    assert_eq!(driver["name"], "m1-typecheck");
    let rules = driver["rules"].as_array().unwrap();
    let ids: Vec<&str> = rules.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"T002"), "rules must list every T-code");
    assert!(ids.contains(&"T041"));
    assert!(ids.contains(&"syntax"));

    let results = v["runs"][0]["results"].as_array().unwrap();

    // The script-level T002 finding, 1-based region on the script artifact.
    let t002 = results
        .iter()
        .find(|r| r["ruleId"] == "T002")
        .expect("T002 result");
    assert_eq!(t002["level"], "error");
    let loc = &t002["locations"][0]["physicalLocation"];
    assert!(
        loc["artifactLocation"]["uri"]
            .as_str()
            .unwrap()
            .ends_with("Foo Update.m1scr")
    );
    assert_eq!(loc["region"]["startLine"], 1);

    // The project-level T041 audit anchors to the project file at line 1.
    let t041 = results
        .iter()
        .find(|r| r["ruleId"] == "T041")
        .expect("T041 result");
    assert_eq!(t041["level"], "note");
    let loc = &t041["locations"][0]["physicalLocation"];
    assert!(
        loc["artifactLocation"]["uri"]
            .as_str()
            .unwrap()
            .ends_with("Project.m1prj")
    );
    assert_eq!(loc["region"]["startLine"], 1);
    assert!(
        t041["message"]["text"]
            .as_str()
            .unwrap()
            .contains("Root.Foo.Gain.Value"),
        "subject symbol kept in the message text"
    );
}

#[test]
fn sarif_exit_code_matches_json_mode() {
    let script = setup("cli_sarif_exit");
    // T002 is error severity, so both machine formats exit 1.
    let sarif = run(&["--format", "sarif", script.to_str().unwrap()]);
    let json = run(&["--format", "json", script.to_str().unwrap()]);
    assert_eq!(sarif.status.code(), json.status.code());
    assert_eq!(sarif.status.code(), Some(1));
}

#[test]
fn syntax_errors_use_the_synthetic_rule() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_sarif_syntax");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let script = dir.join("Broken.m1scr");
    fs::write(&script, "if (a {\n").unwrap();
    let o = run(&["--format", "sarif", script.to_str().unwrap()]);
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&o.stdout)).expect("valid JSON");
    let results = v["runs"][0]["results"].as_array().unwrap();
    assert!(
        results
            .iter()
            .any(|r| r["ruleId"] == "syntax" && r["level"] == "error"),
        "syntax error reported under the synthetic rule"
    );
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn rules_with_sarif_format_prints_the_json_catalogue() {
    let o = run(&["--rules", "--format", "sarif"]);
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&o.stdout)).expect("valid JSON");
    assert!(v["rules"].as_array().unwrap().len() > 30);
}
