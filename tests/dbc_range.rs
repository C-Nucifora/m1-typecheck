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
fn ignores_computed_rhs() {
    // Not a literal — value is unknown at check time, so no range check.
    assert_eq!(
        t042_count(&project_with_signal(), "Bus.Msg.Sig = Bus.Msg.Sig + 1;\n"),
        0
    );
}
