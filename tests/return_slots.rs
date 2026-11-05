use m1_typecheck::{
    diagnostics::TypeCode, project::Project, rules::check_script, types::ValueType,
};
use std::path::Path;

fn project() -> Project {
    Project::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/return_slots.m1prj"))
        .unwrap()
}

#[test]
fn parses_ordered_return_slots_without_scalarizing_multiple_results() {
    let p = project();
    let single = p.symbols().get("Root.Test.Single").unwrap();
    assert_eq!(single.return_type, Some(ValueType::Float));
    assert_eq!(single.declared_returns.len(), 1);
    let multi = p.symbols().get("Root.Test.Multi").unwrap();
    assert_eq!(multi.return_type, None);
    assert_eq!(
        multi
            .declared_returns
            .iter()
            .map(|r| (r.name.as_deref(), r.value_type))
            .collect::<Vec<_>>(),
        vec![
            (Some("Value"), ValueType::Float),
            (Some("Valid"), ValueType::Boolean)
        ]
    );
}

#[test]
fn unknown_return_types_are_retained_without_guessing() {
    let p = project();
    let s = p.symbols().get("Root.Test.Unknown").unwrap();
    assert_eq!(s.return_type, None);
    assert_eq!(s.declared_returns[0].declared_type, "Firmware.Custom");
    assert_eq!(s.declared_returns[0].value_type, ValueType::Unknown);
}

#[test]
fn preserves_legacy_and_void_signatures() {
    let p = project();
    let s = p.symbols().get("Root.Test.Legacy").unwrap();
    assert_eq!(s.return_type, Some(ValueType::Integer));
    assert_eq!(s.declared_returns[0].declared_type, "s32");
    assert!(
        p.symbols()
            .get("Root.Test.Void")
            .unwrap()
            .declared_returns
            .is_empty()
    );
}

#[test]
fn new_single_return_layout_participates_in_assignment_validation() {
    let p = project();
    let missing = check_script(&p, Path::new("Test.Single.m1scr"), "local x = 1;\n");
    assert!(missing.diagnostics.iter().any(|d| d.code == TypeCode::T099));
    let assigned = check_script(&p, Path::new("Test.Single.m1scr"), "Out = 1.0;\n");
    assert!(
        !assigned
            .diagnostics
            .iter()
            .any(|d| d.code == TypeCode::T099)
    );
}

#[test]
fn scalar_inference_cannot_overwrite_an_explicit_multi_or_unknown_contract() {
    let mut p = project();
    let scripts = m1_typecheck::parsed::parse_all(&[
        ("Test.Multi.m1scr".into(), "Out = 1.0;\n".into()),
        ("Test.Unknown.m1scr".into(), "Out = 1.0;\n".into()),
    ]);
    p.infer_return_types(&scripts);
    assert_eq!(
        p.symbols().get("Root.Test.Multi").unwrap().return_type,
        None
    );
    assert_eq!(
        p.symbols().get("Root.Test.Unknown").unwrap().return_type,
        None
    );
}

#[test]
fn mixed_return_layouts_preserve_legacy_singleton_compatibility() {
    let xml = r#"<MoTeCM1BuildSession><Project><ComponentStream><List>
      <Component Classname="BuiltIn.FuncUserParam" Name="Root.A">
        <Signature ReturnType="f32"><Returns><Return Type="bool"/></Returns></Signature>
      </Component>
      <Component Classname="BuiltIn.FuncUserParam" Name="Root.B">
        <Signature ReturnType="f32"><Returns><Return Name="Value" Type="bool"/></Returns></Signature>
      </Component>
      <Component Classname="BuiltIn.FuncUserParam" Name="Root.C">
        <Signature><Returns><Return Name="Absent"/><Return Type="bool"/></Returns></Signature>
      </Component>
    </List></ComponentStream></Project></MoTeCM1BuildSession>"#;
    let p = m1_typecheck::symbols::m1prj::parse(xml).unwrap();
    assert_eq!(
        p.table.get("Root.A").unwrap().return_type,
        Some(ValueType::Float)
    );
    assert_eq!(
        p.table.get("Root.B").unwrap().return_type,
        Some(ValueType::Boolean)
    );
    let c = p.table.get("Root.C").unwrap();
    assert_eq!(c.declared_returns.len(), 1);
    assert_eq!(c.return_type, Some(ValueType::Boolean));
}
