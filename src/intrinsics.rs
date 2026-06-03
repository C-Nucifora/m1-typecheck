//! The M1 built-in intrinsic library: the firmware global objects
//! (`Calculate`, `CanComms`, …), their method overloads, the integrated-only
//! object names, the language tables (keywords/operators/unsupported), and the
//! data-driven diagnostic rules.
//!
//! The data is `m1-intrinsics.json`, vendored from an M1 Build firmware export
//! (see `docs/OBJECTS.md` and the EV-M1 `M1-docs/M1-Intrinsics-LSP.md` spec). It
//! is the single source of truth — nothing here hard-codes symbol names.
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Deserialize)]
pub struct Param {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub doc: String,
}

/// One callable signature. A function with several signatures appears as several
/// `Overload`s with the same `name` (overloading is modelled by repetition).
#[derive(Debug, Deserialize)]
pub struct Overload {
    pub name: String,
    pub returns: String,
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub doc: String,
    /// Stateful (purple-icon) functions must be called every execution, never
    /// conditionally or inside a comparison (diagnostic `stateful-conditional`).
    #[serde(default)]
    pub stateful: bool,
    #[serde(default)]
    pub deprecated: bool,
    /// Calibration-only: valid only inside M1 Tune *calibration methods*, never
    /// in ECU `.m1scr` scripts. The manual's `Math.*`, `UI.*` and the calibration
    /// `System.*` functions are calibration-only. Not offered in ECU-script
    /// completion; surfaced (labelled) in hover.
    #[serde(default, rename = "calibrationOnly")]
    pub calibration_only: bool,
}

#[derive(Debug, Deserialize)]
pub struct LibraryObject {
    #[serde(default)]
    pub doc: String,
    #[serde(default)]
    pub functions: Vec<Overload>,
}

#[derive(Debug, Deserialize, Default)]
pub struct Language {
    /// role -> keywords (control / declaration / logical / comparison / literal / scope).
    #[serde(default)]
    pub keywords: HashMap<String, Vec<String>>,
    #[serde(default, rename = "scopeAnchors")]
    pub scope_anchors: HashMap<String, String>,
    #[serde(default)]
    pub operators: HashMap<String, String>,
    /// C token -> the M1 replacement message (diagnostic `unsupported-c-token`).
    #[serde(default)]
    pub unsupported: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct DiagRule {
    pub id: String,
    pub severity: String,
    pub rule: String,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Deserialize)]
pub struct Intrinsics {
    pub version: u32,
    #[serde(default, rename = "dataTypes")]
    pub data_types: Vec<String>,
    /// object name -> { doc, functions }: the 13 ECU-script library objects plus
    /// the calibration-only objects (`Math`, `UI`). Calibration-only functions
    /// carry `calibration_only` and must not be offered in ECU `.m1scr` scripts.
    #[serde(default)]
    pub library: HashMap<String, LibraryObject>,
    /// Methods available on project objects (AsInteger/Set/Validate/Lookup/…).
    #[serde(default, rename = "objectMethods")]
    pub object_methods: Vec<Overload>,
    /// Object names that exist in firmware but must not be called directly.
    #[serde(default, rename = "integratedOnly")]
    pub integrated_only: Vec<String>,
    #[serde(default)]
    pub language: Language,
    #[serde(default)]
    pub diagnostics: Vec<DiagRule>,
}

static INTRINSICS_JSON: &str = include_str!("../assets/m1-intrinsics.json");
static INTRINSICS: OnceLock<Intrinsics> = OnceLock::new();

/// The vendored intrinsic library, parsed once.
pub fn get() -> &'static Intrinsics {
    INTRINSICS.get_or_init(|| {
        serde_json::from_str(INTRINSICS_JSON).expect("vendored m1-intrinsics.json must be valid")
    })
}

