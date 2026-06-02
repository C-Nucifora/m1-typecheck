use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;
use std::path::Path;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("mini.m1prj")).unwrap()
}

fn codes(p: &Project, file: &str, src: &str) -> Vec<TypeCode> {
    check_script(p, Path::new(file), src)
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn write_to_project_rooted_miss_is_t031_not_t001() {
    let p = proj();
    // "Foo Update.m1scr" maps to group Root.Foo; "Root.Foo.Missing" is project-rooted but absent.
    // In *write* position this is T031 (unresolved assignment target), not T001
    // (unresolved reference) — issue #19: a write target is not a read.
    let c = codes(&p, "Foo Update.m1scr", "Root.Foo.Missing = 1;\n");
    assert!(c.contains(&TypeCode::T031), "expected T031, got {c:?}");
    assert!(
        !c.contains(&TypeCode::T001),
        "should not be T001, got {c:?}"
    );
    // Exactly one diagnostic (not one per path prefix).
    assert_eq!(c.iter().filter(|x| **x == TypeCode::T031).count(), 1);
}

#[test]
fn read_of_project_rooted_miss_is_still_t001() {
    let p = proj();
    // Same missing path in *read* position (RHS) remains a genuine T001.
    let c = codes(&p, "Foo Update.m1scr", "local x = Root.Foo.Missing;\n");
    assert!(c.contains(&TypeCode::T001), "expected T001, got {c:?}");
    assert!(
        !c.contains(&TypeCode::T031),
        "should not be T031, got {c:?}"
    );
}

#[test]
fn compound_assignment_to_miss_stays_t001() {
    let p = proj();
    // A compound assignment reads the target before writing, so an unresolved
    // compound target is a genuine unresolved read → T001, not T031.
    let c = codes(&p, "Foo Update.m1scr", "Root.Foo.Missing += 1;\n");
    assert!(c.contains(&TypeCode::T001), "expected T001, got {c:?}");
    assert!(
        !c.contains(&TypeCode::T031),
        "should not be T031, got {c:?}"
    );
}

#[test]
fn no_flag_for_known_symbol_or_library() {
    let p = proj();
    // Speed resolves (group-relative); Calculate is an opaque library object.
    let c = codes(&p, "Foo Update.m1scr", "Speed = Calculate.Min(1, 2);\n");
    assert!(!c.contains(&TypeCode::T001));
    assert!(!c.contains(&TypeCode::T031));
}
