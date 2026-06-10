//! Project-wide symbol-name audit (T050) against the m1-example naming conventions.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make_project_for};
use crate::project::Project;
use crate::symbols::{SymbolKind, SymbolTable};
use m1_core::Severity;

/// Recognises symbols owned by a package-defined object class instance —
/// their member names (and tags) are MoTeC's choice, not the user's, so the
/// audits exempt them. Shared by the naming (T050), tags (T092) and
/// local-case (T091) checks.
pub(crate) struct ObjectOwnership<'a> {
    /// path -> kind, so any ancestor can be tested for being an Object.
    kind_by_path: std::collections::HashMap<&'a str, SymbolKind>,
}

impl<'a> ObjectOwnership<'a> {
    pub(crate) fn new(table: &'a SymbolTable) -> Self {
        ObjectOwnership {
            kind_by_path: table.iter().map(|s| (s.path.as_str(), s.kind)).collect(),
        }
    }

    /// Walk every ancestor: a package-object instance anywhere up the
    /// chain means this whole subtree is MoTeC-defined structure
    /// (`TSAL Buzzer.Output.Pin.Drive.Value`).
    pub(crate) fn owned_by_object(&self, path: &str) -> bool {
        let mut p = path;
        while let Some((parent, _)) = p.rsplit_once('.') {
            if self.kind_by_path.get(parent) == Some(&SymbolKind::Object) {
                return true;
            }
            p = parent;
        }
        false
    }
}

