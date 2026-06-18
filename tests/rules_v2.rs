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

#[test]
fn t021_flags_enum_vs_integer() {
    let p = proj();
    // SwitchMode.Value is enum; comparing to integer literal 1.
    assert!(codes(&p, "if (SwitchMode.Value eq 1) {\n}\n").contains(&TypeCode::T021));
}

#[test]
fn t021_no_flag_enum_vs_member() {
    let p = proj();
    assert!(
        !codes(&p, "if (SwitchMode.Value eq Switch State.On) {\n}\n").contains(&TypeCode::T021)
    );
}

#[test]
fn t002_not_fired_for_enum_vs_float_literal() {
    // `SwitchMode.Value eq 1.0` mixes an enum with a number: the genuine issue is
    // T021 (comparing an enum to a number), NOT T002 (float-equality precision).
    // An enum is a discrete enumerator, so there is no float-precision hazard —
    // T002 firing here is a spurious duplicate. T021 must still fire.
    let p = proj();
    let got = codes(&p, "if (SwitchMode.Value eq 1.0) {\n}\n");
    assert!(
        !got.contains(&TypeCode::T002),
        "T002 must not fire when an operand is an enum: {got:?}"
    );
    assert!(
        got.contains(&TypeCode::T021),
        "T021 should still flag the enum-vs-number comparison: {got:?}"
    );
}

#[test]
fn t002_still_fires_for_two_floats() {
    // Guard against over-suppression: a real float == float comparison must still
    // be flagged when no enum is involved.
    let p = proj();
    let got = codes(&p, "local a = 1.5;\nlocal b = 2.5;\nif (a eq b) {\n}\n");
    assert!(
        got.contains(&TypeCode::T002),
        "T002 must still fire for a genuine float == float: {got:?}"
    );
}

#[test]
fn t021_no_flag_int_vs_int() {
    let p = proj();
    assert!(!codes(&p, "local iX = 1;\nif (iX eq 2) {\n}\n").contains(&TypeCode::T021));
}

#[test]
fn t021_flags_firmware_enum_vs_integer() {
    // `fwMode` is typed `MoTeC Types.Mode Enumeration` (a firmware enum). Even
    // though its members are unknown, comparing the enum to a number is wrong
    // regardless of members, so T021 still fires (#104).
    let p = proj();
    assert!(codes(&p, "if (fwMode eq 1) {\n}\n").contains(&TypeCode::T021));
}

#[test]
fn t020_no_flag_on_open_firmware_enum_member() {
    // A firmware enum the builtin catalogue does NOT document stays an open
    // placeholder: any `Mystery Enumeration.<X>` is a member we cannot
    // disprove — T020 must NOT fire (it would be a false positive on valid
    // firmware-enum usage) (#104).
    let p = proj();
    assert!(
        !codes(&p, "fwMystery = Mystery Enumeration.Whatever;\n").contains(&TypeCode::T020),
        "T020 must be suppressed for open (firmware) enums"
    );
}

#[test]
fn t020_fires_on_documented_builtin_enum_non_member() {
    // `Mode Enumeration` IS documented by the builtin catalogue (members
    // Disabled/Enabled), so a fabricated member is M1 Build Error 1352.
    let p = proj();
    let got = codes(&p, "fwMode = Mode Enumeration.Whatever;\n");
    assert!(got.contains(&TypeCode::T020), "{got:?}");
    assert!(!codes(&p, "fwMode = Mode Enumeration.Enabled;\n").contains(&TypeCode::T020));
}

#[test]
fn t030_flags_enum_member_into_wrong_enum() {
    let p = proj();
    // SwitchMode.Value is Switch State; assigning a Drive State member is a mismatch.
    assert!(codes(&p, "SwitchMode.Value = Drive State.Idle;\n").contains(&TypeCode::T030));
}

#[test]
fn t030_no_flag_same_enum() {
    let p = proj();
    assert!(!codes(&p, "SwitchMode.Value = Switch State.On;\n").contains(&TypeCode::T030));
}

