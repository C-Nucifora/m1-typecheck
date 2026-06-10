//! T091 local-object-case-ambiguity (#157, manual pp.64–65): a `local` whose
//! name matches the leaf of a project object referenced in the same script,
//! identically or differing only by case.
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::{check_script, check_script_no_project};
use std::path::Path;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("mini.m1prj")).unwrap()
}

fn t091(p: &Project, file: &str, src: &str) -> Vec<String> {
    check_script(p, Path::new(file), src)
        .diagnostics
        .iter()
        .filter(|d| d.code == TypeCode::T091)
        .map(|d| d.inner.message.clone())
        .collect()
}

#[test]
fn local_differing_only_by_case_from_referenced_channel_flags() {
    let p = proj();
    // Group-relative `Speed` resolves to the channel Root.Foo.Speed; the local
    // `speed` differs from its leaf only by case — the exact ambiguity the
    // manual's naming rules forbid.
    let msgs = t091(
        &p,
        "Foo Update.m1scr",
        "local speed = 0;\nspeed = Speed * 2;\n",
    );
    assert_eq!(msgs.len(), 1, "expected one T091, got {msgs:?}");
    assert!(msgs[0].contains("local `speed`"), "{msgs:?}");
    assert!(msgs[0].contains("`Root.Foo.Speed`"), "{msgs:?}");
    assert!(msgs[0].contains("only by case"), "{msgs:?}");
}

#[test]
fn local_with_identical_name_to_referenced_channel_flags_as_same_name() {
    let p = proj();
    // The local `Speed` shadows nothing namespace-wise, but referencing the
    // channel via its full path in the same script is exactly the
    // "problems with maintaining references" hazard.
    let msgs = t091(
        &p,
        "Foo Update.m1scr",
        "local Speed = 0;\nRoot.Foo.Speed = Speed;\n",
    );
    assert!(
        msgs.iter().any(|m| m.contains("has the same name")),
        "expected same-name T091, got {msgs:?}"
    );
}

#[test]
fn unrelated_local_name_is_clean() {
    let p = proj();
    let msgs = t091(
        &p,
        "Foo Update.m1scr",
        "local ratio = 0;\nratio = Speed * 2;\n",
    );
    assert!(msgs.is_empty(), "unrelated local must not flag: {msgs:?}");
}

#[test]
fn matching_local_without_a_reference_in_this_script_is_clean() {
    let p = proj();
    // The channel Root.Foo.Speed exists in the project but is never referenced
    // in this script — the rule is scoped to same-file references to stay
    // actionable (issue #157).
    let msgs = t091(
        &p,
        "Foo Update.m1scr",
        "local speed = 0;\nspeed = speed + 1;\n",
    );
    assert!(msgs.is_empty(), "no reference, no ambiguity: {msgs:?}");
}

#[test]
fn allow_annotation_suppresses_t091() {
    let p = proj();
    let src = "// @m1:allow(T091)\nlocal speed = 0;\nspeed = Speed * 2;\n";
    let msgs = t091(&p, "Foo Update.m1scr", src);
    assert!(msgs.is_empty(), "@m1:allow must suppress T091: {msgs:?}");
}

#[test]
fn no_project_means_no_t091() {
    let r = check_script_no_project("local speed = 0;\nspeed = speed + 1;\n");
    assert!(r.diagnostics.iter().all(|d| d.code != TypeCode::T091));
}
