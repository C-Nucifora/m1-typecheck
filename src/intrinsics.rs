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

use crate::types::ValueType;

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

/// Whether the known argument types can select this overload.
///
/// `Unknown` arguments remain viable: an incomplete project model must never
/// turn into a type error. Known arguments use the same widening rules as M1
/// assignment/function-call conversion (manual pp.43–46): integral values can
/// satisfy either integer spelling or a floating-point parameter, while a
/// floating-point value cannot narrow to an integer parameter.
///
/// The catalogue spells M1's paired numeric overloads as repeated
/// `Integer|FloatingPoint` parameters. Those occurrences are one shared
/// overload choice, not independent unions: for example, `Calculate.Max` is
/// `(Integer, Integer)` OR `(Floating Point, Floating Point)`. M1 Build rejects
/// a call that mixes those alternatives, so every known union argument in one
/// signature must select the same numeric family.
pub(crate) fn overload_accepts_args(overload: &Overload, args: &[ValueType]) -> bool {
    if overload.params.len() != args.len() {
        return false;
    }

    let mut union_is_float: Option<bool> = None;
    for (param, &arg) in overload.params.iter().zip(args) {
        if arg == ValueType::Unknown {
            continue;
        }
        let accepted = match param.ty.as_str() {
            "Integer" | "UnsignedInteger" => arg.is_integral(),
            "FloatingPoint" | "FixedPoint7dps" => arg.is_integral() || arg.is_float(),
            "Boolean" => arg == ValueType::Boolean,
            "String" => arg == ValueType::String,
            "Integer|FloatingPoint" if arg.is_integral() || arg.is_float() => {
                let is_float = arg.is_float();
                match union_is_float {
                    Some(selected) => selected == is_float,
                    None => {
                        union_is_float = Some(is_float);
                        true
                    }
                }
            }
            "Integer|FloatingPoint" => false,
            // Handle and any future firmware-specific types are not represented
            // in ValueType. Keep them opaque rather than guessing.
            _ => true,
        };
        if !accepted {
            return false;
        }
    }
    true
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

/// One member of a builtin (firmware/module) enumeration.
#[derive(Debug, Deserialize)]
pub struct EnumMember {
    pub name: String,
    pub value: i64,
    /// M1 Tune indicator (`Information` / `Warning` / `Fault`), when assigned.
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub doc: String,
}

/// A builtin enumeration: a MoTeC firmware/module enumerated data type with
/// its authoritative member set (from the M1 Build help captures). These are
/// the types M1 Build resolves and enforces membership/exhaustiveness on
/// (Errors 1306/1329/1352) that the `.m1prj` alone does not declare.
#[derive(Debug, Deserialize)]
pub struct EnumDef {
    pub name: String,
    pub members: Vec<EnumMember>,
}

/// Provenance of the embedded catalogue. The `target` names the firmware /
/// manual the intrinsics were captured from — the catalogue is not universal,
/// so callers can see (and, via `--firmware`, assert) which target they are
/// checking against. The remaining fields are documentation only.
#[derive(Debug, Deserialize, Default)]
pub struct Source {
    /// The firmware/manual target identifier, e.g. `m1-build-2026-06`. Empty
    /// only for a hand-written catalogue that predates keying; callers fall back
    /// to [`DEFAULT_TARGET`] in that case.
    #[serde(default)]
    pub target: String,
}

#[derive(Debug, Deserialize)]
pub struct Intrinsics {
    pub version: u32,
    /// Catalogue provenance, including the firmware/manual `target`.
    #[serde(default)]
    pub source: Source,
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
    /// Builtin firmware/module enumerations with authoritative membership
    /// (M1 Build help captures): registered into every project's symbol table
    /// so script-side literals (`Output Drive Enumeration.High Side`) resolve
    /// and the membership checks (T020/T021/T030/T070) fire as M1 Build does.
    #[serde(default)]
    pub enums: Vec<EnumDef>,
    /// Package class name -> help summary (first paragraph), for hover docs on
    /// project objects (`MoTeC Input.Sensor`, …).
    #[serde(default)]
    pub classes: HashMap<String, String>,
}

static INTRINSICS_JSON: &str = include_str!("../assets/m1-intrinsics.json");
static INTRINSICS: OnceLock<Intrinsics> = OnceLock::new();

/// The vendored intrinsic library, parsed once.
pub fn get() -> &'static Intrinsics {
    INTRINSICS.get_or_init(|| {
        serde_json::from_str(INTRINSICS_JSON).expect("vendored m1-intrinsics.json must be valid")
    })
}

/// Fallback target for a catalogue whose `source.target` is empty (a
/// hand-written catalogue predating firmware keying).
pub const DEFAULT_TARGET: &str = "m1-build-2026-06";

/// The firmware/manual target the embedded catalogue was captured from. This is
/// the *default* — and, until a second catalogue is vendored, the *only* —
/// target the checker can resolve against. Reads `source.target`, falling back
/// to [`DEFAULT_TARGET`].
pub fn active_target() -> &'static str {
    let t = get().source.target.as_str();
    if t.is_empty() { DEFAULT_TARGET } else { t }
}

