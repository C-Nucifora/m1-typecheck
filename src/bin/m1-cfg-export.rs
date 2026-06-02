//! `m1-cfg-export` — emit the expected `.m1cfg` parameter list for an M1 project.
//!
//! Reads a `Project.m1prj` (and, where available, an existing `.m1cfg` to enrich
//! parameter types), then writes every `BuiltIn.Parameter` as a `.m1cfg` skeleton.
//! Output is a starting point for a new calibration file, a diff target to find
//! drift, or — with `--missing-only` — just the parameters absent from an
//! existing `.m1cfg` (the high-value CI check).
//!
//! See C-Nucifora/m1-tools#10. The real `.m1cfg` is XML (`<Configuration><Group>
//! <Parameter><Cell Type=… Unit=…>`), with parameter names *un*prefixed (the cfg
//! lists `Foo.Bar`; the symbol model keys `Root.Foo.Bar`). `Security` lives in the
//! `.m1prj`, not the cfg, so it is only surfaced by the `json`/`csv` formats.
use clap::{Parser, ValueEnum};
use m1_typecheck::Project;
use m1_typecheck::symbols::{Symbol, SymbolKind};
use m1_typecheck::types::ValueType;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    about = "Export the expected .m1cfg parameter list from a MoTeC M1 project",
    version
)]
struct Cli {
    /// Path to the project's `Project.m1prj`.
    project: PathBuf,

    /// Write to this file instead of stdout.
    #[arg(long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Cfg)]
    format: Format,

    /// Existing `.m1cfg` to enrich parameter types from and to diff against.
    /// When omitted, a sibling `parameters.m1cfg` is auto-discovered (upward from
    /// the project) for type enrichment only.
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,

    /// Only emit parameters that are NOT already present in `--baseline`
    /// (which is then required). Ideal for CI drift checks.
    #[arg(long, requires = "baseline")]
    missing_only: bool,
}

#[derive(Copy, Clone, ValueEnum)]
enum Format {
    /// Real MoTeC `.m1cfg` XML.
    Cfg,
    /// JSON array of `{name, type, unit, security}`.
    Json,
    /// CSV with a `name,type,unit,security` header.
    Csv,
}

