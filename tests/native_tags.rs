//! The type checker consumes the same native tag rules as m1-project.
use m1_typecheck::project::Project;
use m1_typecheck::project_check::{ProjectCheckOptions, check};

fn project(components: &str) -> String {
    format!(
        r#"<MoTeCM1BuildSession><Project><ComponentStream><List>
<Component Classname="BuiltIn.GroupCompound" Name="Root.Example"><Props><List.UserTags><Entry Value="Vehicle"/></List.UserTags></Props></Component>
{components}</List></ComponentStream></Project></MoTeCM1BuildSession>"#
    )
}

#[test]
fn native_state_and_io_rules_reach_default_pipeline_once() {
    let xml = project(
        r#"
<Component Classname="BuiltIn.Channel" Name="Root.Example.State"><Props Type="::This.Mode" Security="Tune"><List.UserTags><Entry Value="Diagnostic"/></List.UserTags></Props></Component>
<Component Classname="BuiltIn.IOResourceParameter" Name="Root.Example.Resource"><Props Security="Tune"><List.UserTags><Entry Value="Pin"/></List.UserTags></Props></Component>
<Component Classname="BuiltIn.IOResourceValueInput" Name="Root.Example.Input"><Props NameCreation="AutoParam" NameTarget="This.Value"><List.UserTags><Entry Value="Pin"/></List.UserTags></Props></Component>
<Component Classname="BuiltIn.IOResourceParameter" Name="Root.Example.Input.Value"><Props Security="Tune"/></Component>
"#,
    );
    let mut model = Project::from_xml(&xml).unwrap();
    let result = check(Some(&mut model), &[], &[], &ProjectCheckOptions::default());
    let tags: Vec<_> = result
        .project_diagnostics
        .iter()
        .filter(|d| d.code.as_str() == "T112")
        .collect();
    assert_eq!(tags.len(), 3, "{tags:?}");
    assert_eq!(
        tags.iter()
            .filter(|d| d.inner.message.contains("1647"))
            .count(),
        1
    );
    assert_eq!(
        tags.iter()
            .filter(|d| d.inner.message.contains("1140"))
            .count(),
        2
    );
    assert!(
        !result
            .project_diagnostics
            .iter()
            .any(|d| d.code.as_str() == "T092")
    );
}

#[test]
fn legal_and_unknown_objects_are_silent() {
    let xml = project(
        r#"
<Component Classname="BuiltIn.Channel" Name="Root.Example.State"><Props Type="::This.Mode"><List.UserTags><Entry Value="Normal"/></List.UserTags></Props></Component>
<Component Classname="BuiltIn.Channel" Name="Root.Example.NonEnum.State"><Props Type="f32"><List.UserTags><Entry Value="Diagnostic"/></List.UserTags></Props></Component>
<Component Classname="External.Module" Name="Root.Example.Opaque"/>
<Component Classname="BuiltIn.IOResourceValueInput" Name="Root.Example.Input"><Props NameCreation="AutoParam" NameTarget="This.Value"><List.UserTags><Entry Value="Setup"/><Entry Value="Input"/></List.UserTags></Props></Component>
<Component Classname="BuiltIn.IOResourceParameter" Name="Root.Example.Input.Value"><Props/></Component>
"#,
    );
    let model = Project::from_xml(&xml).unwrap();
    assert!(model.audit_tags().is_empty());
}

#[test]
fn selected_modules_resolve_effective_inherited_tags() {
    let xml = project(
        r#"<Component Classname="Test Module.Example" Name="Root.Example.Module"/>
<Component Classname="BuiltIn.Reference" Name="Root.Example.Module.Pin Choice"><Props TargetCreation="AutoParam" Target="This.Value"/></Component>"#,
    );
    let module = r#"<MoTecM1BuildModuleSet Name="Test Module"><Modules><ModuleStream><List><Module Base="BuiltIn.GroupCompound" Name="Group.Example"><ComponentStream><List>
<Component Classname="BuiltIn.GroupCompound" Name="Base"/>
<Component Classname="BuiltIn.Reference" Name="Base.Pin Choice"><Props TargetCreation="AutoChannel"><List.UserTags><Entry Value="Pin"/></List.UserTags></Props></Component>
</List></ComponentStream></Module></List></ModuleStream></Modules></MoTecM1BuildModuleSet>"#;
    assert!(Project::from_xml(&xml).unwrap().audit_tags().is_empty());
    let tags = Project::from_xml_with_modules(&xml, &[module])
        .unwrap()
        .audit_tags();
    assert_eq!(tags.len(), 1);
    assert!(tags[0].inner.message.contains("1140"));
    assert!(tags[0].inner.message.contains("Pin"));
}

#[test]
fn cli_exposes_native_tag_findings_and_filter_policy() {
    let dir = std::env::temp_dir().join(format!("m1_native_tags_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let project_file = dir.join("Project.m1prj");
    let script_file = dir.join("Update.m1scr");
    std::fs::write(&project_file, project(r#"<Component Classname="BuiltIn.IOResourceParameter" Name="Root.Example.Resource"><Props><List.UserTags><Entry Value="Pin"/></List.UserTags></Props></Component>"#)).unwrap();
    std::fs::write(&script_file, "").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_m1-typecheck"))
        .arg("--project")
        .arg(&project_file)
        .args(["--select", "T112"])
        .arg(&script_file)
        .output()
        .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("T112"), "{text}");
    assert!(text.contains("1140"), "{text}");
    assert_eq!(text.matches("warning[T112]").count(), 1, "{text}");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_m1-typecheck"))
        .arg("--project")
        .arg(&project_file)
        .args(["--ignore", "T112"])
        .arg(&script_file)
        .output()
        .unwrap();
    assert!(!String::from_utf8(output.stdout).unwrap().contains("T112"));
    std::fs::remove_dir_all(dir).unwrap();
}
