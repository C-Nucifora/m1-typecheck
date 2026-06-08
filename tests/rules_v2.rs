use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;
use std::path::Path;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("enums.m1prj"))
        .unwrap()
        .with_config(&dir.join("enums.m1cfg"))
        .unwrap()
}

fn codes(p: &Project, src: &str) -> Vec<TypeCode> {
    check_script(p, Path::new("Foo Update.m1scr"), src)
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn t020_flags_non_member_typed_path() {
    let p = proj();
    // "Drive State.Nope" has the enum-type head but no such member.
    assert!(codes(&p, "driveMode = Drive State.Nope;\n").contains(&TypeCode::T020));
}

#[test]
fn t020_no_flag_for_real_member() {
    let p = proj();
    assert!(!codes(&p, "driveMode = Drive State.Idle;\n").contains(&TypeCode::T020));
}

#[test]
fn t021_flags_enum_vs_integer() {
    let p = proj();
    // SwitchMode.Value is enum; comparing to integer literal 1.
    assert!(codes(&p, "if (SwitchMode.Value eq 1) {\n}\n").contains(&TypeCode::T021));
}

#[test]
fn t021_no_flag_enum_vs_member() {
    let p = proj();
    assert!(
        !codes(&p, "if (SwitchMode.Value eq Switch State.On) {\n}\n").contains(&TypeCode::T021)
    );
}

#[test]
fn t002_not_fired_for_enum_vs_float_literal() {
    // `SwitchMode.Value eq 1.0` mixes an enum with a number: the genuine issue is
    // T021 (comparing an enum to a number), NOT T002 (float-equality precision).
    // An enum is a discrete enumerator, so there is no float-precision hazard —
    // T002 firing here is a spurious duplicate. T021 must still fire.
    let p = proj();
    let got = codes(&p, "if (SwitchMode.Value eq 1.0) {\n}\n");
    assert!(
        !got.contains(&TypeCode::T002),
        "T002 must not fire when an operand is an enum: {got:?}"
    );
    assert!(
        got.contains(&TypeCode::T021),
        "T021 should still flag the enum-vs-number comparison: {got:?}"
    );
}

#[test]
fn t002_still_fires_for_two_floats() {
    // Guard against over-suppression: a real float == float comparison must still
    // be flagged when no enum is involved.
    let p = proj();
    let got = codes(&p, "local a = 1.5;\nlocal b = 2.5;\nif (a eq b) {\n}\n");
    assert!(
        got.contains(&TypeCode::T002),
        "T002 must still fire for a genuine float == float: {got:?}"
    );
}

#[test]
fn t021_no_flag_int_vs_int() {
    let p = proj();
    assert!(!codes(&p, "local iX = 1;\nif (iX eq 2) {\n}\n").contains(&TypeCode::T021));
}

#[test]
fn t021_flags_firmware_enum_vs_integer() {
    // `fwMode` is typed `MoTeC Types.Mode Enumeration` (a firmware enum). Even
    // though its members are unknown, comparing the enum to a number is wrong
    // regardless of members, so T021 still fires (#104).
    let p = proj();
    assert!(codes(&p, "if (fwMode eq 1) {\n}\n").contains(&TypeCode::T021));
}

#[test]
fn t020_no_flag_on_open_firmware_enum_member() {
    // A firmware enum's member list is not in the project, so any
    // `Mode Enumeration.<X>` is a member we cannot disprove — T020 must NOT
    // fire (it would be a false positive on valid firmware-enum usage) (#104).
    let p = proj();
    assert!(
        !codes(&p, "fwMode = Mode Enumeration.Whatever;\n").contains(&TypeCode::T020),
        "T020 must be suppressed for open (firmware) enums"
    );
}

#[test]
fn t030_flags_enum_member_into_wrong_enum() {
    let p = proj();
    // SwitchMode.Value is Switch State; assigning a Drive State member is a mismatch.
    assert!(codes(&p, "SwitchMode.Value = Drive State.Idle;\n").contains(&TypeCode::T030));
}

#[test]
fn t030_no_flag_same_enum() {
    let p = proj();
    assert!(!codes(&p, "SwitchMode.Value = Switch State.On;\n").contains(&TypeCode::T030));
}

#[test]
fn t030_no_flag_unknown_target() {
    let p = proj();
    // driveMode has no known type -> silent.
    assert!(!codes(&p, "driveMode = 1.0;\n").contains(&TypeCode::T030));
}

// ---- T070 when-is-exhaustive --------------------------------------------
// `SwitchMode.Value` is enum `Switch State` with members {Off, On}.

#[test]
fn t070_flags_missing_enumerator() {
    let p = proj();
    let src = "when (SwitchMode.Value) {\nis (Off) {\n}\n}\n";
    assert!(codes(&p, src).contains(&TypeCode::T070));
}

#[test]
fn t070_no_flag_when_all_covered() {
    let p = proj();
    let src = "when (SwitchMode.Value) {\nis (Off) {\n}\nis (On) {\n}\n}\n";
    assert!(!codes(&p, src).contains(&TypeCode::T070));
}

#[test]
fn t070_no_flag_when_all_covered_via_or() {
    let p = proj();
    let src = "when (SwitchMode.Value) {\nis (Off or On) {\n}\n}\n";
    assert!(!codes(&p, src).contains(&TypeCode::T070));
}

#[test]
fn t070_no_flag_non_enum_subject() {
    let p = proj();
    // `gain` is an f32 channel, not an enum -> rule stays silent.
    let src = "when (gain) {\nis (Off) {\n}\n}\n";
    assert!(!codes(&p, src).contains(&TypeCode::T070));
}

#[test]
fn t070_no_flag_with_catch_all_arm() {
    let p = proj();
    // An arm naming a non-member label acts as a default/catch-all -> exhaustive.
    let src = "when (SwitchMode.Value) {\nis (Off) {\n}\nis (Anything) {\n}\n}\n";
    assert!(!codes(&p, src).contains(&TypeCode::T070));
}

// NOTE (deviation from plan): the `LHS is (Member)` clause that the plan's
// Trigger 2 targeted is NOT valid syntax in the tree-sitter-m1 grammar — `is`
// always parses as an ERROR node, and the runner short-circuits on syntax
// errors, so the clause can never reach a rule. It also does not appear in the
// real m1-example corpus (every `is` there is inside a comment). T020 therefore fires
// only on the corpus-real typed-member-path idiom `<EnumType>.<Member>`.