/// One exported parameter, already `Root.`-stripped and type-mapped.
struct ParamEntry {
    /// Unprefixed name, as a `.m1cfg` lists it (`Foo.Bar`, not `Root.Foo.Bar`).
    name: String,
    /// `.m1cfg` cell type (`f32`, `u32`, `enum`, …); `None` when unresolved.
    cell_type: Option<String>,
    unit: Option<String>,
    /// `.m1prj` security/access level. Not part of `.m1cfg`; for json/csv only.
    security: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(text) => {
            if let Some(path) = &cli.output {
                if let Err(e) = std::fs::write(path, text) {
                    eprintln!("m1-cfg-export: writing {}: {e}", path.display());
                    return ExitCode::FAILURE;
                }
            } else {
                print!("{text}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("m1-cfg-export: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<String, String> {
    let mut project = Project::load(&cli.project).map_err(|e| e.to_string())?;

    // Type-enrichment source: explicit --baseline, else an auto-discovered sibling
    // .m1cfg. Enum-valued parameters carry no Type in the .m1prj, so without a cfg
    // their cells stay unresolved.
    let enrich = cli.baseline.clone().or_else(|| {
        cli.project
            .parent()
            .and_then(m1_workspace::find_config_file)
    });
    if let Some(cfg) = &enrich {
        project = project.with_config(cfg).map_err(|e| e.to_string())?;
    }

    // For --missing-only, the names already present in the baseline.
    let baseline_names = match (cli.missing_only, &cli.baseline) {
        (true, Some(cfg)) => Some(read_param_names(cfg)?),
        _ => None,
    };

    let mut entries: Vec<ParamEntry> = project
        .symbols()
        .iter()
        .filter(|s| s.kind == SymbolKind::Parameter)
        .map(entry_for)
        .filter(|e| {
            baseline_names
                .as_ref()
                .is_none_or(|present| !present.contains(&e.name))
        })
        .collect();
    // Stable, diff-friendly ordering.
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(match cli.format {
        Format::Cfg => to_cfg(&entries),
        Format::Json => to_json(&entries),
        Format::Csv => to_csv(&entries),
    })
}

fn entry_for(sym: &Symbol) -> ParamEntry {
    ParamEntry {
        name: m1_workspace::strip_root(&sym.path).to_string(),
        cell_type: cell_type_for(sym),
        unit: sym.unit.clone(),
        security: sym.security.clone(),
    }
}

/// The `.m1cfg` cell `Type` for a parameter, preferring the exact type the
/// `.m1prj` declared and falling back to whatever the `.m1cfg` back-resolution
/// recovered. `None` only when the type is genuinely unknown from both — emitted
/// as a flagged placeholder rather than invented.
fn cell_type_for(sym: &Symbol) -> Option<String> {
    // 1. Exact storage type from the project — verbatim for primitives (so `s16`
    //    stays `s16`), `enum` for any enum reference the model couldn't pin to a
    //    specific type (e.g. `MoTeC Types.Mode Enumeration`, `::Hardware.…`).
    if let Some(declared) = sym.declared_type.as_deref() {
        return Some(cfg_type_from_declared(declared));
    }
    // 2. Type recovered from an existing `.m1cfg` (the enum `*.Value` idiom, where
    //    the `.m1prj` declares no type on the parameter itself).
    cfg_type_from_value(sym.value_type).map(str::to_string)
}

/// A declared `.m1prj` type → `.m1cfg` cell type: primitives verbatim, anything
/// else (an enum reference) → `enum`.
fn cfg_type_from_declared(declared: &str) -> String {
    if m1_typecheck::types::primitive_type(declared).is_some() {
        declared.to_string()
    } else {
        "enum".to_string()
    }
}

/// Map a [`ValueType`] to a canonical `.m1cfg` cell `Type`. `None` for types that
/// can't be expressed (unresolved/string). Widths are canonical here: `ValueType`
/// does not distinguish `f32`/`f64` or `u8`/`u32` — exact widths come from the
/// declared type instead (see [`cfg_type_from_declared`]).
fn cfg_type_from_value(vt: ValueType) -> Option<&'static str> {
    match vt {
        ValueType::Float => Some("f32"),
        ValueType::Unsigned => Some("u32"),
        ValueType::Integer => Some("s32"),
        ValueType::Boolean => Some("bool"),
        ValueType::Enum(_) => Some("enum"),
        ValueType::String | ValueType::Unknown => None,
    }
}

/// Parameter names declared in an existing `.m1cfg` (as written there — unprefixed).
fn read_param_names(cfg: &Path) -> Result<BTreeSet<String>, String> {
    let xml =
        std::fs::read_to_string(cfg).map_err(|e| format!("reading {}: {e}", cfg.display()))?;
    let doc = roxmltree::Document::parse(&xml)
        .map_err(|e| format!("invalid .m1cfg XML in {}: {e}", cfg.display()))?;
    Ok(doc
        .descendants()
        .filter(|n| n.has_tag_name("Parameter"))
        .filter_map(|n| n.attribute("Name"))
        .map(str::to_string)
        .collect())
}

fn to_cfg(entries: &[ParamEntry]) -> String {
    let mut out = String::from("<?xml version=\"1.0\"?>\n");
    out.push_str("<Configuration Locale=\"C\" DefaultLocale=\"C\">\n <Group Name=\"\">\n");
    for e in entries {
        out.push_str(&format!("  <Parameter Name=\"{}\">\n", xml_attr(&e.name)));
        match e.cell_type.as_deref() {
            Some(t) => {
                let unit = e
                    .unit
                    .as_deref()
                    .map(|u| format!(" Unit=\"{}\"", xml_attr(u)))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "   <Cell Type=\"{t}\"{unit}><![CDATA[]]></Cell>\n"
                ));
            }
            None => {
                // Type could not be resolved from the project (and no cfg supplied
                // a type). Flag it rather than invent one.
                out.push_str(
                    "   <!-- type unresolved from .m1prj; pass --baseline <.m1cfg> to fill -->\n",
                );
                out.push_str("   <Cell Type=\"\"><![CDATA[]]></Cell>\n");
            }
        }
        out.push_str("  </Parameter>\n");
    }
    out.push_str(" </Group>\n</Configuration>\n");
    out
}

fn to_json(entries: &[ParamEntry]) -> String {
    // Hand-built so the binary needs no extra derive; values are escaped.
    let mut out = String::from("[\n");
    for (i, e) in entries.iter().enumerate() {
        let comma = if i + 1 < entries.len() { "," } else { "" };
        out.push_str(&format!(
            "  {{\"name\": {}, \"type\": {}, \"unit\": {}, \"security\": {}}}{comma}\n",
            json_str(Some(&e.name)),
            json_str(e.cell_type.as_deref()),
            json_str(e.unit.as_deref()),
            json_str(e.security.as_deref()),
        ));
    }
    out.push_str("]\n");
    out
}

fn to_csv(entries: &[ParamEntry]) -> String {
    let mut out = String::from("name,type,unit,security\n");
    for e in entries {
        out.push_str(&format!(
            "{},{},{},{}\n",
            csv_field(&e.name),
            csv_field(e.cell_type.as_deref().unwrap_or("")),
            csv_field(e.unit.as_deref().unwrap_or("")),
            csv_field(e.security.as_deref().unwrap_or("")),
        ));
    }
    out
}

/// Escape a string for use inside an XML double-quoted attribute.
fn xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render an optional string as a JSON value (`null` or an escaped string).
fn json_str(s: Option<&str>) -> String {
    match s {
        None => "null".to_string(),
        Some(s) => {
            let escaped = s
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\t', "\\t");
            format!("\"{escaped}\"")
        }
    }
}

/// Quote a CSV field if it contains a comma, quote, or newline (RFC 4180).
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, ty: Option<&'static str>, unit: Option<&str>) -> ParamEntry {
        ParamEntry {
            name: name.to_string(),
            cell_type: ty.map(str::to_string),
            unit: unit.map(str::to_string),
            security: None,
        }
    }

    #[test]
    fn value_type_maps_to_cfg_cell_type() {
        assert_eq!(cfg_type_from_value(ValueType::Float), Some("f32"));
        assert_eq!(cfg_type_from_value(ValueType::Unsigned), Some("u32"));
        assert_eq!(cfg_type_from_value(ValueType::Integer), Some("s32"));
        assert_eq!(cfg_type_from_value(ValueType::Boolean), Some("bool"));
        assert_eq!(cfg_type_from_value(ValueType::Enum(0)), Some("enum"));
        assert_eq!(cfg_type_from_value(ValueType::Unknown), None);
    }

    #[test]
    fn declared_type_maps_primitives_verbatim_and_enums_to_enum() {
        // Exact storage widths are preserved, not canonicalised.
        assert_eq!(cfg_type_from_declared("s16"), "s16");
        assert_eq!(cfg_type_from_declared("u8"), "u8");
        assert_eq!(cfg_type_from_declared("f32"), "f32");
        // Any non-primitive declared type is an enum reference -> `enum`.
        assert_eq!(
            cfg_type_from_declared("MoTeC Types.Mode Enumeration"),
            "enum"
        );
        assert_eq!(
            cfg_type_from_declared("::Hardware.av_switch.sw_state"),
            "enum"
        );
        assert_eq!(cfg_type_from_declared("::This.Drive State"), "enum");
    }

    #[test]
    fn cfg_output_is_reparseable_and_matches_schema() {
        let entries = vec![
            e("Inputs.APPS.APPS2.Offset", Some("f32"), Some("V")),
            e("Outputs.Logging.LVLogging", Some("enum"), None),
        ];
        let cfg = to_cfg(&entries);
        // Round-trips through the same XML parser the importer uses.
        let doc = roxmltree::Document::parse(&cfg).expect("emitted cfg is valid XML");
        let params: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("Parameter"))
            .filter_map(|n| n.attribute("Name"))
            .collect();
        assert_eq!(
            params,
            ["Inputs.APPS.APPS2.Offset", "Outputs.Logging.LVLogging"]
        );
        let first_cell = doc.descendants().find(|n| n.has_tag_name("Cell")).unwrap();
        assert_eq!(first_cell.attribute("Type"), Some("f32"));
        assert_eq!(first_cell.attribute("Unit"), Some("V"));
    }

    #[test]
    fn unresolved_type_is_flagged_not_invented() {
        let cfg = to_cfg(&[e("Foo.Bar", None, None)]);
        assert!(cfg.contains("type unresolved"));
        assert!(cfg.contains("Type=\"\""));
    }

    #[test]
    fn xml_attribute_special_chars_are_escaped() {
        let cfg = to_cfg(&[e("A & B.\"x\"", Some("u32"), None)]);
        assert!(cfg.contains("A &amp; B.&quot;x&quot;"));
        assert!(roxmltree::Document::parse(&cfg).is_ok());
    }

    #[test]
    fn json_escapes_and_nulls() {
        let json = to_json(&[e("Foo.Bar", None, None)]);
        assert!(json.contains("\"type\": null"));
        assert!(json.contains("\"name\": \"Foo.Bar\""));
    }

    #[test]
    fn csv_quotes_fields_with_commas() {
        // A name containing a comma must be quoted to stay RFC-4180 valid.
        let csv = to_csv(&[e("Foo,Bar", Some("f32"), None)]);
        assert!(csv.contains("\"Foo,Bar\",f32,,"));
    }
}
