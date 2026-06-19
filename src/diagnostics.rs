//! Type-check diagnostic codes and result types.
use m1_core::{Code, Diagnostic, Node, Position, Range, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCode {
    T001, // unresolved-reference
    T002, // float-equality
    T003, // int-float-mixing
    T004, // signed-unsigned-comparison
    T005, // modulo-on-float (the `%` operator is integer-only; manual p.35)
    T006, // bitwise-on-float (the `& | ^ ~ << >>` operators are integer-only; manual p.37)
    T010, // invalid-component-parent (project audit; child class under a disallowed parent class)
    T020, // enum-non-member
    T021, // enum-numeric-comparison
    T030, // assignment-type-mismatch
    T031, // unresolved-assignment-target
    T040, // channel-multiple-assignment
    T041, // missing-cfg-parameter (project audit; in .m1prj, absent from .m1cfg)
    T042, // dbc-signal-range (literal assigned to a CAN signal is out of range)
    T050, // symbol-name-convention (project audit)
    T060, // stateful-conditional
    T061, // integrated-only-call
    T062, // deprecated-overload
    T063, // calibration-only-call
    T064, // wrong-argument-count (opt-in)
    T070, // when-is-exhaustive
    T071, // name-case-collision (project audit)
    T080, // invalid-value-reaches-finite-sink (NaN/Inf provenance; @requires-finite/@safety-critical)
    T081, // nan-to-integer (a possibly NaN/Inf value is stored to an integer channel)
    T082, // when-subject-not-enum (manual p.32: the when argument must be an enumerated type)
    T083, // static-local-initialiser (manual p.34: must be a literal, enumerator or constant)
    T084, // expand-bounds (manual p.33: literals or constants, 0 or positive)
    T085, // user-argument-mismatch (call arity/types vs the function's declared Signature params)
    T086, // unit-mismatch (assignment copies a value of a different physical quantity)
    T087, // type-restricted-to-locals (project audit; manual p.24: Boolean/String channels)
    T088, // circular-dependency (same-rate write/read cycle between scripts; manual p.29)
    T089, // rate-inversion (opt-in: a faster script reads a channel written only at a slower rate)
    T090, // expression-nesting-too-deep (analysis skipped to avoid a stack overflow)
    T091, // local-object-case-ambiguity (a local's name matches a referenced object leaf modulo case; manual pp.64-65)
    T092, // untagged-component (M1 Build parity 1142/1549: no System / Type tag selected, manual p.67)
    T093, // unassigned-channel (M1 Build Error 1627: a channel no script writes)
    T094, // unread-parameter (M1 Build Error 1631: a parameter no script reads)
    T095, // invalid-display-unit (M1 Build Error 1017: display unit not valid for the quantity)
    T096, // multiple-scheduled-writers (M1 Build Error 1022: channel assigned by >1 periodically scheduled function)
    T097, // recursive-call (cycle in the user-function call graph, incl. a function calling itself)
    T098, // unused-argument (M1 Build Error 1355: a declared `<Param>` never read via `In.<name>`)
    T099, // return-value-not-assigned (M1 Build Error 1353: `<Signature ReturnType>` set but no `Out =`)
    T100, // bare-parameter-reference (M1 Build Error 1338: a param referenced by bare name, not `In.<name>`)
    T101, // return-statement (M1 Build Error 1338: a C-style `return <expr>`; M1 has no `return` keyword)
    T102, // channel-multiple-assignment-cross-function (M1 Build Error 1317: a channel reset in a caller and written by a callee on one path)
    T103, // ambiguous-reference (M1 Build Error 1339: a bare name matching both a sibling channel and an enum type)
    T104, // unscheduled-function (M1 Build Error 1642: a user function no scheduled function reaches)
    T105, // local-use-before-definition (manual p.34: a local may only be used after it has been defined)
    T106, // group-not-a-value-provider (M1 Build Error 1331: a group with no Default Value used where a value is expected)
}

impl TypeCode {
    pub fn as_str(self) -> &'static str {
        match self {
            TypeCode::T001 => "T001",
            TypeCode::T002 => "T002",
            TypeCode::T003 => "T003",
            TypeCode::T004 => "T004",
            TypeCode::T005 => "T005",
            TypeCode::T006 => "T006",
            TypeCode::T010 => "T010",
            TypeCode::T020 => "T020",
            TypeCode::T021 => "T021",
            TypeCode::T030 => "T030",
            TypeCode::T031 => "T031",
            TypeCode::T040 => "T040",
            TypeCode::T041 => "T041",
            TypeCode::T042 => "T042",
            TypeCode::T050 => "T050",
            TypeCode::T060 => "T060",
            TypeCode::T061 => "T061",
            TypeCode::T062 => "T062",
            TypeCode::T063 => "T063",
            TypeCode::T064 => "T064",
            TypeCode::T070 => "T070",
            TypeCode::T071 => "T071",
            TypeCode::T080 => "T080",
            TypeCode::T081 => "T081",
            TypeCode::T082 => "T082",
            TypeCode::T083 => "T083",
            TypeCode::T084 => "T084",
            TypeCode::T085 => "T085",
            TypeCode::T086 => "T086",
            TypeCode::T087 => "T087",
            TypeCode::T088 => "T088",
            TypeCode::T089 => "T089",
            TypeCode::T090 => "T090",
            TypeCode::T091 => "T091",
            TypeCode::T092 => "T092",
            TypeCode::T093 => "T093",
            TypeCode::T094 => "T094",
            TypeCode::T095 => "T095",
            TypeCode::T096 => "T096",
            TypeCode::T097 => "T097",
            TypeCode::T098 => "T098",
            TypeCode::T099 => "T099",
            TypeCode::T100 => "T100",
            TypeCode::T101 => "T101",
            TypeCode::T102 => "T102",
            TypeCode::T103 => "T103",
            TypeCode::T104 => "T104",
            TypeCode::T105 => "T105",
            TypeCode::T106 => "T106",
        }
    }

    /// The kebab-case rule name (e.g. `unresolved-reference`). The single source
    /// of truth for tools that enumerate the type-check rules — mirrors
    /// `m1_lint::diagnostic::LintCode::name` so editors can list L- and T-codes
    /// the same way (config scaffolding, rule pickers).
    pub fn name(self) -> &'static str {
        match self {
            TypeCode::T001 => "unresolved-reference",
            TypeCode::T002 => "float-equality",
            TypeCode::T003 => "int-float-mixing",
            TypeCode::T004 => "signed-unsigned-comparison",
            TypeCode::T005 => "modulo-on-float",
            TypeCode::T006 => "bitwise-on-float",
            TypeCode::T010 => "invalid-component-parent",
            TypeCode::T020 => "enum-non-member",
            TypeCode::T021 => "enum-numeric-comparison",
            TypeCode::T030 => "assignment-type-mismatch",
            TypeCode::T031 => "unresolved-assignment-target",
            TypeCode::T040 => "channel-multiple-assignment",
            TypeCode::T041 => "missing-cfg-parameter",
            TypeCode::T042 => "dbc-signal-range",
            TypeCode::T050 => "symbol-name-convention",
            TypeCode::T060 => "stateful-conditional",
            TypeCode::T061 => "integrated-only-call",
            TypeCode::T062 => "deprecated-overload",
            TypeCode::T063 => "calibration-only-call",
            TypeCode::T064 => "wrong-argument-count",
            TypeCode::T070 => "when-is-exhaustive",
            TypeCode::T071 => "name-case-collision",
            TypeCode::T080 => "invalid-value-reaches-finite-sink",
            TypeCode::T081 => "nan-to-integer",
            TypeCode::T082 => "when-subject-not-enum",
            TypeCode::T083 => "static-local-initialiser",
            TypeCode::T084 => "expand-bounds",
            TypeCode::T085 => "user-argument-mismatch",
            TypeCode::T086 => "unit-mismatch",
            TypeCode::T087 => "type-restricted-to-locals",
            TypeCode::T088 => "circular-dependency",
            TypeCode::T089 => "rate-inversion",
            TypeCode::T090 => "expression-nesting-too-deep",
            TypeCode::T091 => "local-object-case-ambiguity",
            TypeCode::T092 => "untagged-component",
            TypeCode::T093 => "unassigned-channel",
            TypeCode::T094 => "unread-parameter",
            TypeCode::T095 => "invalid-display-unit",
            TypeCode::T096 => "multiple-scheduled-writers",
            TypeCode::T097 => "recursive-call",
            TypeCode::T098 => "unused-argument",
            TypeCode::T099 => "return-value-not-assigned",
            TypeCode::T100 => "bare-parameter-reference",
            TypeCode::T101 => "return-statement",
            TypeCode::T102 => "channel-multiple-assignment-cross-function",
            TypeCode::T103 => "ambiguous-reference",
            TypeCode::T104 => "unscheduled-function",
            TypeCode::T105 => "local-use-before-definition",
            TypeCode::T106 => "group-not-a-value-provider",
        }
    }

    /// Every type-check code, in declaration order — the catalogue external tools
    /// enumerate (config scaffold, diagnostic filters). Mirrors
    /// `m1_lint::diagnostic::LintCode::all_codes`. The `all_codes_match_enum`
    /// test guards against drift from the enum.
    pub fn all_codes() -> &'static [TypeCode] {
        use TypeCode::*;
        &[
            T001, T002, T003, T004, T005, T006, T010, T020, T021, T030, T031, T040, T041, T042,
            T050, T060, T061, T062, T063, T064, T070, T071, T080, T081, T082, T083, T084, T085,
            T086, T087, T088, T089, T090, T091, T092, T093, T094, T095, T096, T097, T098, T099,
            T100, T101, T102, T103, T104, T105, T106,
        ]
    }
}

