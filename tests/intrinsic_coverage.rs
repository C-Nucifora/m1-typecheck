//! Intrinsic-catalogue coverage report + T064 safety-boundary pins (#206).
//!
//! T064 `wrong-argument-count` stays **opt-in** because the vendored intrinsic
//! catalogue (`assets/m1-intrinsics.json`) does not model every firmware
//! method the real corpora call. The corpus-gated report below tracks that gap
//! so the graduation decision is data-driven: run it with
//!
//! ```sh
//! cargo test --test intrinsic_coverage -- --nocapture
//! ```
//!
//! against a corpus (sibling `m1-example/` or `M1_CORPUS_PATH`). It prints
//! counts and method names only — no corpus statements or expressions — and
//! never fails on coverage; the always-on tests pin the rule's safety
//! boundary (modelled methods are checked, unmodelled ones never flag).
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script_with;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

// ---- always-on: the T064 safety boundary ------------------------------------

fn fixture() -> Project {
    Project::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantics.m1prj"))
        .unwrap()
}

fn t064(src: &str) -> bool {
    let enabled: HashSet<String> = ["T064".to_string()].into();
    let project = fixture();
    check_script_with(
        &enabled,
        Some(&project),
        Some(Path::new("Ctrl.Alpha.m1scr")),
        src,
    )
    .diagnostics
    .iter()
    .any(|d| d.code == m1_typecheck::diagnostics::TypeCode::T064)
}

#[test]
fn modelled_method_is_arity_checked() {
    // `Calculate.Min` is in the catalogue; no overload takes zero arguments.
    assert!(
        t064("A Out = Calculate.Min();\n"),
        "modelled method checked"
    );
    assert!(
        !t064("A Out = Calculate.Min(1, 2);\n"),
        "valid arity is clean"
    );
}

#[test]
fn unmodelled_method_stays_opaque_and_unchecked() {
    // An unknown member of a known library object resolves Opaque, never
    // BuiltinFn, so T064 must not fire whatever the arity — the boundary that
    // keeps the rule safe on firmware methods the catalogue doesn't model.
    // (A fabricated name: real once-missing examples keep graduating into the
    // catalogue — `System.FlashFree` did via the help-pane captures.)
    assert!(
        !t064("A Out = Calculate.Frobnicate(1, 2, 3, 4, 5);\n"),
        "unmodelled method must never flag"
    );
    // And a whole object outside the catalogue is opaque too.
    assert!(
        !t064("A Out = Frobnitz.Method(1, 2, 3);\n"),
        "unknown object must never flag"
    );
}

// ---- corpus-gated coverage report -------------------------------------------

fn corpus_dirs() -> Vec<PathBuf> {
    if let Some(p) = std::env::var_os("M1_CORPUS_PATH") {
        return vec![PathBuf::from(p)];
    }
    ["../m1-example/UQR-EV/01.00/Scripts", "../AV-M1/UQR-AV"]
        .iter()
        .map(|p| Path::new(env!("CARGO_MANIFEST_DIR")).join(p))
        .filter(|p| p.is_dir())
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

/// Collect `Head.Method` heads of every call whose callee is a two-segment
/// member path — the shape library calls take. Project-symbol heads are
/// excluded later by catalogue lookup (we only report on the catalogue side).
fn call_heads(src: &str, out: &mut BTreeMap<String, usize>) {
    let cst = m1_core::parse(src);
    let mut stack = vec![cst.root()];
    while let Some(n) = stack.pop() {
        if n.kind() == m1_core::Kind::CallExpression
            && let Some(callee) = n
                .named_children()
                .into_iter()
                .find(|c| c.kind() == m1_core::Kind::MemberExpression)
        {
            let segs: Vec<_> = callee
                .named_children()
                .into_iter()
                .filter(|c| c.kind() == m1_core::Kind::Identifier)
                .map(|c| c.text().to_string())
                .collect();
            if segs.len() == 2 {
                *out.entry(format!("{}.{}", segs[0], segs[1])).or_default() += 1;
            }
        }
        for c in n.children() {
            stack.push(c);
        }
    }
}

#[test]
fn corpus_intrinsic_coverage_report() {
    let dirs = corpus_dirs();
    if dirs.is_empty() {
        eprintln!("corpus absent; skipping coverage report");
        return;
    }
    let mut heads: BTreeMap<String, usize> = BTreeMap::new();
    let mut scripts = Vec::new();
    for d in &dirs {
        scripts_under(d, &mut scripts);
    }
    for s in &scripts {
        if let Ok(src) = m1_workspace::read_text(s) {
            call_heads(&src, &mut heads);
        }
    }
    let intr = m1_typecheck::intrinsics::get();
    let known_objects: BTreeSet<&str> = intr.library_object_names().collect();

    let (mut modelled, mut method_gaps, mut object_gaps) = (0usize, Vec::new(), Vec::new());
    let mut call_sites_modelled = 0usize;
    let mut call_sites_total = 0usize;
    for (head, count) in &heads {
        let (obj, method) = head.split_once('.').unwrap();
        call_sites_total += count;
        if !known_objects.contains(obj) {
            object_gaps.push(head.as_str());
            continue;
        }
        if intr.library_overloads(obj, method).is_empty() {
            method_gaps.push(head.as_str());
        } else {
            modelled += 1;
            call_sites_modelled += count;
        }
    }

    eprintln!("== intrinsic coverage ({} scripts) ==", scripts.len());
    eprintln!(
        "distinct two-segment call heads: {} (modelled {modelled}, unmodelled-method {}, unknown-object {})",
        heads.len(),
        method_gaps.len(),
        object_gaps.len()
    );
    eprintln!("call sites: {call_sites_modelled}/{call_sites_total} modelled");
    eprintln!("unmodelled methods on known objects: {method_gaps:?}");
    eprintln!("heads with objects outside the catalogue: {object_gaps:?}");
    eprintln!(
        "(unknown-object heads include project symbols, which are not intrinsics; \
         the T064 graduation question is the unmodelled-method list)"
    );
    assert!(!heads.is_empty(), "corpus present but no calls found");
}
