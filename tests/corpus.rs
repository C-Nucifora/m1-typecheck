//! Loads the example project and checks every script: must not panic. Skipped
//! unless both the project and corpus are present. Records a baseline count.
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;
use std::path::{Path, PathBuf};

fn project_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("M1_PROJECT") {
        return Some(PathBuf::from(p));
    }
    let cand =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../m1-example/UQR-EV/01.00/Project.m1prj");
    cand.is_file().then_some(cand)
}
fn corpus_dir() -> PathBuf {
    match std::env::var_os("M1_CORPUS_PATH") {
        Some(p) => PathBuf::from(p),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../m1-example/UQR-EV/01.00/Scripts"),
    }
}

#[test]
fn checks_corpus_without_panicking() {
    let Some(proj_path) = project_path() else {
        eprintln!("project absent; skipping");
        return;
    };
    let dir = corpus_dir();
    if !dir.is_dir() {
        eprintln!("corpus absent; skipping");
        return;
    }
    let project = Project::load(&proj_path).expect("load project");
    let mut checked = 0;
    let mut total_diags = 0usize;
    for entry in std::fs::read_dir(&dir).expect("read corpus") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("m1scr") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read");
        let r = check_script(&project, &path, &src);
        total_diags += r.diagnostics.len();
        checked += 1;
    }
    assert!(checked > 0, "no scripts found");
    eprintln!("checked {checked} scripts, {total_diags} type diagnostics");
}

#[test]
fn audits_project_names_without_panicking() {
    let Some(proj_path) = project_path() else {
        return;
    };
    let project = Project::load(&proj_path).expect("load project");
    let n = project.audit().len();
    eprintln!("project name-audit produced {n} T050 diagnostics");
    // No assertion on the exact count: the example project intentionally violates
    // the conventions. This guards only against a panic / API regression.
}

/// (project file, scripts dir) for every real corpus present — EV-M1 and AV-M1.
/// An explicit `M1_PROJECT`/`M1_CORPUS_PATH` override pins EV-M1 only (matching
/// the other corpus tests).
fn all_corpora() -> Vec<(PathBuf, PathBuf)> {
    // An explicit env override pins a single corpus; otherwise use every real
    // corpus checked out as a sibling (EV-M1 and AV-M1).
    if std::env::var_os("M1_PROJECT").is_some() || std::env::var_os("M1_CORPUS_PATH").is_some() {
        if let Some(p) = project_path() {
            let dir = corpus_dir();
            return if dir.is_dir() { vec![(p, dir)] } else { vec![] };
        }
        return vec![];
    }
    [
        (
            "../m1-example/UQR-EV/01.00/Project.m1prj",
            "../m1-example/UQR-EV/01.00/Scripts",
        ),
        ("../AV-M1/UQR-AV/01.00/Project.m1prj", "../AV-M1/UQR-AV"),
    ]
    .iter()
    .map(|(a, b)| {
        (
            Path::new(env!("CARGO_MANIFEST_DIR")).join(a),
            Path::new(env!("CARGO_MANIFEST_DIR")).join(b),
        )
    })
    .filter(|(a, b)| a.is_file() && b.is_dir())
    .collect()
}

fn scripts_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            scripts_under(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("m1scr") {
            out.push(p);
        }
    }
}

/// #233 corpus safety: the In/Out parity checks (T098/T099/T100/T101) must never
/// fire on real, M1-Build-valid code. The corpora pass M1 Build's "Validate
/// Project", so a correctly-precise check leaves them clean — any hit here is a
/// false positive in the new rule, not a finding.
#[test]
fn in_out_io_checks_are_corpus_clean() {
    let corpora = all_corpora();
    if corpora.is_empty() {
        eprintln!("corpus absent; skipping");
        return;
    }
    use m1_typecheck::diagnostics::TypeCode::{T098, T099, T100, T101};
    let mut checked = 0usize;
    for (proj_path, dir) in &corpora {
        let project = Project::load(proj_path).expect("load project");
        let mut scripts = Vec::new();
        scripts_under(dir, &mut scripts);
        for s in &scripts {
            let src = std::fs::read_to_string(s).expect("read");
            let hits: Vec<_> = check_script(&project, s, &src)
                .diagnostics
                .into_iter()
                .filter(|d| matches!(d.code, T098 | T099 | T100 | T101))
                .map(|d| format!("{} {}: {}", d.code.as_str(), s.display(), d.inner.message))
                .collect();
            assert!(
                hits.is_empty(),
                "In/Out parity checks false-positived on the real corpus:\n{}",
                hits.join("\n")
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no corpus scripts found");
    eprintln!("in/out parity checks clean over {checked} corpus scripts");
}

#[test]
fn sbg_gyro_unit_is_the_base_unit_degrees_per_second() {
    // `Root.Vehicle.SBG.IMU.Gyro.Z` is declared `<Props Qty="rad/s">` with a
    // display unit of rpm/deg/s. Its stored/base unit is `deg/s` (Angle uses
    // degrees, not radians) — NOT the display unit. Guards the real-corpus path
    // for the quantity→base-unit fix.
    let Some(proj_path) = project_path() else {
        eprintln!("project absent; skipping");
        return;
    };
    let project = Project::load(&proj_path).expect("load project");
    let sym = project
        .symbols()
        .get("Root.Vehicle.SBG.IMU.Gyro.Z")
        .expect("SBG gyro Z channel present in corpus");
    assert_eq!(sym.unit.as_deref(), Some("deg/s"));
}
