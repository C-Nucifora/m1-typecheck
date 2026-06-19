use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;
use std::path::Path;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("enums.m1prj")).unwrap()
}

fn codes(p: &Project, src: &str) -> Vec<TypeCode> {
    check_script(p, Path::new("Foo Update.m1scr"), src)
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn t040_flags_unconditional_double_assignment() {
    let p = proj();
    let src = "driveMode = Drive State.Idle;\ndriveMode = Drive State.Ready To Drive;\n";
    assert!(codes(&p, src).contains(&TypeCode::T040));
}

#[test]
fn t040_points_at_first_write() {
    // The diagnostic should land on the earlier (first) conflicting write, not
    // the second. Regression for #15.
    let p = proj();
    let src = "driveMode = Drive State.Idle;\ndriveMode = Drive State.Ready To Drive;\n";
    let first_line_end = src.find('\n').unwrap();
    let d = check_script(&p, Path::new("Foo Update.m1scr"), src)
        .diagnostics
        .into_iter()
        .find(|d| d.code == TypeCode::T040)
        .expect("expected a T040 diagnostic");
    assert!(
        d.inner.byte_range.start < first_line_end,
        "T040 should point at the first write (line 0), got byte {}",
        d.inner.byte_range.start
    );
}

#[test]
fn t040_no_flag_mutually_exclusive_branches() {
    let p = proj();
    let src = "if (gain > 1) {\ndriveMode = Drive State.Idle;\n} else {\ndriveMode = Drive State.Ready To Drive;\n}\n";
    assert!(!codes(&p, src).contains(&TypeCode::T040));
}

#[test]
fn t040_flags_unconditional_plus_conditional() {
    let p = proj();
    let src = "driveMode = Drive State.Idle;\nif (gain > 1) {\ndriveMode = Drive State.Ready To Drive;\n}\n";
    assert!(codes(&p, src).contains(&TypeCode::T040));
}

#[test]
fn t040_no_flag_for_local() {
    let p = proj();
    let src = "local iX = 1;\niX = 2;\n";
    assert!(!codes(&p, src).contains(&TypeCode::T040));
}

#[test]
fn t040_no_flag_across_is_clauses() {
    // The same channel written in two different `is` clauses is fine — the
    // clauses are mutually exclusive (#20).
    let p = proj();
    let src = "when (driveMode) {\nis (Drive State.Idle) {\ndriveMode = Drive State.Ready To Drive;\n}\nis (Drive State.Ready To Drive) {\ndriveMode = Drive State.Idle;\n}\n}\n";
    assert!(
        !codes(&p, src).contains(&TypeCode::T040),
        "mutually-exclusive is clauses must not flag T040"
    );
}

#[test]
fn t040_is_an_error_not_a_warning() {
    // #242: M1 Build Error 1317 ("assigned more than once in the same code
    // path") FAILS the build. T040 was a Warning, so `tc_exit` stayed 0 and the
    // CLI green-lit code M1 Build rejects. T040 must be an Error — matching its
    // cross-function sibling T102, which already reports 1317 as an Error.
    let p = proj();
    let src = "driveMode = Drive State.Idle;\ndriveMode = Drive State.Ready To Drive;\n";
    let d = check_script(&p, Path::new("Foo Update.m1scr"), src)
        .diagnostics
        .into_iter()
        .find(|d| d.code == TypeCode::T040)
        .expect("expected a T040 diagnostic");
    assert_eq!(
        d.inner.severity,
        m1_core::Severity::Error,
        "T040 (M1 Build 1317) must be an Error so the CLI fails the build"
    );
}

#[test]
fn t040_flags_conditional_in_if_then_unconditional_after() {
    // #242 part 1: a write inside an `if` branch (with an `else if` arm that
    // does not write the channel) followed by an unconditional write *after*
    // the if — on the path through the branch the channel is written twice.
    let p = proj();
    let src = "if (gain > 1) {\ndriveMode = Drive State.Idle;\n} else if (gain > 2) {\ngain = 5;\n}\ndriveMode = Drive State.Ready To Drive;\n";
    assert!(codes(&p, src).contains(&TypeCode::T040));
}

#[test]
fn t040_flags_inside_if_plus_unconditional_within_is_clause() {
    // #242 part 1: the reporter's exact shape — within a `when … is` clause, a
    // conditional write inside an `if` plus an unconditional write at the end of
    // the same clause body.
    let p = proj();
    let src = "when (driveMode) {\nis (Drive State.Idle) {\nif (gain > 1) {\ndriveMode = Drive State.Ready To Drive;\n} else if (gain > 2) {\ngain = 5;\n}\ndriveMode = Drive State.Idle;\n}\n}\n";
    assert!(codes(&p, src).contains(&TypeCode::T040));
}

#[test]
fn t040_flags_double_assignment_in_one_is_clause() {
    // Two writes to the same channel within a single `is` clause body still run
    // sequentially → flagged.
    let p = proj();
    let src = "when (driveMode) {\nis (Drive State.Idle) {\ndriveMode = Drive State.Ready To Drive;\ndriveMode = Drive State.Idle;\n}\n}\n";
    assert!(
        codes(&p, src).contains(&TypeCode::T040),
        "two writes in one is clause must flag T040"
    );
}