#[derive(Debug, Clone)]
pub struct TypeDiagnostic {
    pub code: TypeCode,
    pub inner: Diagnostic,
    /// The project symbol a project-level (zero-range) diagnostic is about
    /// (`Root.Engine.Speed`), enabling symbol-scoped suppression via
    /// `[diagnostics] ignore_symbols` (#151). `None` for source-anchored
    /// diagnostics — those are suppressed in-source with `@m1:allow`.
    pub subject: Option<String>,
    /// Secondary locations of a two-location diagnostic (#200): T030 points
    /// at the assignment but the declared type lives on a `.m1prj`
    /// `<Component>`; T085's signature and T086's unit likewise. Empty for
    /// single-location diagnostics. The CLI prints these as `note:` lines and
    /// the LSP maps them to `DiagnosticRelatedInformation`.
    pub related: Vec<RelatedLocation>,
}

/// A secondary location a two-location diagnostic points at (#200).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedLocation {
    pub place: RelatedPlace,
    /// What the other end *is* — "declared here", "signature declared here".
    pub message: String,
}

/// Where a [`RelatedLocation`] lives. Rules only know a symbol's `def_line`,
/// not which file the project was loaded from — the consumer (CLI, LSP) knows
/// the project path and resolves `Project` to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelatedPlace {
    /// A 0-based line of the project file (`Project.m1prj` or a `.m1dbc` —
    /// the line numbering follows [`crate::symbols::Symbol::def_line`]).
    Project { line: u32 },
}

