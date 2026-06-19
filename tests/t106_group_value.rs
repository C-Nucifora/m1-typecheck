//! T106 — group-not-a-value-provider (M1 Build Error 1331, #242). A `Group` is a
//! value provider only when it declares a Default Value (`<Props UseDefValue>`);
//! passing a bare group with no default value where a value is expected is
//! rejected by M1 Build.
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;
use std::path::Path;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("groups.m1prj")).unwrap()
}

fn codes(src: &str) -> Vec<TypeCode> {
    let p = proj();
    check_script(&p, Path::new("Foo Update.m1scr"), src)
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn t106_flags_no_default_group_as_local_initialiser() {
    assert!(codes("local a = NoDef;\n").contains(&TypeCode::T106));
}

#[test]
fn t106_flags_no_default_group_as_call_argument() {
    assert!(codes("out = Helper(NoDef);\n").contains(&TypeCode::T106));
}

#[test]
fn t106_flags_no_default_group_as_assignment_rhs() {
    assert!(codes("out = NoDef;\n").contains(&TypeCode::T106));
}

#[test]
fn t106_no_flag_for_group_with_default_value() {
    // A group declaring a Default Value (UseDefValue) IS a value provider.
    assert!(!codes("local a = HasDef;\n").contains(&TypeCode::T106));
    assert!(!codes("out = Helper(HasDef);\n").contains(&TypeCode::T106));
}

#[test]
fn t106_no_flag_for_member_access_into_group() {
    // `HasDef.Value` / `NoDef.Value` resolve to the channel, not the group — the
    // group is only a namespace prefix here, not used as a value.
    assert!(!codes("local a = HasDef.Value;\n").contains(&TypeCode::T106));
    assert!(!codes("local a = NoDef.Value;\n").contains(&TypeCode::T106));
}

#[test]
fn t106_is_an_error() {
    let p = proj();
    let d = check_script(&p, Path::new("Foo Update.m1scr"), "local a = NoDef;\n")
        .diagnostics
        .into_iter()
        .find(|d| d.code == TypeCode::T106)
        .expect("expected a T106 diagnostic");
    assert_eq!(d.inner.severity, m1_core::Severity::Error);
}