/// Every catalogue target the binary can check against. One embedded catalogue
/// today, so a single-element list — the plumbing (`--firmware`, the error
/// listing) is in place for when a second target is vendored.
pub fn known_targets() -> Vec<&'static str> {
    vec![active_target()]
}

/// Resolve a user-requested `--firmware <target>` against the embedded
/// catalogue(s). `Ok` with the canonical target when it is known; `Err` with a
/// user-facing message that lists the known targets otherwise (the CLI exits 2
/// on that error — an unknown target must fail loud, never silently check
/// against the wrong firmware).
pub fn resolve_target(requested: &str) -> Result<&'static str, String> {
    known_targets()
        .into_iter()
        .find(|t| *t == requested)
        .ok_or_else(|| {
            format!(
                "unknown firmware target `{requested}`; known targets: {}",
                known_targets().join(", ")
            )
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
    /// The builtin enumeration named `name`, if the catalogue documents it.
    pub fn builtin_enum(&self, name: &str) -> Option<&EnumDef> {
        self.enums.iter().find(|e| e.name == name)
    }
    /// The help summary for package class `name` (`MoTeC Input.Sensor` matches
    /// its leaf `Sensor`… callers pass the class display name as captured).
    pub fn class_doc(&self, name: &str) -> Option<&str> {
        self.classes.get(name).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_has_the_library() {
        let i = get();
        // 13 ECU library objects + 2 calibration-only objects (Math, UI) + the
        // 8 libraries from the help-pane captures (J1939, LTC, MDD, MPSE,
        // Switch, TC, UnixTime, VCS).
        assert_eq!(i.library.len(), 23, "23 library objects");
        let total: usize = i.library.values().map(|o| o.functions.len()).sum();
        assert_eq!(total, 279, "279 library overloads");
        assert!(i.library_object("Calculate").is_some());
        assert!(i.library_object("CanComms").is_some());
        assert!(i.library_object("J1939").is_some(), "capture library loads");
        assert!(i.library_object("NotAnObject").is_none());
    }

    #[test]
    fn builtin_enum_catalogue_loads() {
        let i = get();
        assert!(i.enums.len() >= 130, "130 captured enumerations");
        let uss = i.builtin_enum("Universal Switch State").expect("USS");
        let names: Vec<&str> = uss.members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Off", "On"]);
        assert!(i.builtin_enum("No Such Enumeration").is_none());
        // Class help summaries load too.
        assert!(i.classes.len() >= 110, "110 captured classes");
        assert!(i.class_doc("Absolute Pressure").is_some());
    }

    #[test]
    fn capture_functions_have_signatures() {
        // A capture-sourced function carries a full signature for T064
        // arg-count checking and LSP signature help.
        let i = get();
        let sync = i.library_overloads("VCS", "Synchronise");
        assert!(!sync.is_empty());
        assert_eq!(sync[0].params.len(), 2);
        assert_eq!(sync[0].returns, "Integer");
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
    fn firmware_object_methods_present() {
        // CAN-signal and timer group-object accessors (#105): real firmware
        // methods (heavily used in real projects) that were previously
        // unmodelled, so hover fell through to "type not modelled".
        let i = get();
        let scaled = i.object_method("GetScaled");
        assert!(!scaled.is_empty(), "GetScaled modelled");
        assert_eq!(scaled[0].returns, "FloatingPoint");
        assert!(scaled[0].params.is_empty());

        let recv = i.object_method("Receive");
        assert!(!recv.is_empty(), "Receive modelled");
        assert_eq!(recv[0].returns, "Boolean");

        let remaining = i.object_method("Remaining");
        assert!(!remaining.is_empty(), "Remaining modelled");
        assert_eq!(remaining[0].returns, "FloatingPoint");

        // Timer control methods return Void (statement-position).
        for m in ["Start", "Stop", "Reset"] {
            let ov = i.object_method(m);
            assert!(!ov.is_empty(), "{m} modelled");
            assert_eq!(ov[0].returns, "Void", "{m} returns Void");
        }
    }

    #[test]
    fn catalogue_carries_a_firmware_target() {
        // The embedded catalogue is keyed by the firmware/manual it was captured
        // from (#260), so a consumer can see — and assert — which target it is
        // checking against rather than treating the intrinsics as universal.
        assert_eq!(active_target(), "m1-build-2026-06");
        assert_eq!(get().source.target, "m1-build-2026-06");
        assert_eq!(known_targets(), vec!["m1-build-2026-06"]);
    }

    #[test]
    fn resolve_target_accepts_known_and_rejects_unknown() {
        assert_eq!(
            resolve_target("m1-build-2026-06").unwrap(),
            "m1-build-2026-06"
        );
        let err = resolve_target("m1-build-2099-01").unwrap_err();
        // The error names the bad target AND lists what is known, so the user
        // can correct it (fail loud, never silently wrong-firmware).
        assert!(
            err.contains("m1-build-2099-01"),
            "names the bad target: {err}"
        );
        assert!(
            err.contains("m1-build-2026-06"),
            "lists known targets: {err}"
        );
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
