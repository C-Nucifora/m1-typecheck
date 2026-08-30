//! Related declarations retain their actual Project/DBC file through every CLI
//! format. Lines in the public/JSON model are 0-based; human and SARIF are 1-based.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_m1-typecheck")
}

const PROJECT: &str = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="Related" TargetHardware="ecu120">
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Foo"/>
    <Component Classname="BuiltIn.Channel" Name="Root.Foo.Count"><Props Type="u32"/></Component>
    <Component Classname="BuiltIn.FuncUser" Filename="Foo.Update.m1scr" Name="Root.Foo.Update"/>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

const DBC: &str = r#"<?xml version="1.0"?>
<DBCs>
 <Component Classname="BuiltIn.CAN.Signal" Name="Bus.Msg.Sig"><Props Type="u32" StartBit="0" Length="8"/></Component>
</DBCs>
"#;

const SCRIPT: &str = "Count = 1.5;\nBus.Msg.Sig = 1.5;\n";
const PROJECT_LINE: u64 = 6;
const DBC_LINE: u64 = 2;

struct Fixture {
    project: PathBuf,
    dbc: PathBuf,
    script: PathBuf,
}

fn setup(name: &str) -> Fixture {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("dbc")).unwrap();
    let project = dir.join("Project.m1prj");
    let dbc = dir.join("dbc/Bus.m1dbc");
    let script = dir.join("Foo.Update.m1scr");
    fs::write(&project, PROJECT).unwrap();
    fs::write(&dbc, DBC).unwrap();
    fs::write(&script, SCRIPT).unwrap();
    Fixture {
        project,
        dbc,
        script,
    }
}

fn run(format: &str, script: &Path) -> Output {
    Command::new(bin())
        .args([
            "--select",
            "T030",
            "--format",
            format,
            script.to_str().unwrap(),
        ])
        .output()
        .unwrap()
}

#[test]
fn human_notes_name_project_and_dbc_declaration_files() {
    let fixture = setup("cli_related_human");
    let output = run("human", &fixture.script);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains(&format!(
            "{}:{}",
            fixture.project.display(),
            PROJECT_LINE + 1
        )),
        "Project declaration missing from:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("{}:{}", fixture.dbc.display(), DBC_LINE + 1)),
        "DBC declaration missing from:\n{stdout}"
    );
}

#[test]
fn json_related_entries_name_project_and_dbc_declaration_files() {
    let fixture = setup("cli_related_json");
    let output = run("json", &fixture.script);
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON output");
    let related: Vec<&serde_json::Value> = document["files"][0]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "T030")
        .flat_map(|d| d["related"].as_array().unwrap())
        .collect();

    let project = related
        .iter()
        .find(|r| r["kind"] == "project")
        .expect("Project related entry");
    assert_eq!(project["path"], fixture.project.display().to_string());
    assert_eq!(project["line"], PROJECT_LINE);
    assert_eq!(project["project_line"], PROJECT_LINE);

    let dbc = related
        .iter()
        .find(|r| r["kind"] == "dbc")
        .expect("DBC related entry");
    assert_eq!(dbc["path"], fixture.dbc.display().to_string());
    assert_eq!(dbc["line"], DBC_LINE);
    assert!(dbc.get("project_line").is_none());
}

#[test]
fn sarif_related_locations_name_project_and_dbc_declaration_files() {
    let fixture = setup("cli_related_sarif");
    let output = run("sarif", &fixture.script);
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid SARIF output");
    let related: Vec<&serde_json::Value> = document["runs"][0]["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|result| result["ruleId"] == "T030")
        .flat_map(|result| result["relatedLocations"].as_array().unwrap())
        .collect();

    assert!(related.iter().any(|location| {
        location["physicalLocation"]["artifactLocation"]["uri"]
            == fixture.project.display().to_string()
            && location["physicalLocation"]["region"]["startLine"] == PROJECT_LINE + 1
    }));
    assert!(related.iter().any(|location| {
        location["physicalLocation"]["artifactLocation"]["uri"] == fixture.dbc.display().to_string()
            && location["physicalLocation"]["region"]["startLine"] == DBC_LINE + 1
    }));
}
