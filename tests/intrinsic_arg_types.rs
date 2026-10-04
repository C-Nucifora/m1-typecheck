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

fn receive_source(value: &str) -> String {
    format!(
        "local <Unsigned Integer> h = CanComms.RxOpenStandardBuffered(2, true);\n\
         local <Unsigned Integer> expected = 0x00;\n\
         local <boolean> received = CanComms.RxFindMessage(h, 0x500, 0, 0, {value});\n"
    )
}

#[test]
fn native_unsigned_receive_value_is_checked_with_and_without_project() {
    use m1_typecheck::diagnostics::TypeCode;
    use m1_typecheck::project::Project;
    use m1_typecheck::rules::{check_script, check_script_no_project};

    let project = Project::from_xml(PROJECT).unwrap();
    for value in ["0", "(0)", "-1"] {
        let source = receive_source(value);
        for result in [
            check_script_no_project(&source),
            check_script(&project, Path::new("Receive.m1scr"), &source),
        ] {
            let finding = result
                .diagnostics
                .iter()
                .find(|d| d.code == TypeCode::T065)
                .expect("native Error 1301 must produce an intrinsic argument finding");
            assert!(finding.inner.message.contains("argument `value`"));
            assert!(finding.inner.message.contains("requires Unsigned Integer"));
            assert!(finding.inner.message.contains("Integer supplied"));
        }
    }
    // Decimal zero does not receive a special conversion exception. The
    // unsigned literal and explicitly typed local express the native signature.
    for value in ["0x00", "expected", "Firmware.Value"] {
        let source = receive_source(value);
        for result in [
            check_script_no_project(&source),
            check_script(&project, Path::new("Receive.m1scr"), &source),
        ] {
            assert!(
                !result.diagnostics.iter().any(|d| d.code == TypeCode::T065),
                "valid or unknown receive argument was rejected: {value}"
            );
        }
    }
}

#[test]
fn both_documented_buffered_open_overloads_are_valid() {
    use m1_typecheck::diagnostics::TypeCode;
    use m1_typecheck::rules::check_script_with;

    let enabled = ["T064".to_string()].into();
    for source in [
        "local h = CanComms.RxOpenStandardBuffered(2, true);\n",
        "local h = CanComms.RxOpenStandardBuffered(2, 0x500, 0x00, true);\n",
    ] {
        let result = check_script_with(&enabled, None, None, source);
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| matches!(d.code, TypeCode::T064 | TypeCode::T065)),
            "documented overload was rejected: {source}"
        );
    }
    let wrong_types = check_script_with(
        &enabled,
        None,
        None,
        "local h = CanComms.RxOpenStandardBuffered(2, 0);\n",
    );
    assert!(
        wrong_types
            .diagnostics
            .iter()
            .any(|d| d.code == TypeCode::T065)
    );
}

#[test]
fn unsigned_receive_mismatch_reaches_cli() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("intrinsic_receive_unsigned");
    fs::create_dir_all(&root).unwrap();
    let script = root.join("Receive.m1scr");
    fs::write(&script, receive_source("0")).unwrap();
    let output = run(&script);
    assert!(output.contains("T065"), "missing CLI finding: {output}");
    assert!(output.contains("argument `value`"));
    assert!(output.contains("requires Unsigned Integer"));
}

#[test]
fn ordinary_library_numeric_assignment_conversions_remain_valid() {
    use m1_typecheck::diagnostics::TypeCode;
    use m1_typecheck::rules::check_script_no_project;

    // The manual p46 allows assignment conversions for ordinary arguments;
    // native RxFindMessage.value evidence must not make every unsigned slot strict.
    let result =
        check_script_no_project("local h = CanComms.RxOpenStandardBuffered(2, 0, 0, true);\n");
    assert!(!result.diagnostics.iter().any(|d| d.code == TypeCode::T065));
}
