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

// NOTE (deviation from plan): the `LHS is (Member)` clause that the plan's
// Trigger 2 targeted is NOT valid syntax in the tree-sitter-m1 grammar — `is`
// always parses as an ERROR node, and the runner short-circuits on syntax
// errors, so the clause can never reach a rule. It also does not appear in the
// real m1-example corpus (every `is` there is inside a comment). T020 therefore fires
// only on the corpus-real typed-member-path idiom `<EnumType>.<Member>`.