#[test]
fn t030_no_flag_unknown_target() {
    let p = proj();
    // driveMode has no known type -> silent.
    assert!(!codes(&p, "driveMode = 1.0;\n").contains(&TypeCode::T030));
}

#[test]
fn t030_flags_float_to_integer_local_plain_assignment() {
    // Manual p.44: `local <Unsigned Integer> low = 5; local high = 6.5;` then
    // `low = high;` is *not valid*. A plain `=` float→integer write to a local
    // is the same mismatch as a channel assignment and must flag (#210).
    let p = proj();
    let src = "local <Unsigned Integer> low = 5;\nlocal high = 6.5;\nlow = high;\n";
    let got = codes(&p, src);
    assert!(got.contains(&TypeCode::T030), "expected T030, got {got:?}");
}

#[test]
fn t030_no_flag_integer_to_float_local() {
    // The reverse (integer into a float local) is numerically compatible — silent.
    let p = proj();
    let src = "local hi = 6.5;\nlocal <Unsigned Integer> low = 5;\nhi = low;\n";
    assert!(!codes(&p, src).contains(&TypeCode::T030));
}

#[test]
fn t030_silent_when_local_type_is_unknown() {
    // A local whose initialiser is an unresolved channel read has Unknown type
    // (#210). Re-assigning that local from a float literal must NOT fire T030 —
    // the rule's `!value_ty.is_known()` / `!target_ty.is_known()` guard is
    // specifically designed to stay silent under Unknown so that incomplete
    // projects don't produce a flood of spurious mismatches.
    let p = proj();
    let src = "local x = Unresolved.Channel;\nx = 1.5;\n";
    assert!(
        !codes(&p, src).contains(&TypeCode::T030),
        "Unknown-typed local must not generate T030 on float assignment"
    );
}

#[test]
fn t030_message_renders_enum_name_not_internal_id() {
    // #214: the T030 message must name the enum (`enum \`Switch State\``), not
    // leak the internal `Enum(0)` id. `SwitchMode.Value` is enum `Switch State`;
    // assigning a plain Integer is a mismatch.
    let p = proj();
    let diags =
        check_script(&p, Path::new("Foo Update.m1scr"), "SwitchMode.Value = 3;\n").diagnostics;
    let t030 = diags
        .iter()
        .find(|d| d.code == TypeCode::T030)
        .expect("expected a T030 diagnostic");
    assert!(
        t030.inner.message.contains("enum `Switch State`"),
        "message should name the enum, got: {}",
        t030.inner.message
    );
    assert!(
        !t030.inner.message.contains("Enum("),
        "message must not leak the internal id, got: {}",
        t030.inner.message
    );
}

#[test]
fn t030_resolves_this_qualified_target() {
    // #211: `This.<X>` anchors to the script's group (`Root.Foo`), so a
    // `This.`-qualified assignment participates in the type rules exactly like the
    // bare reference. `This.gain` is the f32 channel `Root.Foo.gain`; assigning an
    // enum member to it is a T030 mismatch.
    let p = proj();
    let bare = codes(&p, "gain = Drive State.Idle;\n");
    let qualified = codes(&p, "This.gain = Drive State.Idle;\n");
    assert!(
        bare.contains(&TypeCode::T030),
        "bare baseline, got {bare:?}"
    );
    assert!(
        qualified.contains(&TypeCode::T030),
        "This.-qualified target must resolve and flag, got {qualified:?}"
    );
}

// ---- T030 compound-assignment type mismatch (#221) ----------------------

#[test]
fn t030_compound_assign_flags_integer_into_enum_channel() {
    // `SwitchMode.Value` is enum `Switch State`; `+= 2` is incompatible (#221).
    let p = proj();
    let got = codes(&p, "SwitchMode.Value += 2;\n");
    assert!(
        got.contains(&TypeCode::T030),
        "enum target with integer RHS via += must flag T030, got {got:?}"
    );
}

