//! T081 nan-to-integer (#120): a possibly NaN/Inf value (e.g. a division result)
//! stored to an integer-typed channel is reinterpreted as a garbage integer.
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;
use std::path::Path;

fn project() -> Project {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let uniq = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("m1tc_t081_{}_{uniq}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let prj = dir.join("Project.m1prj");
    std::fs::write(
        &prj,
        r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Demo"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Demo.Count"><Props Type="u32"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Demo.A"><Props Type="f32"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Demo.B"><Props Type="f32"/></Component>
</Project>"#,
    )
    .unwrap();
    let p = Project::load(&prj).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    p
}

fn t081(p: &Project, src: &str) -> usize {
    check_script(p, Path::new("X.m1scr"), src)
        .diagnostics
        .iter()
        .filter(|d| d.code == TypeCode::T081)
        .count()
}

#[test]
fn division_into_integer_channel_flagged() {
    assert_eq!(
        t081(&project(), "Root.Demo.Count = Root.Demo.A / Root.Demo.B;\n"),
        1
    );
}

#[test]
fn division_into_float_channel_not_flagged() {
    // A float channel can hold NaN safely (no integer reinterpretation).
    assert_eq!(
        t081(&project(), "Root.Demo.A = Root.Demo.A / Root.Demo.B;\n"),
        0
    );
}

#[test]
fn non_division_assignment_not_flagged() {
    assert_eq!(t081(&project(), "Root.Demo.Count = Root.Demo.A;\n"), 0);
}

#[test]
fn tainted_local_into_integer_channel_flagged() {
    // The NaN provenance flows through a local copy.
    assert_eq!(
        t081(
            &project(),
            "local r = Root.Demo.A / Root.Demo.B;\nRoot.Demo.Count = r;\n"
        ),
        1
    );
}
