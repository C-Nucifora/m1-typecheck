use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use std::path::Path;

fn proj() -> Project {
    Project::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/naming.m1prj"))
        .unwrap()
}

fn load(fixture: &str) -> Project {
    Project::load(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(fixture),
    )
    .unwrap()
}

fn t071_messages(p: &Project) -> Vec<String> {
    p.audit()
        .iter()
        .filter(|d| d.code == TypeCode::T071)
        .map(|d| d.inner.message.clone())
        .collect()
}

#[test]
fn t050_flags_only_violators() {
    let p = proj();
    let msgs: Vec<String> = p
        .audit()
        .iter()
        .filter(|d| d.code == TypeCode::T050)
        .map(|d| d.inner.message.clone())
        .collect();
    let all = msgs.join("\n");
    // Violators present.
    assert!(
        all.contains("BrakePressure"),
        "channel UpperCamel should flag: {all}"
    );
    assert!(
        all.contains("brakeBias"),
        "parameter lowerCamel should flag"
    );
    assert!(all.contains("trackWidth"), "constant non-caps should flag");
    assert!(
        all.contains("CheckLimit"),
        "function UpperCamel should flag"
    );
    assert!(all.contains("bad state"), "enum with space should flag");
    // Conformers absent.
    assert!(
        !all.contains("`brakePressure`"),
        "conforming channel must not flag"
    );
    assert!(
        !all.contains("`BrakeBias`"),
        "conforming parameter must not flag"
    );
    assert!(!all.contains("`TRACKWIDTH`"));
    assert!(!all.contains("`checkLimit`"));
    assert!(!all.contains("`GoodState`"));
}

#[test]
fn t071_flags_case_collisions() {
    // naming.m1prj has pairs differing only by case (e.g. brakePressure /
    // BrakePressure, BrakeBias / brakeBias, TRACKWIDTH / trackWidth).
    let msgs = t071_messages(&proj()).join("\n");
    assert!(
        msgs.contains("Root.Inputs.BrakePressure") && msgs.contains("Root.Inputs.brakePressure"),
        "expected case-collision pair, got: {msgs}"
    );
    assert!(msgs.contains("differ only by case"));
}

#[test]
fn t071_flags_library_shadow() {
    // shadow.m1prj declares a channel named `Calculate` (a library object).
    let msgs = t071_messages(&load("shadow.m1prj")).join("\n");
    assert!(
        msgs.contains("shadows the library object `Calculate`"),
        "expected library-shadow warning, got: {msgs}"
    );
}

#[test]
fn t071_clean_project_has_none() {
    assert!(
        t071_messages(&load("clean.m1prj")).is_empty(),
        "clean project must produce no T071"
    );
}

fn t010_messages(p: &Project) -> Vec<String> {
    p.audit()
        .iter()
        .filter(|d| d.code == TypeCode::T010)
        .map(|d| d.inner.message.clone())
        .collect()
}

#[test]
fn t010_flags_can_signal_under_wrong_parent() {
    // classname_structure.m1prj nests one signal correctly (under a message) and
    // one incorrectly (directly under a group compound). Only the latter flags.
    let msgs = t010_messages(&load("classname_structure.m1prj")).join("\n");
    assert!(
        msgs.contains("Root.Loose.Rpm") && msgs.contains("expected parent class"),
        "expected a T010 for the misplaced CAN signal, got: {msgs}"
    );
    assert!(
        !msgs.contains("Root.Comms.Status.Speed"),
        "the correctly-nested signal must not flag: {msgs}"
    );
}

#[test]
fn t010_clean_project_has_none() {
    // The naming fixture has no CAN components, so the conservative schema is
    // silent — proving well-formed projects never see a T010.
    assert!(
        t010_messages(&proj()).is_empty(),
        "project without constrained classes must produce no T010"
    );
}

// ---- T087 type-restricted-to-locals (#149) --------------------------------

#[test]
fn t087_flags_bool_channels_and_parameters() {
    let p = load("type_restrictions.m1prj");
    let msgs: Vec<String> = p
        .audit()
        .iter()
        .filter(|d| d.code == TypeCode::T087)
        .map(|d| d.inner.message.clone())
        .collect();
    let all = msgs.join("\n");
    assert!(all.contains("boolFlag"), "bool channel must flag: {all}");
    assert!(all.contains("BoolParam"), "bool parameter must flag: {all}");
    assert!(!all.contains("fineFloat"), "f32 channel must not flag");
    assert!(!all.contains("fineInt"), "s32 channel must not flag");
    assert_eq!(msgs.len(), 2, "exactly the two bool symbols: {all}");
}

#[test]
fn project_diagnostics_carry_their_subject() {
    // #151: every project-level (zero-range) finding names the symbol it is
    // about, so [diagnostics] ignore_symbols can suppress it per-symbol.
    let p = proj();
    for d in p.audit() {
        assert!(
            d.subject.is_some(),
            "audit diagnostic without subject: {} {}",
            d.code.as_str(),
            d.inner.message
        );
    }
}
