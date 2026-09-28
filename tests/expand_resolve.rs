//! #246 — T031/T001 must expand `expand…to` loop variables (`$(N)`) before
//! resolving a templated assignment/reference target, matching M1 Build's
//! compile-time loop expansion (manual p.33). M1 Build → Validate Project
//! reports 0 errors on `$(N)`-parameterised targets whose every expansion
//! exists; m1-typecheck must agree.
//!
//! The script lives in group `Root.Drv` and writes targets rooted at the *real*
//! project group `Acc` (`Acc.Segment $(N).Cell`), so the templated segment is in
//! the middle of the path — the root is a known group and the literal `$(N)`
//! path is `Unresolved` (the false positive), not an opaque foreign root.
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;
use std::path::Path;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("expand_targets.m1prj")).unwrap()
}

fn codes(p: &Project, src: &str) -> Vec<TypeCode> {
    check_script(p, Path::new("Drv Update.m1scr"), src)
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn expand_templated_target_within_bounds_is_not_t031() {
    // The #246 case: under `expand (N = 0 to 3)` the target `Acc.Segment $(N).Cell`
    // expands to Acc.Segment 0..3 — all exist — so M1 Build reports 0 errors. No T031.
    let p = proj();
    let src = "expand (N = 0 to 3)\n{\n\tAcc.Segment $(N).Cell = 1;\n}\n";
    let c = codes(&p, src);
    assert!(
        !c.contains(&TypeCode::T031),
        "false-positive T031 on in-bounds expand-templated target: {c:?}"
    );
}

#[test]
fn expand_templated_read_within_bounds_is_not_t001() {
    // The same expansion applies to a templated read on the value side.
    let p = proj();
    let src = "expand (N = 0 to 3)\n{\n\tlocal x = Acc.Segment $(N).Cell;\n}\n";
    let c = codes(&p, src);
    assert!(
        !c.contains(&TypeCode::T001),
        "false-positive T001 on in-bounds expand-templated read: {c:?}"
    );
}

#[test]
fn expand_templated_target_out_of_bounds_names_the_concrete_miss() {
    // Segment 4 does not exist; M1 Build *would* report Error 1338 on that
    // iteration. Expanding and checking each value must still flag it — and name
    // the concrete failing index, not the literal `$(N)` text. The fix suppresses
    // false positives, it must not hide real misses.
    let p = proj();
    let src = "expand (N = 0 to 4)\n{\n\tAcc.Segment $(N).Cell = 1;\n}\n";
    let diags = check_script(&p, Path::new("Drv Update.m1scr"), src).diagnostics;
    let t031 = diags
        .iter()
        .find(|d| d.code == TypeCode::T031)
        .expect("T031 fires for the out-of-range iteration");
    assert!(
        t031.inner.message.contains("Segment 4"),
        "T031 should name the concrete failing iteration, got: {}",
        t031.inner.message
    );
}

#[test]
fn expand_templated_target_with_non_literal_bound_is_suppressed() {
    // A constant-named bound (`to Count`) can't be statically enumerated, so the
    // `$(N)` target can't be expanded to a concrete name. We cannot prove it is
    // unknown — M1 Build resolves it at expansion — so no T031 (conservative).
    let p = proj();
    let src = "expand (N = 0 to Count)\n{\n\tAcc.Segment $(N).Cell = 1;\n}\n";
    let c = codes(&p, src);
    assert!(
        !c.contains(&TypeCode::T031),
        "non-literal-bound expand target must not be flagged: {c:?}"
    );
}