impl Intrinsics {
    /// `Some(&'static name)` if `name` is one of the library objects.
    pub fn library_object_name(&'static self, name: &str) -> Option<&'static str> {
        self.library.get_key_value(name).map(|(k, _)| k.as_str())
    }
    pub fn library_object(&self, name: &str) -> Option<&LibraryObject> {
        self.library.get(name)
    }
    /// The names of the firmware library objects (`Calculate`, `CanComms`, …).
    pub fn library_object_names(&self) -> impl Iterator<Item = &str> {
        self.library.keys().map(String::as_str)
    }
    pub fn is_integrated_only(&self, name: &str) -> bool {
        self.integrated_only.iter().any(|n| n == name)
    }
    /// All overloads of `method` on library object `object`.
    pub fn library_overloads(&'static self, object: &str, method: &str) -> Vec<&'static Overload> {
        self.library
            .get(object)
            .map(|o| o.functions.iter().filter(|f| f.name == method).collect())
            .unwrap_or_default()
    }
    /// Overloads of a project-object method (AsInteger/Set/Lookup/…).
    pub fn object_method(&'static self, method: &str) -> Vec<&'static Overload> {
        self.object_methods
            .iter()
            .filter(|f| f.name == method)
            .collect()
    }
    /// If `token` is an unsupported C token, the M1 replacement message.
    pub fn unsupported_c_token(&self, token: &str) -> Option<&str> {
        self.language.unsupported.get(token).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_has_the_library() {
        let i = get();
        // 13 ECU library objects + 2 calibration-only objects (Math, UI).
        assert_eq!(i.library.len(), 15, "15 library objects");
        let total: usize = i.library.values().map(|o| o.functions.len()).sum();
        assert_eq!(total, 150, "150 library overloads");
        assert!(i.library_object("Calculate").is_some());
        assert!(i.library_object("CanComms").is_some());
        assert!(i.library_object("NotAnObject").is_none());
    }

    #[test]
    fn looks_up_overloads_and_stateful() {
        let i = get();
        let max = i.library_overloads("Calculate", "Max");
        assert!(!max.is_empty(), "Calculate.Max exists");
        assert_eq!(max[0].returns, "Integer|FloatingPoint");
        // Delay.Rising is a stateful (purple) function.
        let rising = i.library_overloads("Delay", "Rising");
        assert!(
            !rising.is_empty() && rising[0].stateful,
            "Delay.Rising is stateful"
        );
        // Calculate.Max is not stateful.
        assert!(!max[0].stateful);
    }

    #[test]
    fn integrated_only_and_unsupported_tokens() {
        let i = get();
        assert!(i.is_integrated_only("PDM"));
        assert!(!i.is_integrated_only("Calculate"));
        assert!(i.unsupported_c_token("==").is_some());
        assert!(i.unsupported_c_token("&&").is_some());
        assert!(i.unsupported_c_token("+").is_none());
    }

    #[test]
    fn object_methods_present() {
        let i = get();
        assert!(!i.object_method("AsInteger").is_empty());
        assert!(!i.object_method("Lookup").is_empty());
    }

    #[test]
    fn calibration_only_functions_are_present_and_flagged() {
        let i = get();
        // Calibration Maths / UI functions exist and are flagged calibration-only.
        let sqrt = i.library_overloads("Math", "Sqrt");
        assert!(!sqrt.is_empty(), "Math.Sqrt exists");
        assert!(sqrt[0].calibration_only, "Math.Sqrt is calibration-only");
        assert_eq!(sqrt[0].returns, "FloatingPoint");

        let isnan = i.library_overloads("Math", "IsNaN");
        assert!(!isnan.is_empty() && isnan[0].calibration_only);
        assert_eq!(isnan[0].returns, "Boolean");

        let prompt = i.library_overloads("UI", "PromptOK");
        assert!(!prompt.is_empty() && prompt[0].calibration_only);

        // System carries both ECU and calibration functions; StrCat is calibration-only.
        let strcat = i.library_overloads("System", "StrCat");
        assert!(!strcat.is_empty() && strcat[0].calibration_only);

        // ECU library functions are NOT calibration-only.
        let abs = i.library_overloads("Calculate", "Absolute");
        assert!(
            !abs[0].calibration_only,
            "Calculate.Absolute is ECU, not calibration"
        );
    }
}
