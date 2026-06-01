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
