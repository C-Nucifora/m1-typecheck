use m1_typecheck::{
    compatibility::BuildVersion, diagnostics::TypeCode, parsed::parse_all, project::Project,
    rules::check_script,
};
use std::path::Path;

fn xml(version: &str, signature: &str) -> String {
    format!(
        r#"<MoTeCM1BuildSession ProductVersion="{version}"><Project><ComponentStream><List>
<Component Classname="BuiltIn.GroupCompound" Name="Root.Test"/>
<Component Classname="BuiltIn.FuncUserParam" Name="Root.Test.Value" Filename="Test.Value.m1scr">{signature}</Component>
</List></ComponentStream></Project></MoTeCM1BuildSession>"#
    )
}
const RETURNS: &str = r#"<Signature><Params/><Returns><Return Type="bool"/></Returns></Signature>"#;
const LEGACY: &str = r#"<Signature ReturnType="bool"><Params/></Signature>"#;

#[test]
fn version_selection_changes_return_checks_and_can_override_metadata() {
    let text = xml("1.4.4.981", RETURNS);
    let stable = Project::from_xml(&text).unwrap();
    let beta = Project::from_xml_for_build(&text, BuildVersion::Beta145556).unwrap();
    assert_eq!(stable.build_version(), BuildVersion::Stable144981);
    assert!(
        stable
            .symbols()
            .get("Root.Test.Value")
            .unwrap()
            .declared_returns
            .is_empty()
    );
    assert_eq!(
        beta.symbols()
            .get("Root.Test.Value")
            .unwrap()
            .declared_returns
            .len(),
        1
    );
    let check = |p: &Project| {
        check_script(p, Path::new("Test.Value.m1scr"), "local x = 1;")
            .diagnostics
            .iter()
            .any(|d| d.code == TypeCode::T099)
    };
    assert!(!check(&stable));
    assert!(check(&beta));
    // Loading another profile must not change an already loaded project.
    assert!(!check(&stable));
}

#[test]
fn legacy_scalar_contracts_work_in_both_profiles() {
    for version in [BuildVersion::Stable144981, BuildVersion::Beta145556] {
        let p = Project::from_xml_for_build(&xml("", LEGACY), version).unwrap();
        let diagnostics =
            check_script(&p, Path::new("Test.Value.m1scr"), "local x = 1;").diagnostics;
        let missing = diagnostics
            .iter()
            .find(|d| d.code == TypeCode::T099)
            .unwrap();
        let hint = match version {
            BuildVersion::Stable144981 => "must assign a return value",
            BuildVersion::Beta145556 => "is not assigned",
        };
        assert!(missing.inner.message.contains(hint));
        assert!(
            check_script(&p, Path::new("Test.Value.m1scr"), "local x = 1;")
                .diagnostics
                .iter()
                .any(|d| d.code == TypeCode::T099)
        );
    }
}

#[test]
fn stable_does_not_infer_an_ignored_new_signature_from_the_body() {
    let mut p = Project::from_xml(&xml("1.4.4.981", RETURNS)).unwrap();
    p.infer_return_types(&parse_all(&[(
        "Test.Value.m1scr".into(),
        "Out = true;".into(),
    )]));
    assert_eq!(
        p.symbols().get("Root.Test.Value").unwrap().return_type,
        None
    );
}

#[test]
fn beta_metadata_and_unknown_metadata_have_documented_defaults() {
    assert_eq!(
        Project::from_xml(&xml("1.4.5.556", RETURNS))
            .unwrap()
            .build_version(),
        BuildVersion::Beta145556
    );
    assert_eq!(
        Project::from_xml(&xml("future", RETURNS))
            .unwrap()
            .build_version(),
        BuildVersion::default()
    );
    assert!("future".parse::<BuildVersion>().is_err());
    assert_eq!(
        "01.04.04.0981".parse::<BuildVersion>().unwrap().to_string(),
        "1.4.4.0981"
    );
}

#[test]
fn cli_override_is_reported_and_rejects_unknown_versions() {
    use std::process::Command;
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/return_slots.m1prj");
    let output = Command::new(env!("CARGO_BIN_EXE_m1-typecheck"))
        .args([
            "--project",
            project.to_str().unwrap(),
            "--m1-build-version",
            "1.4.4.0981",
            "--completeness",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["completeness"]["m1_build_version"], "1.4.4.0981");
    assert_eq!(
        value["completeness"]["catalogue_target"],
        "m1-build-2026-06"
    );
    let invalid = Command::new(env!("CARGO_BIN_EXE_m1-typecheck"))
        .args(["--m1-build-version", "future", "--rules"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unsupported M1 Build version"));
}

#[test]
fn x_axis_index_return_type_changes_with_the_compiler_profile() {
    let table = r#"<Component Classname="BuiltIn.Table" Name="Root.Test.Map"/><Component Classname="BuiltIn.Channel" Name="Root.Test.Input"><Props Type="f32"/></Component>"#;
    for (version, expected) in [
        (BuildVersion::Stable144981, true),
        (BuildVersion::Beta145556, false),
    ] {
        let text = xml("", LEGACY).replace("</List>", &format!("{table}</List>"));
        let p = Project::from_xml_for_build(&text, version).unwrap();
        let check = |source: &str| {
            check_script(&p, Path::new("Test.Value.m1scr"), source)
                .diagnostics
                .iter()
                .any(|d| d.code == TypeCode::T002)
        };
        assert_eq!(
            check("local site = Map.XAxisIndex(Input); if (site eq 0) { }"),
            expected
        );
        assert!(check(
            "local site = Map.XAxisIndex(Input); if (site eq 0.0) { }"
        ));
        assert!(!check(
            "local site = Missing.XAxisIndex(Input); if (site eq 0) { }"
        ));
        assert!(!check("local site = Map.XAxisIndex(); if (site eq 0) { }"));
        assert!(!check(
            "local site = Map.XAxisIndex(Unknown); if (site eq 0) { }"
        ));
        assert!(!check(
            "local site = Input.XAxisIndex(Input); if (site eq 0) { }"
        ));
        assert!(!check(
            "local site = Map.YAxisIndex(Input); if (site eq 0) { }"
        ));
    }
}
