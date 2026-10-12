use m1_typecheck::{
    diagnostics::TypeCode, parsed::parse_all, project::Project, rules::check_script,
};
use std::path::Path;

/// Bodies for the fixture's functions. `Bool` assigns the `Output` channel, so
/// calls to it are restricted inside conditions and call arguments.
const WRITER: &[(&str, &str)] = &[("Test.Bool.m1scr", "Output = 1.0;\nOut = true;\n")];

fn project(bodies: &[(&str, &str)]) -> Project {
    let mut project = Project::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/return_slots.m1prj"),
    )
    .unwrap();
    let scripts = parse_all(
        &bodies
            .iter()
            .map(|(name, source)| (name.to_string(), source.to_string()))
            .collect::<Vec<_>>(),
    );
    project.infer_channel_writes(&scripts);
    project
}

fn codes_with(bodies: &[(&str, &str)], source: &str) -> Vec<TypeCode> {
    check_script(&project(bodies), Path::new("Test.Single.m1scr"), source)
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .filter(|code| matches!(code, TypeCode::T116 | TypeCode::T117))
        .collect()
}

fn codes(source: &str) -> Vec<TypeCode> {
    codes_with(WRITER, source)
}

#[test]
fn discarded_direct_or_parenthesized_user_results_are_errors() {
    for source in ["Bool();", "(Bool());"] {
        assert_eq!(codes(source), [TypeCode::T116]);
    }
}

#[test]
fn discarded_results_are_errors_even_when_the_callee_writes_no_channel() {
    let pure = [("Test.Bool.m1scr", "Out = true;\n")];
    assert_eq!(codes_with(&pure, "Bool();"), [TypeCode::T116]);
}

#[test]
fn local_result_can_be_used_in_a_condition() {
    assert!(codes("local result = Bool(); if (result) { }").is_empty());
}

#[test]
fn user_calls_in_conditions_are_errors() {
    for source in [
        "if (Bool()) { }",
        "if (not Bool()) { }",
        "if (Bool() and true) { }",
    ] {
        assert_eq!(codes(source), [TypeCode::T117]);
    }
}

#[test]
fn user_calls_inside_other_call_arguments_are_errors() {
    assert_eq!(codes("System.Unused(Bool());"), [TypeCode::T117]);
}

#[test]
fn calls_to_functions_without_channel_writes_are_allowed_in_conditions_and_arguments() {
    let pure = [("Test.Bool.m1scr", "Out = true;\n")];
    for source in ["if (Bool()) { }", "System.Unused(Bool());"] {
        assert!(codes_with(&pure, source).is_empty(), "{source}");
    }
}

#[test]
fn channel_writes_reached_through_another_user_function_count() {
    let indirect = [
        ("Test.Bool.m1scr", "Helper();\nOut = true;\n"),
        ("Test.Helper.m1scr", "Output = 1.0;\n"),
    ];
    assert_eq!(codes_with(&indirect, "if (Bool()) { }"), [TypeCode::T117]);
}

#[test]
fn without_project_wide_inference_condition_calls_stay_silent() {
    let project = Project::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/return_slots.m1prj"),
    )
    .unwrap();
    let codes: Vec<_> = check_script(&project, Path::new("Test.Single.m1scr"), "if (Bool()) { }")
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .filter(|code| *code == TypeCode::T117)
        .collect();
    assert!(codes.is_empty());
}

#[test]
fn calls_in_branch_bodies_are_allowed_when_their_result_is_consumed() {
    assert!(codes("if (true) { local result = Bool(); if (result) { } }").is_empty());
}

#[test]
fn unknown_multiple_void_and_library_returns_stay_outside_this_rule() {
    assert!(codes("Unknown(); Multi(); Void(); Calculate.Max(1.0, 2.0);").is_empty());
}
