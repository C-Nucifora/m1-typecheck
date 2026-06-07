use m1_typecheck::project::Project;
use std::path::Path;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("mini.m1prj"))
        .unwrap()
        .with_config(&dir.join("mini.m1cfg"))
        .unwrap()
}

#[test]
fn loads_table_and_config() {
    let p = proj();
    assert!(p.symbols().get("Root.Foo.Speed").is_some());
    assert_eq!(
        p.symbols()
            .get("Root.Foo.Gain.Value")
            .unwrap()
            .unit
            .as_deref(),
        Some("ratio")
    );
}

#[test]
fn parameter_without_type_or_qty_inferred_from_leaf_name() {
    // #106: a parameter with no inline Type and no .m1cfg entry, whose leaf is a
    // float-quantity word, is inferred Float by convention (recovering T030/T003).
    let dir = std::env::temp_dir().join(format!("m1tc_param_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let prj = dir.join("Project.m1prj");
    std::fs::write(
        &prj,
        r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Demo"/>
  <Component Classname="BuiltIn.Parameter" Name="Root.Demo.Widget Voltage"><Props/></Component>
  <Component Classname="BuiltIn.Parameter" Name="Root.Demo.Widget Count"><Props/></Component>
</Project>"#,
    )
    .unwrap();
    let p = Project::load(&prj).unwrap();
    use m1_typecheck::types::ValueType;
    assert_eq!(
        p.symbols()
            .get("Root.Demo.Widget Voltage")
            .unwrap()
            .value_type,
        ValueType::Float,
        "leaf ending in `Voltage` -> Float"
    );
    // A non-quantity leaf stays Unknown (no false positive).
    assert_eq!(
        p.symbols()
            .get("Root.Demo.Widget Count")
            .unwrap()
            .value_type,
        ValueType::Unknown
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn table_meta_from_m1prj_axis_without_cfg() {
    // #165: a Table's shape (axis breakpoint counts) is encoded in the .m1prj
    // `<Axis><X MaxSites/><Y MaxSites/></Axis>` and must populate `table_meta`
    // even when no `.m1cfg` calibration export is loaded.
    let dir = std::env::temp_dir().join(format!("m1tc_tbl_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let prj = dir.join("Project.m1prj");
    std::fs::write(
        &prj,
        r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Table" Name="Root.Demo.Fuel Map">
    <Axis><X MaxSites="11"/><Y MaxSites="7"/></Axis>
  </Component>
</Project>"#,
    )
    .unwrap();
    let p = Project::load(&prj).unwrap();
    let sym = p
        .symbols()
        .get("Root.Demo.Fuel Map")
        .expect("table symbol present");
    let meta = sym
        .table_meta
        .as_ref()
        .expect("table_meta derived from .m1prj <Axis>");
    let sizes: Vec<u32> = meta.axes.iter().map(|a| a.size).collect();
    assert_eq!(sizes, vec![11, 7], "2-D shape 11 x 7");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opaque_roots_include_modules_and_builtins() {
    let p = proj();
    assert!(p.is_opaque_root("MoTeC Comms")); // from module sets
    assert!(p.is_opaque_root("Calculate")); // static builtin
    assert!(p.is_opaque_root("This"));
    assert!(!p.is_opaque_root("Root")); // Root is a project group, not opaque
}

#[test]
fn group_for_script() {
    let p = proj();
    assert_eq!(
        p.group_for_script("Foo Update.m1scr").as_deref(),
        Some("Root.Foo")
    );
    assert_eq!(p.group_for_script("Unknown.m1scr"), None);
}