/// Audit the project's own symbol + enum-type names. Empty unless violations.
pub fn audit_project(project: &Project) -> Vec<TypeDiagnostic> {
    let table = project.symbols();
    let mut out = Vec::new();

    let ownership = ObjectOwnership::new(table);

    for sym in table.iter() {
        let leaf = leaf_of(&sym.path);
        // Members of a package-object instance (`Debounce.Value`,
        // `CAN Bus.Bus`) carry MoTeC-defined names — auditing them only
        // produces unactionable noise (#153).
        if ownership.owned_by_object(&sym.path) {
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

/// Project-wide display-unit audit (T095, default-on) — M1 Build Error 1017
/// "Invalid display unit" parity.
///
/// Manual (*Quantities and Base Units*): every floating-point channel/parameter/
/// table has a physical *quantity*, and its `<Locale><Default Unit>` display unit
/// must be a unit *of that quantity* (conversion to the base unit is handled by
/// the M1 System). Choosing a display unit from a different dimension — e.g.
/// quantity `rad/s` (Angular Speed) displayed as `%` — is M1 Build Error 1017.
///
/// We mirror that exactly: for each symbol carrying both a `Qty` and a
/// `<Default Unit>`, look the quantity up in MoTeC's units database
/// ([`crate::unit_table`], generated from `units.xml`) and flag a display unit
/// that is not one of that quantity's units. A quantity absent from the database
/// can't be checked (skipped, no false positive); a symbol with no explicit
/// display unit shows in the quantity's base unit (always valid, skipped).
/// Package-object internals are exempt (their quantity/unit are MoTeC-defined,
/// same `ObjectOwnership` rule as the other audits).
///
/// Default-on: M1 Build emits 1017 as an Error, so this is parity, not noise.
pub fn audit_display_units(project: &Project) -> Vec<TypeDiagnostic> {
    let table = project.symbols();
    let ownership = ObjectOwnership::new(table);
    let mut out = Vec::new();
    for sym in table.iter() {
        if !matches!(
            sym.kind,
            SymbolKind::Channel | SymbolKind::Parameter | SymbolKind::Table
        ) {
            continue;
        }
        if ownership.owned_by_object(&sym.path) {
            continue;
        }
        let (Some(qty), Some(unit)) = (sym.qty.as_deref(), sym.display_unit.as_deref()) else {
            continue;
        };
        let Some(valid) = crate::unit_table::valid_display_units(qty) else {
            continue; // quantity not in MoTeC's units database — can't validate
        };
        if valid.contains(&unit) {
            continue; // a valid display unit for this quantity
        }
        out.push(make_project_for(
            TypeCode::T095,
            Severity::Error,
            format!(
                "{:?} `{}` has display unit `{unit}`, which is not a unit of its quantity `{qty}` (M1 Build Error 1017: \"Invalid display unit\")",
                sym.kind, sym.path
            ),
            &sym.path,
        ));
    }
    out.sort_by(|a, b| a.inner.message.cmp(&b.inner.message));
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

/// Project-level tags audit (T092, default-on) — M1 Build tag-warning parity.
///
/// Manual p.67 (*Tags*): every object can carry tags in three groups — System
/// (`Engine`/`Vehicle`/`Driver`), Type (`Normal`/`Diagnostic`/`Advanced`/
/// `Pin`/`Tune`/`Setup`) and the optional I/O group — and *"If a tag in this
/// group is not selected, M1 Build will emit a warning"* for the first two.
/// This mirrors those two warnings per user-defined Channel/Parameter, using
/// each symbol's **effective** tag set ([`crate::symbols::Symbol::tags`]: own
/// `SelectedTags` ∪ inherited group tags, #170). Package-object internals are
/// exempt (same `ObjectOwnership` rule as the naming audit).
///
/// Default-on: M1 Build emits exactly these warnings itself (the real AV-M1
/// project has hundreds), so surfacing them is parity, not noise. (Severity stays
/// Warning; teams that don't tag can `--ignore T092`.)
pub fn audit_tags(project: &Project) -> Vec<TypeDiagnostic> {
    const SYSTEM_TAGS: [&str; 3] = ["Engine", "Vehicle", "Driver"];
    const TYPE_TAGS: [&str; 6] = ["Normal", "Diagnostic", "Advanced", "Pin", "Tune", "Setup"];
    let table = project.symbols();
    let ownership = ObjectOwnership::new(table);
    let mut out = Vec::new();
    for sym in table.iter() {
        if !matches!(sym.kind, SymbolKind::Channel | SymbolKind::Parameter) {
            continue;
        }
        if ownership.owned_by_object(&sym.path) {
            continue;
        }
        let has_tag_in = |group: &[&str]| {
            sym.tags
                .iter()
                .any(|t| group.iter().any(|g| g.eq_ignore_ascii_case(t)))
        };
        if !has_tag_in(&SYSTEM_TAGS) {
            out.push(make_project_for(
                TypeCode::T092,
                Severity::Warning,
                format!(
                    "{:?} `{}` has no System tag selected (Engine/Vehicle/Driver) — M1 Build will warn",
                    sym.kind, sym.path
                ),
                &sym.path,
            ));
        }
        if !has_tag_in(&TYPE_TAGS) {
            out.push(make_project_for(
                TypeCode::T092,
                Severity::Warning,
                format!(
                    "{:?} `{}` has no Type tag selected (Normal/Diagnostic/Advanced/Pin/Tune/Setup) — M1 Build will warn",
                    sym.kind, sym.path
                ),
                &sym.path,
            ));
        }
    }
    // Deterministic order for stable CI output.
    out.sort_by(|a, b| a.inner.message.cmp(&b.inner.message));
    out
}

pub(crate) fn leaf_of(path: &str) -> &str {
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

#[cfg(test)]
mod display_unit_tests {
    use super::audit_display_units;
    use crate::diagnostics::TypeCode;
    use crate::project::Project;
    use m1_core::Severity;

    fn t095_subjects(xml: &str) -> Vec<String> {
        let project = Project::from_xml(xml).expect("parse project");
        audit_display_units(&project)
            .into_iter()
            .inspect(|d| {
                assert_eq!(d.code, TypeCode::T095);
                // M1 Build reports 1017 as an Error.
                assert_eq!(d.inner.severity, Severity::Error);
            })
            .filter_map(|d| d.subject)
            .collect()
    }

    #[test]
    fn flags_a_display_unit_from_the_wrong_dimension() {
        // Qty rad/s (Angular Speed) displayed as `%` is M1 Build Error 1017;
        // displayed as `rpm` (a unit of rad/s) is valid; with no display unit it
        // shows in the base unit (valid). A duty-cycle Qty `ratio` shown as `%`
        // is valid (`%` is a unit of `ratio`).
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Bad Speed">
    <Props Qty="rad/s"><Locale><Default Unit="%"/></Locale></Props>
  </Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Good Speed">
    <Props Qty="rad/s"><Locale><Default Unit="rpm"/></Locale></Props>
  </Component>
  <Component Classname="BuiltIn.Channel" Name="Root.No Display">
    <Props Qty="rad/s"/>
  </Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Duty">
    <Props Qty="ratio"><Locale><Default Unit="%"/></Locale></Props>
  </Component>
</Project>"#;
        assert_eq!(t095_subjects(xml), vec!["Root.Bad Speed".to_string()]);
    }

    #[test]
    fn skips_quantities_absent_from_the_units_database() {
        // A quantity symbol not in MoTeC's units.xml can't be validated — we must
        // not guess (no false positive), even with a nonsense display unit.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Weird">
    <Props Qty="zorkmids/fortnight"><Locale><Default Unit="banana"/></Locale></Props>
  </Component>
</Project>"#;
        assert!(t095_subjects(xml).is_empty());
    }

    #[test]
    fn temperature_celsius_and_fahrenheit_are_valid() {
        // Qty K (Temperature) accepts C and F (its base unit is Celsius); kPa is
        // a pressure unit and must flag.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.Parameter" Name="Root.Coolant">
    <Props Qty="K"><Locale><Default Unit="C"/></Locale></Props>
  </Component>
  <Component Classname="BuiltIn.Parameter" Name="Root.Ambient">
    <Props Qty="K"><Locale><Default Unit="F"/></Locale></Props>
  </Component>
  <Component Classname="BuiltIn.Parameter" Name="Root.Confused">
    <Props Qty="K"><Locale><Default Unit="kPa"/></Locale></Props>
  </Component>
</Project>"#;
        assert_eq!(t095_subjects(xml), vec!["Root.Confused".to_string()]);
    }
}
