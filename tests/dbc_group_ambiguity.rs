//! Native M1 Build Error 1334: DBC globals compete with visible project groups.
//! Regression for the BMS/SBG/TTP examples in m1-tools#68.
use m1_core::Severity;
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::resolve::{Resolution, Scope, resolve};
use m1_typecheck::rules::check_script;
use std::collections::HashMap;
use std::path::Path;

const XML: &str = include_str!("fixtures/dbc_group_ambiguity.m1prj");

fn ambiguities(project: &Project, file: &str, source: &str) -> Vec<String> {
    check_script(project, Path::new(file), source)
        .diagnostics
        .into_iter()
        .filter(|d| d.code == TypeCode::T103)
        .map(|d| {
            assert_eq!(d.inner.severity, Severity::Error);
            d.inner.message
        })
        .collect()
}

#[test]
fn native_group_dbc_collisions_are_errors_even_when_only_one_tail_exists() {
    for xml in [XML.to_string(), XML.replace("Name=\"DBC.", "Name=\"")] {
        let project = Project::from_xml(&xml).unwrap();
        let hits = ambiguities(
            &project,
            "CAN.Update.m1scr",
            "local timeout = BMS.Timeout;\nBMS.Contactors OK = True;\nSBG.Init(1);\nTTP.Init(2);\n",
        );
        assert_eq!(hits.len(), 3, "one error per colliding head: {hits:?}");
        for name in ["BMS", "SBG", "TTP"] {
            assert!(hits.iter().any(|m| m.contains(&format!("`{name}`"))
                && m.contains("Error 1334")
                && m.contains(&format!("Root.CAN.{name}"))
                && m.contains(&format!("DBC.{name}"))));
        }
    }
}

#[test]
fn explicit_project_and_database_paths_remain_unambiguous_and_resolve_as_before() {
    let project = Project::from_xml(XML).unwrap();
    assert!(ambiguities(
        &project,
        "CAN.Update.m1scr",
        "local x = Root.CAN.BMS.Timeout;\nThis.BMS.Contactors OK = True;\nDBC.BMS.Init(1);\nDBC.TTP.Init(2);\n",
    ).is_empty());
    let scope = Scope {
        locals: HashMap::new(),
        group: Some("Root.CAN".into()),
        project: Some(&project),
        fn_symbol: None,
    };
    assert!(matches!(resolve("Root.CAN.BMS.Timeout", &scope),
        Resolution::Symbol(s) if s.path == "Root.CAN.BMS.Timeout"));
    assert!(matches!(resolve("DBC.BMS", &scope),
        Resolution::Symbol(s) if s.path == "DBC.BMS"));
    assert!(matches!(
        resolve("DBC.BMS.Init", &scope),
        Resolution::Opaque
    ));
    assert!(matches!(
        resolve("UnmodelledFirmware.Unknown", &scope),
        Resolution::Opaque
    ));
}

#[test]
fn distant_groups_and_local_names_do_not_compete_with_database_roots() {
    let project = Project::from_xml(XML).unwrap();
    assert!(ambiguities(&project, "Other.Update.m1scr", "BMS.Init(1);\n").is_empty());
    assert!(
        ambiguities(
            &project,
            "CAN.Update.m1scr",
            "local BMS = 1;\nlocal x = BMS;\n"
        )
        .is_empty()
    );
}

#[test]
fn groups_without_dbc_metadata_are_not_guessed_to_be_ambiguous() {
    let project =
        Project::from_xml(&XML.replace("BuiltIn.CAN.DBC\"", "External.Unknown\"")).unwrap();
    assert!(ambiguities(&project, "CAN.Update.m1scr", "local x = BMS.Timeout;\n").is_empty());
}

#[test]
fn duplicate_database_aliases_report_once_and_library_roots_keep_precedence() {
    let xml = XML.replace(
        "</Project>",
        r#"
  <Component Classname="BuiltIn.CAN.DBC" Name="BMS"/>
  <Component Classname="BuiltIn.CAN.DBC" Name="DBC.Calculate"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.CAN.Calculate"/>
</Project>"#,
    );
    let project = Project::from_xml(&xml).unwrap();
    let hits = ambiguities(
        &project,
        "CAN.Update.m1scr",
        "local x = BMS.Timeout;\nlocal y = BMS.Timeout;\nlocal z = Calculate.Max(1, 2);\n",
    );
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].contains("`BMS`"));
}
