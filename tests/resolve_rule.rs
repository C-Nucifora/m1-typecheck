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

#[test]
fn bare_single_segment_unknown_read_is_t001() {
    let p = proj();
    // "WidgetCount" is not a local, not a library object, not a project symbol,
    // and not a group-relative channel of Root.Foo — a bare typo. In *read*
    // position it is a genuine unresolved reference (#109).
    let c = codes(&p, "Foo Update.m1scr", "local x = WidgetCount;\n");
    assert!(c.contains(&TypeCode::T001), "expected T001, got {c:?}");
}

#[test]
fn bare_single_segment_unknown_write_is_t031() {
    let p = proj();
    // Same bare miss as a plain assignment *target* → T031, not T001 (#109/#19).
    let c = codes(&p, "Foo Update.m1scr", "WidgetCount = 42;\n");
    assert!(c.contains(&TypeCode::T031), "expected T031, got {c:?}");
    assert!(
        !c.contains(&TypeCode::T001),
        "should not be T001, got {c:?}"
    );
}

#[test]
fn bare_single_segment_known_group_relative_not_flagged() {
    let p = proj();
    // "Speed" IS a group-relative channel (Root.Foo.Speed) — must stay silent.
    let c = codes(&p, "Foo Update.m1scr", "local x = Speed;\n");
    assert!(!c.contains(&TypeCode::T001), "false positive, got {c:?}");
    assert!(!c.contains(&TypeCode::T031));
}

#[test]
fn bare_reserved_keyword_not_flagged() {
    let p = proj();
    // Bare anchor keywords are not project references — they must not be flagged
    // unresolved even though they have no symbol-table entry.
    for kw in ["This", "Library", "Root"] {
        let c = codes(&p, "Foo Update.m1scr", &format!("local x = {kw};\n"));
        assert!(
            !c.contains(&TypeCode::T001),
            "{kw} should not be T001, got {c:?}"
        );
    }
}

#[test]
fn bare_enumerator_not_flagged() {
    let p = proj();
    // "Off"/"On" are members of the project's `Switch State` enum, referenced
    // here bare (no type prefix) — valid enumerators, not unresolved (#109).
    let c = codes(&p, "Foo Update.m1scr", "local x = On;\nlocal y = Off;\n");
    assert!(!c.contains(&TypeCode::T001), "enumerator flagged: {c:?}");
}

#[test]
fn expand_loop_variable_not_flagged() {
    let p = proj();
    // `expand (seg = 0 to 3) { … $(seg) … }` declares an integer loop variable;
    // neither the declaration nor the `$(seg)` use is an unresolved reference.
    let c = codes(
        &p,
        "Foo Update.m1scr",
        "expand (seg = 0 to 3)\n{\n\tlocal x = $(seg);\n}\n",
    );
    assert!(!c.contains(&TypeCode::T001), "expand var flagged: {c:?}");
}

#[test]
fn typo_on_value_compound_trailing_segment_is_flagged() {
    let p = proj();
    // `Foo.Mode` is a value-compound group (has a `.Value` child). `Foo.Mode.Valuee`
    // is a typo of `.Value` — previously accepted opaquely because any segment after
    // a value compound was treated as an accessor (#108). Now flagged.
    let c = codes(&p, "Foo Update.m1scr", "local x = Foo.Mode.Valuee;\n");
    assert!(c.contains(&TypeCode::T001), "expected T001, got {c:?}");
}

#[test]
fn value_compound_real_accessor_and_value_not_flagged() {
    let p = proj();
    // The real `.Value` child resolves; recognised accessors (`.AsInteger`,
    // `.Set`) on the compound stay opaque — neither is a miss.
    for src in [
        "local x = Foo.Mode.Value;\n",
        "local x = Foo.Mode.AsInteger;\n",
        "Foo.Mode.Set(1);\n",
    ] {
        let c = codes(&p, "Foo Update.m1scr", src);
        assert!(
            !c.contains(&TypeCode::T001) && !c.contains(&TypeCode::T031),
            "false positive on `{src}`: {c:?}"
        );
    }
}

#[test]
fn value_compound_enumerator_member_not_flagged() {
    let p = proj();
    // Where a channel name collides with an enum type name, the compound can be
    // addressed by an enumerator (`Foo.Mode.On`, with `On` a `Switch State`
    // member). An enum member is a valid trailing segment, not a miss (#108).
    let c = codes(&p, "Foo Update.m1scr", "local x = Foo.Mode.On;\n");
    assert!(!c.contains(&TypeCode::T001), "enumerator flagged: {c:?}");
}

#[test]
fn bare_typo_in_is_clause_state_not_flagged() {
    let p = proj();
    // The `state` of a `when…is (...)` clause is an enumerator (possibly of a
    // firmware enum whose members aren't modelled), never a project reference —
    // T001 must not fire there (membership is T020/T070's concern) (#109).
    let c = codes(
        &p,
        "Foo Update.m1scr",
        "when (Gain.Value)\n{\n\tis (SomeEnumerator)\n\t{\n\t}\n}\n",
    );
    assert!(
        !c.contains(&TypeCode::T001),
        "is-clause state flagged: {c:?}"
    );
}