#[test]
fn t030_compound_assign_flags_wrong_enum_into_enum_channel() {
    // Compound assignment with a mismatched enum member on an enum target.
    let p = proj();
    let got = codes(&p, "SwitchMode.Value += Drive State.Idle;\n");
    assert!(
        got.contains(&TypeCode::T030),
        "enum target with wrong-enum RHS via += must flag T030, got {got:?}"
    );
}

#[test]
fn t030_compound_assign_silent_for_unknown_target() {
    // `driveMode` has no declared type → Unknown → must stay silent per design.
    let p = proj();
    let got = codes(&p, "driveMode += 2;\n");
    assert!(
        !got.contains(&TypeCode::T030),
        "Unknown-typed target via += must not generate T030, got {got:?}"
    );
}

#[test]
fn t030_compound_assign_no_double_flag_int_target_float_rhs() {
    // `intCh += 2.0` is already flagged by T003 (float→integral narrowing);
    // T030 must NOT also fire to avoid duplicate diagnostics on the same node.
    // `gain` is a Float channel, so use a typed-integer local to test.
    let p = proj();
    // local <Integer> iX; iX += 2.0 → T003 fires, T030 must not.
    let src = "local <Integer> iX = 1;\niX += 2.0;\n";
    let got = codes(&p, src);
    assert!(
        !got.contains(&TypeCode::T030),
        "T030 must not double-fire on int-target + float-rhs compound assign (T003 owns this), got {got:?}"
    );
}

#[test]
fn t030_compound_assign_no_flag_float_target_int_rhs() {
    // `gain` is a Float channel; `+= 2` is integer→float widening — compatible, silent.
    let p = proj();
    let got = codes(&p, "gain += 2;\n");
    assert!(
        !got.contains(&TypeCode::T030),
        "float target with integer RHS via += is widening — must not flag T030, got {got:?}"
    );
}

#[test]
fn t030_compound_assign_no_flag_same_enum() {
    // Same-enum compound assign is compatible (odd but not a type mismatch).
    let p = proj();
    let got = codes(&p, "SwitchMode.Value += Switch State.On;\n");
    assert!(
        !got.contains(&TypeCode::T030),
        "same-enum compound assign must not flag T030, got {got:?}"
    );
}

// ---- T070 when-is-exhaustive --------------------------------------------
// `SwitchMode.Value` is enum `Switch State` with members {Off, On}.

#[test]
fn t070_flags_missing_enumerator() {
    let p = proj();
    let src = "when (SwitchMode.Value) {\nis (Off) {\n}\n}\n";
    assert!(codes(&p, src).contains(&TypeCode::T070));
}

#[test]
fn t070_no_flag_when_all_covered() {
    let p = proj();
    let src = "when (SwitchMode.Value) {\nis (Off) {\n}\nis (On) {\n}\n}\n";
    assert!(!codes(&p, src).contains(&TypeCode::T070));
}

#[test]
fn t070_no_flag_when_all_covered_via_or() {
    let p = proj();
    let src = "when (SwitchMode.Value) {\nis (Off or On) {\n}\n}\n";
    assert!(!codes(&p, src).contains(&TypeCode::T070));
}

#[test]
fn t070_no_flag_non_enum_subject() {
    let p = proj();
    // `gain` is an f32 channel, not an enum -> exhaustiveness stays silent;
    // the non-enum subject itself is T082's finding (manual p.32).
    let src = "when (gain) {\nis (Off) {\n}\n}\n";
    let got = codes(&p, src);
    assert!(!got.contains(&TypeCode::T070));
    assert!(got.contains(&TypeCode::T082), "non-enum subject is T082");
}

// ---- package (`::Hardware.*`) enums with manual-documented membership ------
// In `package_enums.m1prj`, `bspdActive` is typed `::Hardware.av_switch.sw_state`
// — the standard "Universal Switch State" firmware enum, whose Off/On membership
// the M1 Development Manual documents. M1 Build resolves it and enforces
// membership checks (Errors 1306/1352), so the toolchain must too (#167/#168).

