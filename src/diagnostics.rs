//! Type-check diagnostic codes and result types.
use m1_core::{Code, Diagnostic, Node, Position, Range, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCode {
    T001, // unresolved-reference
    T002, // float-equality
    T003, // int-float-mixing
    T004, // signed-unsigned-comparison
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
}

impl TypeCode {
    pub fn as_str(self) -> &'static str {
        match self {
            TypeCode::T001 => "T001",
            TypeCode::T002 => "T002",
            TypeCode::T003 => "T003",
            TypeCode::T004 => "T004",
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
        }
    }

    /// Every type-check code, in declaration order — the catalogue external tools
    /// enumerate (config scaffold, diagnostic filters). Mirrors
    /// `m1_lint::diagnostic::LintCode::all_codes`. The `all_codes_match_enum`
    /// test guards against drift from the enum.
    pub fn all_codes() -> &'static [TypeCode] {
        use TypeCode::*;
        &[
            T001, T002, T003, T004, T010, T020, T021, T030, T031, T040, T041, T042, T050, T060,
            T061, T062, T063, T064, T070, T071,
        ]
    }
}

#[derive(Debug, Clone)]
pub struct TypeDiagnostic {
    pub code: TypeCode,
    pub inner: Diagnostic,
}

/// Build a `TypeDiagnostic` spanning `node`. The inner `Code` field is unused by
/// type rules (they carry identity in `TypeDiagnostic::code`); it is set to
/// `Code::SyntaxError` as a placeholder.
pub fn make(code: TypeCode, node: &Node, severity: Severity, message: String) -> TypeDiagnostic {
    TypeDiagnostic {
        code,
        inner: Diagnostic {
            range: node.range(),
            byte_range: node.byte_range(),
            severity,
            code: Code::SyntaxError,
            message,
        },
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
            code: Code::SyntaxError,
            message,
        },
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
        assert_eq!(codes.len(), 20);
    }
}
