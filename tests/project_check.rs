//! Contract tests for the reusable complete-project orchestration API.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use m1_typecheck::filter::DiagFilter;
use m1_typecheck::parsed;
use m1_typecheck::project::Project;
use m1_typecheck::project_check::{self, ProjectCheckOptions, SourceInput};

const PROJECT: &str = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="Pipeline" TargetHardware="ecu120">
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Foo"/>
    <Component Classname="BuiltIn.Parameter" Name="Root.Foo.Gain.Value"><Props/></Component>
    <Component Classname="BuiltIn.Table" Name="Root.Foo.Map"><Props/></Component>
    <Component Classname="BuiltIn.Channel" Name="Root.Foo.Speed">
     <Props Qty="rad/s"><Locale><Default Unit="%"/></Locale></Props>
    </Component>
    <Component Classname="BuiltIn.Channel" Name="Root.Foo.Menu">
     <Props Storage="Flash" Security="Tune"/>
    </Component>
    <Component Classname="BuiltIn.FuncUser" Filename="Foo.Update.m1scr" Name="Root.Foo.Update">
     <Props SelectedTrigger="Root.Events.On 100Hz"/>
    </Component>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

const EMPTY_CONFIG: &str = r#"<?xml version="1.0"?>
<Configuration><Group Name=""/></Configuration>
"#;

const SOURCE: &str = concat!(
    "local f = 1.5; if (f == 2.5) { }\n",
    "local x = Calculate.Max(1, 2, 3);\n",
);

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "m1tc_project_check_{}_{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn loaded_project(dir: &Path) -> Project {
    let config = dir.join("parameters.m1cfg");
    fs::write(&config, EMPTY_CONFIG).unwrap();
    Project::from_xml(PROJECT)
        .unwrap()
        .with_config(&config)
        .unwrap()
}

fn parsed_source() -> Vec<parsed::ParsedScript> {
    parsed::parse_all(&[("Foo.Update.m1scr".to_string(), SOURCE.to_string())])
}

fn project_codes(result: &project_check::ProjectCheckResult) -> Vec<&str> {
    result
        .project_diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn code_counts<'a>(
    codes: impl IntoIterator<Item = &'a str>,
) -> std::collections::BTreeMap<String, usize> {
    let mut counts = std::collections::BTreeMap::new();
    for code in codes {
        *counts.entry(code.to_string()).or_insert(0) += 1;
    }
    counts
}

#[test]
fn complete_default_pipeline_includes_model_and_script_dependent_audits() {
    let dir = temp_dir("defaults");
    let mut project = loaded_project(&dir);
    let scripts = parsed_source();
    let source_path = dir.join("Foo.Update.m1scr");
    let result = project_check::check(
        Some(&mut project),
        &scripts,
        &[SourceInput::at_path(&source_path, SOURCE)],
        &ProjectCheckOptions::default(),
    );
    let codes = project_codes(&result);
    for expected in ["T041", "T092", "T095", "T111"] {
        assert!(codes.contains(&expected), "missing {expected}: {codes:?}");
    }
    assert!(
        result.sources[0]
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "T002")
    );
    assert!(
        result.sources[0]
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code.as_str() != "T064"),
        "T064 must remain opt-in"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn discovered_select_activates_opt_in_source_rules_and_filters_project_findings() {
    let dir = temp_dir("select");
    fs::write(
        dir.join("m1-tools.toml"),
        "[diagnostics]\nselect = [\"T064\"]\n",
    )
    .unwrap();
    let mut project = loaded_project(&dir);
    let scripts = parsed_source();
    let source_path = dir.join("Foo.Update.m1scr");
    let result = project_check::check(
        Some(&mut project),
        &scripts,
        &[SourceInput::at_path(&source_path, SOURCE)],
        &ProjectCheckOptions::discover(Some(&dir)),
    );
    assert!(result.project_diagnostics.is_empty());
    assert_eq!(result.sources[0].diagnostics.len(), 1);
    assert_eq!(result.sources[0].diagnostics[0].code.as_str(), "T064");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn discovered_ignore_and_ignore_symbols_apply_to_project_findings() {
    let dir = temp_dir("ignore");
    fs::write(
        dir.join("m1-tools.toml"),
        concat!(
            "[diagnostics]\n",
            "ignore = [\"T095\"]\n",
            "ignore_symbols = [",
            "\"T041:Root.Foo.Gain.Value\", ",
            "\"T092:Root.Foo.Map\"",
            "]\n",
        ),
    )
    .unwrap();
    let mut project = loaded_project(&dir);
    let scripts = parsed_source();
    let result = project_check::check(
        Some(&mut project),
        &scripts,
        &[SourceInput::inline(SOURCE)],
        &ProjectCheckOptions::discover(Some(&dir)),
    );
    let codes = project_codes(&result);
    assert!(!codes.contains(&"T041"), "{codes:?}");
    assert!(!codes.contains(&"T092"), "{codes:?}");
    assert!(!codes.contains(&"T095"), "{codes:?}");
    assert!(
        codes.contains(&"T111"),
        "unrelated findings survive: {codes:?}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn selecting_t089_activates_the_opt_in_project_schedule_rule() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantics.m1prj");
    let mut project = Project::load(&fixture).unwrap();
    let scripts = parsed::parse_all(&[
        (
            "Ctrl.Alpha.m1scr".to_string(),
            "Slow Out = 1.0;\n".to_string(),
        ),
        (
            "Ctrl.Fast.m1scr".to_string(),
            "B Out = Slow Out + 1.0;\n".to_string(),
        ),
    ]);
    let options = ProjectCheckOptions {
        filter: DiagFilter {
            select: ["T089".to_string()].into_iter().collect(),
            ..DiagFilter::default()
        },
        audit_names: false,
    };
    let result = project_check::check(Some(&mut project), &scripts, &[], &options);
    assert_eq!(result.project_diagnostics.len(), 1, "{result:?}");
    assert_eq!(result.project_diagnostics[0].code.as_str(), "T089");
}

#[test]
fn cli_and_library_entry_point_produce_identical_code_counts() {
    let dir = temp_dir("cli_parity");
    let project_path = dir.join("Project.m1prj");
    let config_path = dir.join("parameters.m1cfg");
    let source_path = dir.join("Foo.Update.m1scr");
    fs::write(&project_path, PROJECT).unwrap();
    fs::write(&config_path, EMPTY_CONFIG).unwrap();
    fs::write(&source_path, SOURCE).unwrap();

    let mut project = Project::load(&project_path)
        .unwrap()
        .with_config(&config_path)
        .unwrap();
    let scripts = parsed_source();
    let library = project_check::check(
        Some(&mut project),
        &scripts,
        &[SourceInput::at_path(&source_path, SOURCE)],
        &ProjectCheckOptions::default(),
    );
    let library_source = code_counts(
        library.sources[0]
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str()),
    );
    let library_project = code_counts(
        library
            .project_diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str()),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_m1-typecheck"))
        .args([
            "--format",
            "json",
            "--project",
            project_path.to_str().unwrap(),
            "--config",
            config_path.to_str().unwrap(),
            source_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let cli_source = code_counts(
        json["files"][0]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["code"].as_str().unwrap()),
    );
    let cli_project = code_counts(
        json["project"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["code"].as_str().unwrap()),
    );
    assert_eq!(cli_source, library_source);
    assert_eq!(cli_project, library_project);
    let _ = fs::remove_dir_all(dir);
}
