//! Real MoTeC `parameters.m1cfg` exports name parameters WITHOUT the implicit
//! `Root.` group prefix that the project symbol table keys use (cfg
//! `Foo.Gain.Value` vs symbol `Root.Foo.Gain.Value`). `augment` must resolve
//! across that prefix, otherwise the entire cfg is silently dropped on real
//! projects. (An already-qualified name must still match — older/hand-written
//! cfgs and the crate's own fixtures use `Root.`-prefixed names.)

use m1_typecheck::project::Project;
use m1_typecheck::types::ValueType;
use std::fs;
use std::path::Path;

const PROJECT: &str = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="Demo" TargetHardware="ecu120">
  <DataTypes>
   <Type Name="Switch State" Storage="enum" Default="Off">
    <Enum Name="Off" ContainerOrder="0"/>
    <Enum Name="On" ContainerOrder="1"/>
   </Type>
  </DataTypes>
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Foo"/>
    <Component Classname="BuiltIn.Parameter" Name="Root.Foo.Gain.Value"><Props/></Component>
    <Component Classname="BuiltIn.Parameter" Name="Root.Foo.Mode"><Props/></Component>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

// Bare names — no `Root.` prefix, as a real MoTeC export writes them.
const CFG_BARE: &str = r#"<?xml version="1.0"?>
<Configuration>
 <Group Name="">
  <Parameter Name="Foo.Gain.Value"><Cell Type="u16" Unit="ratio"><![CDATA[1]]></Cell></Parameter>
  <Parameter Name="Foo.Mode"><Cell Type="enum"><![CDATA[On]]></Cell></Parameter>
 </Group>
</Configuration>
"#;

// Same content, but already `Root.`-qualified (must keep working).
const CFG_ROOTED: &str = r#"<?xml version="1.0"?>
<Configuration>
 <Group Name="">
  <Parameter Name="Root.Foo.Gain.Value"><Cell Type="u16" Unit="ratio"><![CDATA[1]]></Cell></Parameter>
 </Group>
</Configuration>
"#;

fn load(dir: &str, cfg: &str) -> Project {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(dir);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("Project.m1prj"), PROJECT).unwrap();
    fs::write(d.join("parameters.m1cfg"), cfg).unwrap();
    Project::load(&d.join("Project.m1prj"))
        .unwrap()
        .with_config(&d.join("parameters.m1cfg"))
        .unwrap()
}

#[test]
fn applies_cfg_with_bare_unprefixed_names() {
    let p = load("cfg_bare", CFG_BARE);
    let gain = p
        .symbols()
        .get("Root.Foo.Gain.Value")
        .expect("param symbol");
    assert!(
        matches!(gain.value_type, ValueType::Unsigned),
        "u16 cfg cell should type the param Unsigned, got {:?}",
        gain.value_type
    );
    assert_eq!(gain.unit.as_deref(), Some("ratio"));

    let mode = p.symbols().get("Root.Foo.Mode").expect("enum param symbol");
    assert!(
        matches!(mode.value_type, ValueType::Enum(_)),
        "enum cfg cell should associate the param with the enum type, got {:?}",
        mode.value_type
    );
}

#[test]
fn still_applies_already_rooted_names() {
    let p = load("cfg_rooted", CFG_ROOTED);
    let gain = p
        .symbols()
        .get("Root.Foo.Gain.Value")
        .expect("param symbol");
    assert!(matches!(gain.value_type, ValueType::Unsigned));
    assert_eq!(gain.unit.as_deref(), Some("ratio"));
}
