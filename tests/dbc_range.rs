//! T042 dbc-signal-range: a literal assigned to a CAN signal outside its
//! derived physical range is flagged; in-range and computed RHS are not.
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;
use std::path::Path;

fn temp(name: &str, content: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let uniq = N.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("m1tc_dbc_{}_{}_{}", std::process::id(), uniq, name));
    std::fs::write(&p, content).unwrap();
    p
}

/// Project with one signal `Bus.Msg.Sig`: u32, 8-bit, mult 1, offset 0 → [0, 255].
fn project_with_signal() -> Project {
    let prj = temp(
        "Project.m1prj",
        "<?xml version=\"1.0\"?>\n<Project></Project>",
    );
    let dbc = temp(
        "bus.m1dbc",
        "<?xml version=\"1.0\"?>\n<DBCs>\n\
         <Component Classname=\"BuiltIn.CAN.Signal\" Name=\"Bus.Msg.Sig\">\
         <Props Type=\"u32\" StartBit=\"0\" Length=\"8\" Multiplier=\"1.0\" Offset=\"0.0\"/>\
         </Component>\n</DBCs>",
    );
    let p = Project::load(&prj)
        .unwrap()
        .with_dbc(&dbc, "bus.m1dbc")
        .unwrap();
    let _ = std::fs::remove_file(&prj);
    let _ = std::fs::remove_file(&dbc);
    p
}

fn t042_count(p: &Project, src: &str) -> usize {
    check_script(p, Path::new("X.m1scr"), src)
        .diagnostics
        .iter()
        .filter(|d| d.code == TypeCode::T042)
        .count()
}

#[test]
fn flags_literal_above_signal_range() {
    assert_eq!(
        t042_count(&project_with_signal(), "Bus.Msg.Sig = 300;\n"),
        1
    );
}

#[test]
fn accepts_literal_within_range() {
    assert_eq!(
        t042_count(&project_with_signal(), "Bus.Msg.Sig = 200;\n"),
        0
    );
}

#[test]
fn flags_negative_literal_below_range() {
    // u32 signal's min is 0, so a negative literal is out of range.
    assert_eq!(t042_count(&project_with_signal(), "Bus.Msg.Sig = -5;\n"), 1);
}

#[test]
fn flags_hex_literal_beyond_i64_range() {
    // #97: a hex constant past i64::MAX used to parse as None and skip the range
    // check entirely. A [0,255] signal must still flag these grossly-out-of-range
    // writes. `0x100` (256) is the in-i64 baseline; the wide ones are the fix.
    let p = project_with_signal();
    assert_eq!(t042_count(&p, "Bus.Msg.Sig = 0x100;\n"), 1, "0x100 (256)");
    assert_eq!(
        t042_count(&p, "Bus.Msg.Sig = 0xFFFFFFFFFFFFFFFF;\n"),
        1,
        "u64::MAX hex must still be flagged"
    );
    assert_eq!(
        t042_count(&p, "Bus.Msg.Sig = 0x8000000000000000;\n"),
        1,
        "top-bit-set hex must be a large positive, not a wrapped negative"
    );
}

#[test]
fn accepts_in_range_hex_literal() {
    // A hex literal that lands inside [0,255] must not be flagged.
    assert_eq!(
        t042_count(&project_with_signal(), "Bus.Msg.Sig = 0xFF;\n"),
        0
    );
}

#[test]
fn flags_out_of_range_literal_compared_against_signal() {
    // #113: read-only signals are compared, never assigned. A guard literal past
    // the signal's physical range is the common real-world bug T042 missed.
    let p = project_with_signal();
    assert_eq!(
        t042_count(&p, "if (Bus.Msg.Sig > 300)\n{\nx = 1;\n}\n"),
        1,
        "comparison `Sig > 300` (max 255) should be flagged"
    );
    // Literal-on-the-left form, too.
    assert_eq!(
        t042_count(&p, "if (300 < Bus.Msg.Sig)\n{\nx = 1;\n}\n"),
        1,
        "`300 < Sig` should be flagged"
    );
}

#[test]
fn accepts_in_range_literal_comparison() {
    assert_eq!(
        t042_count(
            &project_with_signal(),
            "if (Bus.Msg.Sig > 200)\n{\nx = 1;\n}\n"
        ),
        0,
        "an in-range threshold is a legitimate guard"
    );
}

#[test]
fn ignores_computed_rhs() {
    // Not a literal — value is unknown at check time, so no range check.
    assert_eq!(
        t042_count(&project_with_signal(), "Bus.Msg.Sig = Bus.Msg.Sig + 1;\n"),
        0
    );
}

#[test]
fn flags_hex_literal_wider_than_u128() {
    // A hex constant with 33 hex digits (> 128 bits) cannot be parsed by either
    // u64::from_str_radix or u128::from_str_radix. parse_hex falls back to the
    // digit-by-digit f64 accumulation added in #97. The value is astronomically
    // larger than the signal's [0, 255] range so T042 must still fire.
    assert_eq!(
        t042_count(
            &project_with_signal(),
            "Bus.Msg.Sig = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF1;\n"
        ),
        1,
        "hex constant wider than u128 must be caught by the digit accumulation branch"
    );
}
