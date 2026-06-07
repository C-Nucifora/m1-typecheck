//! Hand-rolled JSON rendering for the `m1-typecheck` CLI's `--format json`.
//!
//! Extracted from `main.rs` so the rendering (document shape, escaping, range
//! encoding) lives apart from argument parsing and the check pipeline. The output
//! is shaped like `m1-lint --format json`:
//! `{"version":1,"files":[{path,syntax_errors,diagnostics}],"project":[…],"summary":{…}}`.
//!
//! Deliberately dependency-free string building (no `serde_json`) to keep the
//! exact byte-for-byte shape the existing CLI tests assert.

use m1_core::Severity;
use m1_typecheck::diagnostics::TypeDiagnostic;

/// One file's buffered diagnostics for the JSON document.
pub struct JsonFile {
    pub path: String,
    pub syntax_errors: Vec<m1_core::Diagnostic>,
    pub diagnostics: Vec<TypeDiagnostic>,
}

/// The lower-case label for a severity, used in both human and JSON output.
pub fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
        Severity::Hint => "hint",
    }
}

/// Minimal JSON escaping for a string value (incl. control chars).
pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn range_json(range: &m1_core::Range, byte: &std::ops::Range<usize>) -> String {
    format!(
        ",\"range\":{{\"start\":{{\"line\":{},\"column\":{}}},\"end\":{{\"line\":{},\"column\":{}}}}},\"byte_range\":{{\"start\":{},\"end\":{}}}",
        range.start.line,
        range.start.column,
        range.end.line,
        range.end.column,
        byte.start,
        byte.end
    )
}

/// Machine-parsable diagnostics document, shaped like `m1-lint --format json`:
/// `{"version":1,"files":[{path,syntax_errors,diagnostics}],"project":[…],"summary":{…}}`.
pub fn render_json(files: &[JsonFile], project: &[TypeDiagnostic], project_label: &str) -> String {
    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut out = String::from("{\"version\":1,\"files\":[");
    for (fi, f) in files.iter().enumerate() {
        if fi > 0 {
            out.push(',');
        }
        out.push_str("{\"path\":");
        out.push_str(&json_str(&f.path));
        out.push_str(",\"syntax_errors\":[");
        for (i, d) in f.syntax_errors.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            errors += 1;
            out.push_str("{\"code\":\"syntax\",\"severity\":");
            out.push_str(&json_str(severity_str(d.severity)));
            out.push_str(",\"message\":");
            out.push_str(&json_str(&d.message));
            out.push_str(&range_json(&d.range, &d.byte_range));
            out.push('}');
        }
        out.push_str("],\"diagnostics\":[");
        for (i, d) in f.diagnostics.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            match d.inner.severity {
                Severity::Error => errors += 1,
                Severity::Warning => warnings += 1,
                _ => {}
            }
            out.push_str(&diag_json(d));
        }
        out.push_str("]}");
    }
    out.push_str("],\"project\":[");
    for (i, d) in project.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match d.inner.severity {
            Severity::Error => errors += 1,
            Severity::Warning => warnings += 1,
            _ => {}
        }
        out.push_str(&diag_json(d));
    }
    out.push_str("],\"project_path\":");
    out.push_str(&json_str(project_label));
    out.push_str(&format!(
        ",\"summary\":{{\"errors\":{errors},\"warnings\":{warnings},\"files\":{}}}}}",
        files.len()
    ));
    out
}

fn diag_json(d: &TypeDiagnostic) -> String {
    let mut out = String::from("{\"code\":");
    out.push_str(&json_str(d.code.as_str()));
    out.push_str(",\"name\":");
    out.push_str(&json_str(d.code.name()));
    out.push_str(",\"severity\":");
    out.push_str(&json_str(severity_str(d.inner.severity)));
    out.push_str(",\"message\":");
    out.push_str(&json_str(&d.inner.message));
    out.push_str(&range_json(&d.inner.range, &d.inner.byte_range));
    out.push('}');
    out
}
