//! Type-check diagnostic codes and result types.
use m1_core::{Code, Diagnostic, Node, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCode {
    T001, // unresolved-reference
    T002, // float-equality
    T003, // int-float-mixing
    T010, // local-hungarian-prefix
    T011, // hungarian-prefix-mismatch
}

impl TypeCode {
    pub fn as_str(self) -> &'static str {
        match self {
            TypeCode::T001 => "T001",
            TypeCode::T002 => "T002",
            TypeCode::T003 => "T003",
            TypeCode::T010 => "T010",
            TypeCode::T011 => "T011",
        }
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

#[derive(Debug, Default)]
pub struct CheckResult {
    pub diagnostics: Vec<TypeDiagnostic>,
    pub syntax_errors: Vec<Diagnostic>,
}
