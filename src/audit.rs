//! Project-wide symbol-name audit (T050) against the m1-example naming conventions.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make_project_for};
use crate::project::Project;
use crate::symbols::SymbolKind;
use m1_core::Severity;

/// Audit the project's own symbol + enum-type names. Empty unless violations.
pub fn audit_project(project: &Project) -> Vec<TypeDiagnostic> {
    let table = project.symbols();
    let mut out = Vec::new();

    // path -> kind, to recognise symbols owned by a package-defined object
    // class instance (their member names are MoTeC's choice, not the user's).
    let kind_by_path: std::collections::HashMap<&str, SymbolKind> =
        table.iter().map(|s| (s.path.as_str(), s.kind)).collect();
    let owned_by_object = |path: &str| {
        // Walk every ancestor: a package-object instance anywhere up the
        // chain means this whole subtree is MoTeC-defined structure
        // (`TSAL Buzzer.Output.Pin.Drive.Value`).
        let mut p = path;
        while let Some((parent, _)) = p.rsplit_once('.') {
            if kind_by_path.get(parent) == Some(&SymbolKind::Object) {
                return true;
            }
            p = parent;
        }
        false
    };

    for sym in table.iter() {
        let leaf = leaf_of(&sym.path);
        // Members of a package-object instance (`Debounce.Value`,
        // `CAN Bus.Bus`) carry MoTeC-defined names — auditing them only
        // produces unactionable noise (#153).
        if owned_by_object(&sym.path) {
            continue;
        }
        // Manual p.64 "Naming Objects": every object name begins with an
        // uppercase letter (spaces between constituents are fine). The real
        // corpus agrees — only locals begin lowercase (the lint side, L016).
        // (#153: this previously demanded lowerCamelCase for channels/
        // functions/methods, which flooded real projects with warnings.)
        let conv = match sym.kind {
            SymbolKind::Channel
            | SymbolKind::Parameter
            | SymbolKind::Group
            | SymbolKind::Function
            | SymbolKind::Method => Some(("begin-with-uppercase", begins_uppercase(leaf))),
            SymbolKind::Constant => Some(("CAPITALISATION", capitalised(leaf))),
            _ => None,
        };
        if let Some((name, ok)) = conv
            && !ok
        {
            out.push(make_project_for(
                TypeCode::T050,
                Severity::Warning,
                format!(
                    "{:?} `{leaf}` does not follow {name} (in `{}`)",
                    sym.kind, sym.path
                ),
                &sym.path,
            ));
        }
    }
    for e in table.enums() {
        // Dotted enum names (`av_switch.sw_state`) are external package/
        // hardware types referenced via `::Module.…` — not the user's to name.
        if e.name.contains('.') {
            continue;
        }
        if !begins_uppercase(&e.name) {
            out.push(make_project_for(
                TypeCode::T050,
                Severity::Warning,
                format!(
                    "enum type `{}` does not begin with an uppercase letter",
                    e.name
                ),
                &e.name,
            ));
        }
    }
    out.extend(audit_name_collisions(project));
    out.extend(audit_classname_structure(project));
    out.extend(audit_type_restrictions(project));
    out
}

/// Project-wide data-type restriction audit (T087): the manual (p.24)
/// restricts Boolean and String data types to local variables — a channel or
/// parameter declared with a `bool`/`string` storage type cannot exist as an
/// ECU project object. CAN/DBC symbols (`BuiltIn.CAN.*`) are exempt: flag
/// signals are legitimately boolean at the bus level; the restriction is
/// about M1 project objects.
pub fn audit_type_restrictions(project: &Project) -> Vec<TypeDiagnostic> {
    use crate::types::ValueType;
    let mut out: Vec<TypeDiagnostic> = project
        .symbols()
        .iter()
        .filter(|s| matches!(s.kind, SymbolKind::Channel | SymbolKind::Parameter))
        .filter(|s| {
            !s.classname
                .as_deref()
                .is_some_and(|c| c.starts_with("BuiltIn.CAN."))
        })
        .filter(|s| matches!(s.value_type, ValueType::Boolean | ValueType::String))
        .map(|s| {
            make_project_for(
                TypeCode::T087,
                Severity::Warning,
                format!(
                    "{:?} `{}` is declared {:?} — Boolean/String data types are restricted to local variables",
                    s.kind, s.path, s.value_type
                ),
                &s.path,
            )
        })
        .collect();
    out.sort_by(|a, b| a.inner.message.cmp(&b.inner.message));
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
            out.push(make_project_for(
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
                &sym.path,
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
                out.push(make_project_for(
                    TypeCode::T071,
                    Severity::Warning,
                    format!(
                        "symbol names `{}` and `{}` differ only by case",
                        sorted[i], sorted[j]
                    ),
                    sorted[i],
                ));
            }
        }
    }

    // (b) project symbol leaf shadowing a library object name.
    let lib_names: Vec<&str> = crate::intrinsics::get().library_object_names().collect();
    for sym in table.iter() {
        let leaf = leaf_of(&sym.path);
        if let Some(obj) = lib_names.iter().find(|n| n.eq_ignore_ascii_case(leaf)) {
            out.push(make_project_for(
                TypeCode::T071,
                Severity::Warning,
                format!("symbol `{}` shadows the library object `{obj}`", sym.path),
                &sym.path,
            ));
        }
    }
    out
}

fn leaf_of(path: &str) -> &str {
    path.rsplit_once('.').map(|(_, l)| l).unwrap_or(path)
}

/// Manual p.64: an object name begins with an uppercase letter.
fn begins_uppercase(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

/// CAPITALISATION: letters are upper-case; spaces and digits allowed (per
/// CONTRIBUTING "Constant names may use spaces").
fn capitalised(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphabetic())
        && s.chars()
            .filter(|c| c.is_ascii_alphabetic())
            .all(|c| c.is_ascii_uppercase())
}
