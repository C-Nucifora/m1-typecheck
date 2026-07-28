//! End-to-end regression for the EV-M1 DTI rear-inverter failure: a fresh CLI
//! run must auto-discover `parameters.m1cfg`, see Pole Pairs as `u32`, and reject
//! the mixed `Calculate.Max(1.0, Pole Pairs)` overload before M1 Build does.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_m1-typecheck")
}

const PROJECT: &str = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="EV" TargetHardware="ecu150">
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.CAN"/>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.CAN.DTI FSIC Rear"/>
    <Component Classname="BuiltIn.Parameter" Name="Root.CAN.DTI FSIC Rear.Pole Pairs">
     <Props Security="Calibration"/>
    </Component>
    <Component Classname="BuiltIn.FuncUser"
      Filename="CAN.Inverters Transcieve 200hz.m1scr"
      Name="Root.CAN.Inverters Transcieve 200hz"/>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

const SCRIPT: &str = "local fRearPolePairs = Calculate.Max(1.0, DTI FSIC Rear.Pole Pairs);\n";

fn setup(name: &str, cell_type: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let project_dir = root.join("UQR-EV").join("01.00");
    let scripts_dir = project_dir.join("Scripts");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&scripts_dir).unwrap();
    fs::write(project_dir.join("Project.m1prj"), PROJECT).unwrap();
    fs::write(
        root.join("parameters.m1cfg"),
        format!(
            r#"<?xml version="1.0"?>
<Configuration>
 <Group Name="">
  <Parameter Name="CAN.DTI FSIC Rear.Pole Pairs">
   <Cell Type="{cell_type}"><![CDATA[10]]></Cell>
  </Parameter>
 </Group>
</Configuration>
"#
        ),
    )
    .unwrap();
    let script = scripts_dir.join("CAN.Inverters Transcieve 200hz.m1scr");
    fs::write(&script, SCRIPT).unwrap();
    script
}

fn run(script: &Path) -> String {
    let out = Command::new(bin())
        .arg(script)
        .env_remove("M1_PROJECT")
        .env_remove("M1_CONFIG")
        .output()
        .unwrap();
    let mut text = String::from_utf8(out.stdout).unwrap();
    text.push_str(&String::from_utf8(out.stderr).unwrap());
    text
}

#[test]
fn cfg_typed_u32_parameter_rejects_mixed_calculate_max_overload() {
    let output = run(&setup("intrinsic_arg_u32", "u32"));
    assert!(
        output.contains("T065") && output.contains("Floating Point, Unsigned Integer"),
        "expected the EV-M1 overload failure to be diagnosed:\n{output}"
    );
}

#[test]
fn cfg_typed_f32_parameter_selects_floating_calculate_max_overload() {
    let output = run(&setup("intrinsic_arg_f32", "f32"));
    assert!(
        !output.contains("T065"),
        "the corrected EV-M1 parameter type should be accepted:\n{output}"
    );
}
