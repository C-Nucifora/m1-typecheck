use m1_typecheck::{diagnostics::TypeCode, project::Project, rules::check_script};
use std::path::Path;

fn codes(function: &str, source: &str) -> Vec<TypeCode> {
    let project = Project::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/return_slots.m1prj"),
    )
    .unwrap();
    check_script(&project, Path::new(function), source)
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .filter(|code| matches!(code, TypeCode::T099 | TypeCode::T112 | TypeCode::T113))
        .collect()
}

#[test]
fn missing_output_retains_the_existing_diagnostic() {
    assert_eq!(codes("Test.Single.m1scr", "local x = 1;"), [TypeCode::T099]);
}

#[test]
fn straight_line_duplicate_output_is_an_error() {
    assert_eq!(
        codes("Test.Single.m1scr", "Out = 1.0; Out = 2.0;"),
        [TypeCode::T112]
    );
}

#[test]
fn partial_output_is_distinct_from_no_output() {
    assert_eq!(
        codes("Test.Single.m1scr", "if (true) { Out = 1.0; }"),
        [TypeCode::T113]
    );
}

#[test]
fn alternatives_are_exclusive_and_else_if_is_nested() {
    assert!(
        codes(
            "Test.Single.m1scr",
            "if (true) { Out = 1.0; } else if (false) { Out = 2.0; } else { Out = 3.0; }"
        )
        .is_empty()
    );
    assert_eq!(
        codes(
            "Test.Single.m1scr",
            "if (true) { Out = 1.0; } else if (false) { Out = 2.0; }"
        ),
        [TypeCode::T113]
    );
}

#[test]
fn later_write_does_not_repair_an_incomplete_conditional_join() {
    assert_eq!(
        codes("Test.Single.m1scr", "if (true) { Out = 1.0; } Out = 2.0;"),
        [TypeCode::T112, TypeCode::T113]
    );
}

#[test]
fn earlier_write_does_not_repair_an_incomplete_conditional_join() {
    assert_eq!(
        codes("Test.Single.m1scr", "Out = 1.0; if (true) { Out = 2.0; }"),
        [TypeCode::T112, TypeCode::T113]
    );
}

#[test]
fn legacy_scalar_returns_use_the_same_path_checks() {
    assert_eq!(
        codes("Test.Legacy.m1scr", "Out = 1; Out = 2;"),
        [TypeCode::T112]
    );
}

#[test]
fn unknown_multiple_and_void_contracts_stay_silent() {
    for function in ["Test.Unknown.m1scr", "Test.Multi.m1scr", "Test.Void.m1scr"] {
        assert!(codes(function, "if (true) { Out = 1.0; Out = 2.0; }").is_empty());
    }
}

#[test]
fn unsupported_output_paths_stay_silent() {
    for source in ["expand (N = 0 to 1) { Out = 1.0; }", "Out.Value = 1.0;"] {
        assert!(codes("Test.Single.m1scr", source).is_empty());
    }
}
