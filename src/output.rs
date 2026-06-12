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
    if !d.related.is_empty() {
        // Secondary locations (#200). `project_line` is 0-based in the file
        // named by the document's `project_path`.
        out.push_str(",\"related\":[");
        for (i, r) in d.related.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let m1_typecheck::diagnostics::RelatedPlace::Project { line } = r.place;
            out.push_str(&format!("{{\"project_line\":{line},\"message\":"));
            out.push_str(&json_str(&r.message));
            out.push('}');
        }
        out.push(']');
    }
    out.push('}');
    out
}

/// SARIF 2.1.0 output (`--format sarif`, #185) — the interchange format GitHub
/// code scanning ingests natively, mirroring `m1-lint --format sarif`. One run;
/// one reportingDescriptor per T-code (plus the synthetic `syntax` rule); one
/// result per finding. Project-level diagnostics (zero-range audits such as
/// T041/T050/T092/T095) anchor to the project file at line 1 so every result
/// has a valid physical location.
pub fn render_sarif(files: &[JsonFile], project: &[TypeDiagnostic], project_label: &str) -> String {
    use m1_typecheck::diagnostics::TypeCode;
    use serde_json::json;

    fn level(s: Severity) -> &'static str {
        match s {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info | Severity::Hint => "note",
        }
    }

    let mut rules: Vec<serde_json::Value> = TypeCode::all_codes()
        .iter()
        .map(|c| {
            json!({
                "id": c.as_str(),
                "name": c.name(),
                "helpUri": format!("https://github.com/C-Nucifora/m1-typecheck#{}", c.name()),
            })
        })
        .collect();
    rules.push(json!({"id": "syntax", "name": "syntax-error"}));

    let mut results: Vec<serde_json::Value> = Vec::new();
    for f in files {
        for d in &f.syntax_errors {
            results.push(json!({
                "ruleId": "syntax",
                "level": "error",
                "message": {"text": d.message},
                "locations": [{"physicalLocation": {
                    "artifactLocation": {"uri": f.path},
                    "region": {
                        "startLine": d.range.start.line + 1,
                        "startColumn": d.range.start.column + 1,
                    },
                }}],
            }));
        }
        for d in &f.diagnostics {
            let mut result = json!({
                "ruleId": d.code.as_str(),
                "level": level(d.inner.severity),
                "message": {"text": d.inner.message},
                "locations": [{"physicalLocation": {
                    "artifactLocation": {"uri": f.path},
                    "region": {
                        "startLine": d.inner.range.start.line + 1,
                        "startColumn": d.inner.range.start.column + 1,
                        "endLine": d.inner.range.end.line + 1,
                        "endColumn": d.inner.range.end.column + 1,
                    },
                }}],
            });
            if !d.related.is_empty() {
                result["relatedLocations"] = related_sarif(&d.related, project_label);
            }
            results.push(result);
        }
    }
    for d in project {
        // Zero-range project audits anchor to the project file, line 1 — the
        // `+1` below maps their 0,0 range there naturally.
        let mut message = d.inner.message.clone();
        if let Some(subject) = &d.subject {
            // The subject symbol is part of the finding's identity; keep it in
            // the text since the anchor is the whole project file.
            if !message.contains(subject.as_str()) {
                message = format!("{subject}: {message}");
            }
        }
        results.push(json!({
            "ruleId": d.code.as_str(),
            "level": level(d.inner.severity),
            "message": {"text": message},
            "locations": [{"physicalLocation": {
                "artifactLocation": {"uri": project_label},
                "region": {
                    "startLine": d.inner.range.start.line + 1,
                    "startColumn": d.inner.range.start.column + 1,
                },
            }}],
        }));
    }

    fn related_sarif(
        related: &[m1_typecheck::diagnostics::RelatedLocation],
        project_label: &str,
    ) -> serde_json::Value {
        related
            .iter()
            .map(|r| {
                let m1_typecheck::diagnostics::RelatedPlace::Project { line } = r.place;
                json!({
                    "message": {"text": r.message},
                    "physicalLocation": {
                        "artifactLocation": {"uri": project_label},
                        "region": {"startLine": line + 1},
                    },
                })
            })
            .collect()
    }

    json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                "name": "m1-typecheck",
                "version": env!("CARGO_PKG_VERSION"),
                "informationUri": "https://github.com/C-Nucifora/m1-typecheck",
                "rules": rules,
            }},
            "results": results,
        }],
    })
    .to_string()
}