fn pkg_proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("package_enums.m1prj")).unwrap()
}

fn pkg_codes(p: &Project, src: &str) -> Vec<TypeCode> {
    check_script(p, Path::new("Pkg Update.m1scr"), src)
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn t070_flags_missing_enumerator_on_package_enum() {
    // Probe from #167: `when` covering only Off → M1 Build Error 1306
    // ("missing 'is' statement for enumerator 'On'").
    let p = pkg_proj();
    let src = "when (bspdActive) {\nis (Off) {\n}\n}\n";
    assert!(pkg_codes(&p, src).contains(&TypeCode::T070));
}

#[test]
fn t070_no_flag_package_enum_all_covered() {
    let p = pkg_proj();
    let src = "when (bspdActive) {\nis (Off) {\n}\nis (On) {\n}\n}\n";
    assert!(!pkg_codes(&p, src).contains(&TypeCode::T070));
}

#[test]
fn t020_flags_non_member_on_package_enum() {
    // Probe from #168: typo'd enumerator → M1 Build Errors 1352/1320.
    let p = pkg_proj();
    let got = pkg_codes(&p, "bspdActive = Universal Switch State.Nonexistent;\n");
    assert!(got.contains(&TypeCode::T020), "{got:?}");
}

#[test]
fn t020_t030_no_flag_for_documented_package_enum_member() {
    // The channel's declared type and the typed member path resolve to the SAME
    // enum id, so neither membership (T020) nor assignment (T030) fires.
    let p = pkg_proj();
    let got = pkg_codes(&p, "bspdActive = Universal Switch State.On;\n");
    assert!(!got.contains(&TypeCode::T020), "{got:?}");
    assert!(!got.contains(&TypeCode::T030), "{got:?}");
}

#[test]
fn t020_is_an_error_like_m1_build_1352() {
    // M1 Build fails the build on a non-member enumerator (Error 1352), so
    // T020 must be an error (non-zero exit), not a warning.
    let p = pkg_proj();
    let diags = check_script(
        &p,
        Path::new("Pkg Update.m1scr"),
        "bspdActive = Universal Switch State.Nonexistent;\n",
    )
    .diagnostics;
    let t020 = diags
        .iter()
        .find(|d| d.code == TypeCode::T020)
        .expect("T020 fires");
    assert_eq!(t020.inner.severity, m1_core::Severity::Error);
}

#[test]
fn t020_head_case_variant_resolves_and_warns() {
    // M1 Build resolves the enum *type name* case-insensitively too: AV-M1's
    // `… eq universal Switch State.On` validates with 0 errors. The case-variant
    // head must resolve to the enum — so the member is still checked — and the
    // head spelling itself gets the style Warning, mirroring the member
    // carve-out below. (#183)
    let p = pkg_proj();
    let diags = check_script(
        &p,
        Path::new("Pkg Update.m1scr"),
        "bspdActive = universal Switch State.On;\n",
    )
    .diagnostics;
    let t020 = diags
        .iter()
        .find(|d| d.code == TypeCode::T020)
        .expect("case-variant head must resolve and be flagged");
    assert_eq!(t020.inner.severity, m1_core::Severity::Warning);
    assert!(
        t020.inner.message.contains("case"),
        "{}",
        t020.inner.message
    );
}

#[test]
fn t020_head_case_variant_nonmember_still_error() {
    // A name that is no member under any casing is M1 Build Error 1352 even
    // when the head is itself a case variant — resolution must not weaken the
    // membership check. (#183)
    let p = pkg_proj();
    let diags = check_script(
        &p,
        Path::new("Pkg Update.m1scr"),
        "bspdActive = universal Switch State.Nope;\n",
    )
    .diagnostics;
    let t020 = diags
        .iter()
        .find(|d| d.code == TypeCode::T020)
        .expect("non-member behind a case-variant head must be flagged");
    assert_eq!(t020.inner.severity, m1_core::Severity::Error);
}

#[test]
fn t020_case_variant_of_member_stays_warning() {
    // M1 Build resolves names case-insensitively (manual pp.64-65), so a
    // case-variant of a real member ("OFf" for "Off") builds with 0 errors —
    // it must stay a Warning (style), not the Error-1352 parity error.
    // Mirrors `ASSI.OFf` in the AV-M1 corpus, which Validate Project accepts.
    let p = pkg_proj();
    let diags = check_script(
        &p,
        Path::new("Pkg Update.m1scr"),
        "bspdActive = Universal Switch State.OFf;\n",
    )
    .diagnostics;
    let t020 = diags
        .iter()
        .find(|d| d.code == TypeCode::T020)
        .expect("case variant still flagged");
    assert_eq!(t020.inner.severity, m1_core::Severity::Warning);
    assert!(
        t020.inner.message.contains("case"),
        "{}",
        t020.inner.message
    );
}

// ---- T021 on package/IO-method enums + Error severity (#173) ---------------
// `ASMS` is an `_IOMethod.av_switch` object: M1 Build types its value (and its
// auto-created `State` sub-channel) as "Universal Switch State" and rejects a
// comparison against a number with build-failing Error 1329.

#[test]
fn t021_fires_on_io_switch_object_vs_number() {
    // Probe from #173: `Driver.ASMS eq 0` → M1 Build Errors 1329/1351.
    let p = pkg_proj();
    let got = pkg_codes(&p, "if (ASMS eq 0) {\n}\n");
    assert!(got.contains(&TypeCode::T021), "{got:?}");
}

#[test]
fn t021_fires_on_io_switch_state_subchannel_vs_number() {
    let p = pkg_proj();
    let got = pkg_codes(&p, "if (ASMS.State eq 1) {\n}\n");
    assert!(got.contains(&TypeCode::T021), "{got:?}");
}

#[test]
fn t021_no_flag_io_switch_vs_its_enum_member() {
    // The valid form from the corpus: `Driver.ASMS eq Universal Switch
    // State.Off` — both sides the same enum, no finding.
    let p = pkg_proj();
    let got = pkg_codes(&p, "if (ASMS eq Universal Switch State.Off) {\n}\n");
    assert!(!got.contains(&TypeCode::T021), "{got:?}");
    assert!(!got.contains(&TypeCode::T030), "{got:?}");
}

#[test]
fn t021_io_switch_voltage_subchannel_stays_float() {
    // The object's other auto-created children must not be mistyped as the
    // switch enum: `Voltage` is a measured float, so comparing it to a number
    // is fine (no T021).
    let p = pkg_proj();
    let got = pkg_codes(&p, "if (ASMS.Voltage > 2.5) {\n}\n");
    assert!(!got.contains(&TypeCode::T021), "{got:?}");
}

#[test]
fn t021_is_an_error_like_m1_build_1329() {
    // M1 Build fails the build on an enum-vs-number comparison (Error 1329),
    // so T021 must be an error (non-zero exit), not a warning.
    let p = pkg_proj();
    let diags =
        check_script(&p, Path::new("Pkg Update.m1scr"), "if (ASMS eq 0) {\n}\n").diagnostics;
    let t021 = diags
        .iter()
        .find(|d| d.code == TypeCode::T021)
        .expect("T021 fires");
    assert_eq!(t021.inner.severity, m1_core::Severity::Error);
}

// ---- T021 on ordering comparisons between two enums -----------------------
// The Manual's "Enumeration Comparison" table (p.36-37) lists ONLY `a eq b` and
// `a neq b` for enumerated types. Ordering (`>`/`<`/`>=`/`<=`) is not defined;
// M1 Build rejects it with Error 1329 (incompatible data types). Equality
// (eq/neq) between two enums stays valid.

#[test]
fn t021_flags_enum_ordering_comparison() {
    let p = proj();
    // Both operands are the same project enum `Switch State`; `>` is not a
    // defined enum operator, so M1 Build rejects it (Error 1329).
    let got = codes(&p, "if (SwitchMode.Value > Switch State.On) {\n}\n");
    assert!(
        got.contains(&TypeCode::T021),
        "ordering an enum must be flagged: {got:?}"
    );
}

#[test]
fn t021_flags_enum_ordering_with_lt() {
    let p = proj();
    let got = codes(&p, "if (SwitchMode.Value < Switch State.On) {\n}\n");
    assert!(got.contains(&TypeCode::T021), "{got:?}");
}

#[test]
fn t021_no_flag_enum_equality_between_two_enums() {
    let p = proj();
    // eq/neq between two enums is the ONLY valid enum comparison per the manual.
    assert!(
        !codes(&p, "if (SwitchMode.Value eq Switch State.On) {\n}\n").contains(&TypeCode::T021)
    );
    assert!(
        !codes(&p, "if (SwitchMode.Value == Switch State.On) {\n}\n").contains(&TypeCode::T021)
    );
}

#[test]
fn t021_no_flag_enum_ordering_against_unknown() {
    let p = proj();
    // The right side is unresolvable, so we cannot prove both sides are enums:
    // ordering must NOT fire (no false positive on an Unknown operand).
    assert!(!codes(&p, "if (SwitchMode.Value > Mystery Channel) {\n}\n").contains(&TypeCode::T021));
}

// ---- T021 on eq/neq between two DIFFERENT enum types ----------------------
// M1 Build rejects comparing values of two distinct enumerated types even with
// `eq`/`neq` (Error 1329, incompatible data types) — only same-enum comparison
// is valid. `DriveSel.Value` is `Drive State`; `SwitchMode.Value` is
// `Switch State`, so comparing them is a real logic bug M1 Build would reject.

#[test]
fn t021_flags_eq_between_two_different_enums() {
    let p = proj();
    let got = codes(&p, "if (DriveSel.Value eq SwitchMode.Value) {\n}\n");
    assert!(
        got.contains(&TypeCode::T021),
        "comparing two different enum types must be flagged: {got:?}"
    );
}

#[test]
fn t021_flags_neq_between_two_different_enums() {
    let p = proj();
    assert!(
        codes(&p, "if (DriveSel.Value neq SwitchMode.Value) {\n}\n").contains(&TypeCode::T021),
        "neq across two enum types must be flagged"
    );
}

#[test]
fn t021_flags_eqeq_between_two_different_enums() {
    let p = proj();
    assert!(
        codes(&p, "if (DriveSel.Value == SwitchMode.Value) {\n}\n").contains(&TypeCode::T021),
        "== across two enum types must be flagged"
    );
}

#[test]
fn t021_no_flag_eq_between_same_enum_channels() {
    let p = proj();
    // Both sides are the same `Switch State` enum: a valid comparison, no T021.
    assert!(
        !codes(&p, "if (SwitchMode.Value eq Switch State.Off) {\n}\n").contains(&TypeCode::T021),
        "same-enum eq must stay valid"
    );
}

#[test]
fn t021_no_flag_cross_enum_against_open_firmware_enum() {
    let p = proj();
    // `fwMystery` is an open firmware enum (`Mystery Enumeration`) whose id we
    // cannot match against a project enum: comparing it to a project enum must
    // NOT fire (conservatism — we only flag when BOTH enum ids are known,
    // closed, and differ). NOTE: `fwMode` (`Mode Enumeration`) is a *documented*
    // builtin and therefore closed, so it would legitimately flag.
    assert!(
        !codes(&p, "if (DriveSel.Value eq fwMystery) {\n}\n").contains(&TypeCode::T021),
        "cross-enum must stay silent when one side is an open firmware enum"
    );
}

#[test]
fn t082_no_flag_enum_subject() {
    let p = proj();
    let src = "when (SwitchMode.Value) {\nis (Off) {\n}\nis (On) {\n}\n}\n";
    assert!(!codes(&p, src).contains(&TypeCode::T082));
}

#[test]
fn t082_no_flag_unknown_subject() {
    let p = proj();
    // An unresolvable subject stays silent (T001 territory, not T082).
    let src = "when (Mystery Channel) {\nis (Off) {\n}\n}\n";
    assert!(!codes(&p, src).contains(&TypeCode::T082));
}

#[test]
fn t070_nonmember_label_on_closed_enum_is_t020_and_does_not_suppress_t070() {
    let p = proj();
    // `SwitchMode.Value` is the CLOSED project enum `Switch State` {Off, On}.
    // M1's `when…is` has no catch-all syntax, so a non-member is-label is a
    // typo, not a default: M1 Build rejects it (Error 1352). It must surface as
    // T020 and must NOT mask the missing `On` enumerator (#212).
    let src = "when (SwitchMode.Value) {\nis (Off) {\n}\nis (Anything) {\n}\n}\n";
    let got = codes(&p, src);
    assert!(got.contains(&TypeCode::T020), "expected T020, got {got:?}");
    assert!(got.contains(&TypeCode::T070), "expected T070, got {got:?}");
}

#[test]
fn t070_typo_in_or_list_on_closed_enum_does_not_suppress_missing_enumerator() {
    let p = proj();
    // `Off or Onn` — `Onn` is a typo of `On`. Before #212 the bad label
    // disabled both checks; now `Onn` is a T020 and `On` is still reported
    // missing by T070 (the closed enum `Switch State` is {Off, On}).
    let src = "when (SwitchMode.Value) {\nis (Off or Onn) {\n}\n}\n";
    let got = codes(&p, src);
    assert!(got.contains(&TypeCode::T020), "expected T020, got {got:?}");
    assert!(got.contains(&TypeCode::T070), "expected T070, got {got:?}");
}

#[test]
fn t070_open_firmware_enum_keeps_conservative_catch_all_bail() {
    // `fwMystery` is typed by an OPEN firmware enum (`Mystery Enumeration`, not in
    // the builtin catalogue), whose full member list is unknown, so an unlisted
    // is-label may be a real member we cannot see — keep the conservative bail
    // (no T020, no T070).
    let p = proj();
    let src = "when (fwMystery) {\nis (SomethingFirmware) {\n}\n}\n";
    let got = codes(&p, src);
    assert!(
        !got.contains(&TypeCode::T020),
        "open enum: no T020, got {got:?}"
    );
    assert!(
        !got.contains(&TypeCode::T070),
        "open enum: no T070, got {got:?}"
    );
}

// NOTE (deviation from plan): the `LHS is (Member)` clause that the plan's
// Trigger 2 targeted is NOT valid syntax in the tree-sitter-m1 grammar — `is`
// always parses as an ERROR node, and the runner short-circuits on syntax
// errors, so the clause can never reach a rule. It also does not appear in the
// real m1-example corpus (every `is` there is inside a comment). T020 therefore fires
// only on the corpus-real typed-member-path idiom `<EnumType>.<Member>`.

#[test]
fn t030_carries_declaration_related_location() {
    // #200: the declared-type end of the mismatch points at the .m1prj line.
    use m1_typecheck::diagnostics::RelatedPlace;
    let p = proj();
    let def_line = p
        .symbols()
        .get("Root.Foo.SwitchMode.Value")
        .unwrap()
        .def_line
        .expect("fixture symbol has a def line");
    let diags = check_script(
        &p,
        Path::new("Foo Update.m1scr"),
        "SwitchMode.Value = Drive State.Idle;\n",
    )
    .diagnostics;
    let d = diags
        .iter()
        .find(|d| d.code == TypeCode::T030)
        .expect("T030 fires");
    let r = d.related.first().expect("related declaration attached");
    assert_eq!(r.place, RelatedPlace::Project { line: def_line });
    assert!(r.message.contains("declared"), "{:?}", r.message);
}
