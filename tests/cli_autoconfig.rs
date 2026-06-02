//! The CLI auto-discovers a sibling `parameters.m1cfg` (mirroring the LSP), so
//! parameter value types from the config are applied without an explicit
//! `--config`. Proven end-to-end: a `u16` parameter (typed only via the cfg)
//! assigned a float triggers T030, which stays silent when the cfg is absent.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

// Types Root.Foo.Gain.Value as u16 (Unsigned). Nothing in the .m1prj carries a
// value type for it, so this is the only source.
const CONFIG: &str = r#"<?xml version="1.0"?>
<Configuration>
 <Group Name="">
  <Parameter Name="Root.Foo.Gain.Value"><Cell Type="u16" Unit="ratio"><![CDATA[1]]></Cell></Parameter>
 </Group>
</Configuration>
"#;

// Assigning a float to the u16 parameter is a type mismatch (T030) — but only if
// the parameter's type is known, which requires the cfg.
const SCRIPT: &str = "Gain.Value = 1.5;\n";

fn setup(dir: &Path, with_cfg: bool) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("Project.m1prj"), PROJECT).unwrap();
    if with_cfg {
        fs::write(dir.join("parameters.m1cfg"), CONFIG).unwrap();
    }
    let script = dir.join("Foo Update.m1scr");
    fs::write(&script, SCRIPT).unwrap();
    script
}

fn run(script: &Path) -> String {
    let out = Command::new(bin()).arg(script).output().unwrap();
    let mut s = String::from_utf8(out.stdout).unwrap();
    s.push_str(&String::from_utf8(out.stderr).unwrap());
    s
}

#[test]
fn auto_discovers_sibling_m1cfg_for_param_types() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("autoconfig_with");
    let script = setup(&dir, true);
    let out = run(&script);
    assert!(
        out.contains("T030"),
        "expected T030 from the cfg-typed parameter, got:\n{out}"
    );
}

#[test]
fn without_m1cfg_the_param_type_is_unknown() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("autoconfig_without");
    let script = setup(&dir, false);
    let out = run(&script);
    assert!(
        !out.contains("T030"),
        "without a cfg the parameter type is Unknown — T030 should not fire, got:\n{out}"
    );
}