/// The related-location for a symbol's declaration site, if the symbol came
/// from the project file ([`crate::symbols::Symbol::def_line`] is `None` otherwise).
pub fn related_to_def(
    sym: &crate::symbols::Symbol,
    message: impl Into<String>,
) -> Option<RelatedLocation> {
    Some(RelatedLocation {
        place: RelatedPlace::Project {
            line: sym.def_line?,
        },
        message: message.into(),
    })
}

/// Build a `TypeDiagnostic` spanning `node`. A type diagnostic's specific
/// identity is carried by [`TypeDiagnostic::code`] (the `T0xx` `TypeCode`); the
/// inner [`m1_core::Code`] is only the broad *category*, so it is set to
/// `Code::TypeError` — the category m1-core defines for type-checker findings.
/// (It was previously the `Code::SyntaxError` placeholder, which mislabelled
/// every type finding as a syntax error; no consumer reads `inner.code`, so this
/// corrects the category without changing behaviour.)
pub fn make(code: TypeCode, node: &Node, severity: Severity, message: String) -> TypeDiagnostic {
    TypeDiagnostic {
        code,
        inner: Diagnostic {
            range: node.range(),
            byte_range: node.byte_range(),
            severity,
            code: Code::TypeError,
            message,
        },
        subject: None,
        related: Vec::new(),
    }
}

/// Build a project-level `TypeDiagnostic` (no source node; zero range).
pub fn make_project(code: TypeCode, severity: Severity, message: String) -> TypeDiagnostic {
    let zero = Position { line: 0, column: 0 };
    TypeDiagnostic {
        code,
        inner: Diagnostic {
            range: Range {
                start: zero,
                end: zero,
            },
            byte_range: 0..0,
            severity,
            code: Code::TypeError,
            message,
        },
        subject: None,
        related: Vec::new(),
    }
}

/// [`make_project`] with the symbol the finding is about, so it can be
/// suppressed per-symbol via `[diagnostics] ignore_symbols` (#151).
pub fn make_project_for(
    code: TypeCode,
    severity: Severity,
    message: String,
    subject: impl Into<String>,
) -> TypeDiagnostic {
    TypeDiagnostic {
        subject: Some(subject.into()),
        ..make_project(code, severity, message)
    }
}

#[derive(Debug, Default)]
pub struct CheckResult {
    pub diagnostics: Vec<TypeDiagnostic>,
    pub syntax_errors: Vec<Diagnostic>,
}

#[cfg(test)]
mod tests {
    use super::TypeCode;

    #[test]
    fn all_codes_match_enum_and_have_names() {
        // Every catalogue entry round-trips through as_str/name, and the codes are
        // unique — so all_codes() can't silently drift from the TypeCode enum.
        let codes = TypeCode::all_codes();
        let mut seen = std::collections::HashSet::new();
        for &c in codes {
            assert!(seen.insert(c.as_str()), "duplicate code {}", c.as_str());
            assert!(c.as_str().starts_with('T'));
            assert!(!c.name().is_empty());
            assert!(
                c.name()
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch == '-')
            );
        }
        // Catalogue size tracks the enum (bump both together).
        assert_eq!(codes.len(), 49);
    }
}
