//! Project-wide symbol-name audit (T050) against the m1-example naming conventions.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make_project};
use crate::project::Project;
use crate::symbols::SymbolKind;
use m1_core::Severity;

/// Audit the project's own symbol + enum-type names. Empty unless violations.
pub fn audit_project(project: &Project) -> Vec<TypeDiagnostic> {
    let table = project.symbols();
    let mut out = Vec::new();

    for sym in table.iter() {
        let leaf = leaf_of(&sym.path);
        let conv = match sym.kind {
            SymbolKind::Channel => Some(("lowerCamelCase", lower_camel(leaf))),
            SymbolKind::Parameter | SymbolKind::Group => {
                Some(("UpperCamelCase", upper_camel(leaf)))
            }
            SymbolKind::Constant => Some(("CAPITALISATION", capitalised(leaf))),
            SymbolKind::Function | SymbolKind::Method => {
                Some(("lowerCamelCase", lower_camel(leaf)))
            }
            _ => None,
        };
        if let Some((name, ok)) = conv
            && !ok
        {
            out.push(make_project(
                TypeCode::T050,
                Severity::Warning,
                format!(
                    "{:?} `{leaf}` does not follow {name} (in `{}`)",
                    sym.kind, sym.path
                ),
            ));
        }
    }
    for e in table.enums() {
        if !upper_camel(&e.name) {
            out.push(make_project(
                TypeCode::T050,
                Severity::Warning,
                format!("enum type `{}` does not follow UpperCamelCase", e.name),
            ));
        }
    }
    out.extend(audit_name_collisions(project));
    out.extend(audit_classname_structure(project));
    out
}

/// Project-wide component-class structure audit (T010): a child component whose
/// `Classname` is only valid under a specific parent class but whose actual
/// parent has a different class. Driven by [`crate::classname`], which is
/// deliberately conservative — only classes with an unambiguous parent invariant
/// (the CAN frame hierarchy) are constrained, so well-formed projects never flag.
pub fn audit_classname_structure(project: &Project) -> Vec<TypeDiagnostic> {
    use std::collections::HashMap;
    let table = project.symbols();
    let mut out = Vec::new();

    // path -> classname, so a child can look up its parent's class.
    let class_by_path: HashMap<&str, &str> = table
        .iter()
        .filter_map(|s| Some((s.path.as_str(), s.classname.as_deref()?)))
        .collect();

    for sym in table.iter() {
        let Some(child_class) = sym.classname.as_deref() else {
            continue;
        };
        // Parent path = the symbol path with its last dotted segment removed.
        let Some((parent_path, _)) = sym.path.rsplit_once('.') else {
            continue; // top-level (e.g. `Root`): no parent to constrain against.
        };
        let Some(&parent_class) = class_by_path.get(parent_path) else {
            continue; // parent isn't a known component; nothing to check.
        };
        if let Some(allowed) = crate::classname::parent_violation(child_class, parent_class) {
            out.push(make_project(
                TypeCode::T010,
                Severity::Warning,
                format!(
                    "component `{}` has class `{child_class}` but its parent `{parent_path}` \
                     has class `{parent_class}` — expected parent class {}",
                    sym.path,
                    allowed
                        .iter()
                        .map(|c| format!("`{c}`"))
                        .collect::<Vec<_>>()
                        .join(" or "),
                ),
            ));
        }
    }
    out
}

/// Project-wide name-case audit (T071):
///   (a) two project symbols whose fully-qualified dotted paths are equal
///       case-insensitively but differ in actual case — reported once per pair;
///   (b) a project symbol whose leaf name case-insensitively matches a firmware
///       library object name (`Calculate`, `CanComms`, …), which it would shadow.
pub fn audit_name_collisions(project: &Project) -> Vec<TypeDiagnostic> {
    use std::collections::HashMap;
    let table = project.symbols();
    let mut out = Vec::new();

    // (a) case-insensitive path collisions. Group the distinct (case-preserving)
    //     paths by their lower-cased form; any group with >1 distinct spelling is
    //     a collision. Report each unordered pair once.
    let mut by_lower: HashMap<String, Vec<&str>> = HashMap::new();
    for sym in table.iter() {
        let entry = by_lower.entry(sym.path.to_ascii_lowercase()).or_default();
        if !entry.contains(&sym.path.as_str()) {
            entry.push(sym.path.as_str());
        }
    }
    for variants in by_lower.values() {
        if variants.len() < 2 {
            continue;
        }
        let mut sorted = variants.clone();
        sorted.sort_unstable();
        for i in 0..sorted.len() {
            for j in (i + 1)..sorted.len() {
                out.push(make_project(
                    TypeCode::T071,
                    Severity::Warning,
                    format!(
                        "symbol names `{}` and `{}` differ only by case",
                        sorted[i], sorted[j]
                    ),
                ));
            }
        }
    }

    // (b) project symbol leaf shadowing a library object name.
    let lib_names: Vec<&str> = crate::intrinsics::get().library_object_names().collect();
    for sym in table.iter() {
        let leaf = leaf_of(&sym.path);
        if let Some(obj) = lib_names.iter().find(|n| n.eq_ignore_ascii_case(leaf)) {
            out.push(make_project(
                TypeCode::T071,
                Severity::Warning,
                format!("symbol `{}` shadows the library object `{obj}`", sym.path),
            ));
        }
    }
    out
}

fn leaf_of(path: &str) -> &str {
    path.rsplit_once('.').map(|(_, l)| l).unwrap_or(path)
}

fn lower_camel(s: &str) -> bool {
    let Some(c) = s.chars().next() else {
        return false;
    };
    c.is_ascii_lowercase() && !s.contains(' ') && !s.contains('_')
}
fn upper_camel(s: &str) -> bool {
    let Some(c) = s.chars().next() else {
        return false;
    };
    c.is_ascii_uppercase() && !s.contains(' ') && !s.contains('_')
}
/// CAPITALISATION: letters are upper-case; spaces and digits allowed (per
/// CONTRIBUTING "Constant names may use spaces").
fn capitalised(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphabetic())
        && s.chars()
            .filter(|c| c.is_ascii_alphabetic())
            .all(|c| c.is_ascii_uppercase())
}
