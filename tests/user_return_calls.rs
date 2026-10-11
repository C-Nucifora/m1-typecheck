use m1_typecheck::{diagnostics::TypeCode, project::Project, rules::check_script};
use std::path::Path;

fn codes(source: &str) -> Vec<TypeCode> {
    let project = Project::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/return_slots.m1prj"),
    )
    .unwrap();
    check_script(&project, Path::new("Test.Single.m1scr"), source)
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .filter(|code| matches!(code, TypeCode::T114 | TypeCode::T115))
        .collect()
}

#[test]
fn discarded_direct_or_parenthesized_user_results_are_errors() {
    for source in ["Bool();", "(Bool());"] {
        assert_eq!(codes(source), [TypeCode::T114]);
    }
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
        assert_eq!(codes(source), [TypeCode::T115]);
    }
}

#[test]
fn user_calls_inside_other_call_arguments_are_errors() {
    assert_eq!(codes("System.Unused(Bool());"), [TypeCode::T115]);
}

#[test]
fn calls_in_branch_bodies_are_allowed_when_their_result_is_consumed() {
    assert!(codes("if (true) { local result = Bool(); if (result) { } }").is_empty());
}

#[test]
fn unknown_multiple_void_and_library_returns_stay_outside_this_rule() {
    assert!(codes("Unknown(); Multi(); Void(); Calculate.Max(1.0, 2.0);").is_empty());
}
