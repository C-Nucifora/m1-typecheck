mod output;

use clap::{Parser, ValueEnum};
use m1_core::Severity;
use m1_typecheck::cross_script::{self, ChannelTaints};
use m1_typecheck::diagnostics::{TypeCode, TypeDiagnostic};
use m1_typecheck::filter::DiagFilter;
use m1_typecheck::project::Project;
use m1_typecheck::project_check::{ProjectCheckOptions, SourceCheck, SourceInput};
use output::{JsonFile, json_str, related_path_and_line, render_json, render_sarif, severity_str};
use std::path::{Path, PathBuf};
use std::process;

/// Every `.m1scr` under the project's revision directory (the folder containing
/// `Project.m1prj`), as `(file_name, source)` — the COMPLETE script set the
/// project-wide cross-script passes (T088/T089/T093/T094, taint solve, return-type
/// inference) need to be sound regardless of which files were passed on the command
/// line. Discovery goes through the shared hardened walk
/// ([`m1_workspace::find_scripts`]: symlink-skip, depth cap, sorted — #190).
/// Rooted at the open revision dir, so sibling backup revisions are not included.
fn gather_project_scripts(project_path: &Path) -> Vec<(String, String)> {
    let Some(root) = project_path.parent() else {
        return Vec::new();
    };
    m1_workspace::find_scripts(root)
        .iter()
        .filter_map(|f| {
            let name = f.file_name()?.to_str()?.to_string();
            let src = m1_workspace::read_text(f).ok()?;
            Some((name, src))
        })
        .collect()
}

#[derive(Parser, Debug)]
#[command(
    name = "m1-typecheck",
    version,
    about = "Type checker for MoTeC M1 scripts"
)]
struct Args {
    /// Scripts to check
    files: Vec<PathBuf>,
    /// Project.m1prj (defaults to nearest upward, or $M1_PROJECT)
    #[arg(long)]
    project: Option<PathBuf>,
    /// parameters.m1cfg (optional)
    #[arg(long)]
    config: Option<PathBuf>,
    /// Audit the project's own symbol names against the naming conventions (T050).
    #[arg(long)]
    audit_names: bool,
    /// Run only these T-codes (comma-separated, e.g. `T002,T030`).
    #[arg(long, value_delimiter = ',')]
    select: Option<Vec<String>>,
    /// Suppress these T-codes (comma-separated, e.g. `T041`).
    #[arg(long, value_delimiter = ',')]
    ignore: Option<Vec<String>>,
    /// Suppress a project-level code for one symbol (comma-separated
    /// `CODE:Symbol.Path` entries, e.g. `T050:Root.Engine.Speed`). Overlays
    /// the `[diagnostics] ignore_symbols` list from m1-tools.toml.
    #[arg(
        long = "ignore-symbol",
        value_name = "CODE:PATH",
        value_delimiter = ','
    )]
    ignore_symbol: Option<Vec<String>>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Human)]
    format: Format,
    /// Print the T-code catalogue (honours --format json) and exit.
    #[arg(long)]
    rules: bool,
    /// Report analysis *completeness* rather than findings: how much of the
    /// project the checker could pin down (expressions typed, references
    /// resolved, intrinsic calls catalogued, enum membership verified) versus
    /// the silent `Unknown`/opaque surface. Honours `--format json`, then exits
    /// 0. A clean run is not the same as a complete one; this shows the gap.
    #[arg(long)]
    completeness: bool,
    /// Intrinsic-catalogue firmware/manual target to check against. The embedded
    /// catalogue (`m1-build-2026-06`) is the default; an unknown target fails
    /// loud, listing the known ones (never a silent wrong-firmware check). The
    /// active target is reported by `--completeness`.
    #[arg(long, value_name = "TARGET")]
    firmware: Option<String>,
    /// Explain a channel's physical quantity: its declared base unit and the
    /// unit of every symbol directly assigned into it across the project's
    /// scripts (the whole auto-discovered set, like --explain — the positional
    /// FILES do not narrow it), then exit (#144's units sibling of --explain).
    #[arg(long, value_name = "CHANNEL")]
    explain_units: Option<String>,
    /// Explain a channel's project-wide invalid-value (NaN/Inf) status: print
    /// the solved cross-script taint and its backward provenance chain, then
    /// exit. Accepts the canonical `Root.`-qualified or the bare channel path;
    /// pass every project script as FILES so the solve sees the whole graph.
    #[arg(long, value_name = "CHANNEL")]
    explain: Option<String>,
    /// Treat any finding as a failure: exit 1 if the run produces any diagnostic,
    /// not only error-severity ones. For warnings-clean CI once errors are at
    /// zero (matches m1-lint/m1-fmt, which fail on anything to report).
    #[arg(long)]
    strict: bool,
    /// Drop warning-severity findings entirely — not shown, and not counted
    /// toward the exit status (even under --strict). Lets a team gate on errors
    /// while triaging warnings.
    #[arg(short = 'W', long = "no-warnings")]
    no_warnings: bool,
}

/// How a finding is treated under `--strict` / `--no-warnings`. `drop` removes it
/// from output entirely (a `--no-warnings` warning); `fails` is whether it should
/// fail the run (errors always; under `--strict`, anything still shown). The two
/// are derived together so every report site stays consistent (#199).
fn finding_disposition(args: &Args, severity: Severity) -> (bool, bool) {
    let drop = args.no_warnings && severity == Severity::Warning;
    let fails = !drop && (args.strict || severity == Severity::Error);
    (drop, fails)
}

/// Emit a batch of project-level (zero-range) diagnostics through the one shared
/// disposition path: `allows_subject` filtering, `finding_disposition`
/// (`--strict`/`--no-warnings`), then either buffering into `json_buf.project`
/// or human-printing under the project-path label. Returns whether any kept
/// finding should fail the run. Owns the diagnostics so the JSON path moves them
/// straight in (no clone). Every project-level report site routes through here so
/// the disposition stays uniform — previously the `--audit-names` (T050) block
/// had a hand-edited copy that bypassed `finding_disposition`, so T050 warnings
/// neither failed under `--strict` nor dropped under `--no-warnings`.
fn emit_project_diags(
    args: &Args,
    diags: impl IntoIterator<Item = TypeDiagnostic>,
    filter: &DiagFilter,
    project_path: Option<&Path>,
    json: bool,
    json_buf: &mut JsonBuf,
) -> bool {
    let mut had_error = false;
    let path_label = || {
        project_path
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<project>".into())
    };
    for d in diags {
        if !filter.allows_subject(d.code.as_str(), d.subject.as_deref()) {
            continue;
        }
        let (drop, fails) = finding_disposition(args, d.inner.severity);
        if drop {
            continue;
        }
        had_error |= fails;
        if json {
            json_buf.project.push(d);
        } else {
            println!(
                "{}: {}[{}]: {}",
                path_label(),
                severity_str(d.inner.severity),
                d.code.as_str(),
                d.inner.message
            );
        }
    }
    had_error
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Format {
    Human,
    Json,
    Sarif,
}

/// Print the T-code catalogue (the single source of truth tools enumerate),
/// mirroring `m1-lint --rules`. JSON: `{"version":1,"rules":[{code,name},…]}`.
fn print_rules(format: Format) {
    match format {
        // SARIF is a findings format, not a catalogue one; the JSON catalogue
        // is the machine-readable shape either way.
        Format::Json | Format::Sarif => {
            let mut out = String::from("{\"version\":1,\"rules\":[");
            for (i, code) in TypeCode::all_codes().iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&format!(
                    "{{\"code\":{},\"name\":{}}}",
                    json_str(code.as_str()),
                    json_str(code.name())
                ));
            }
            out.push_str("]}");
            println!("{out}");
        }
        Format::Human => {
            for code in TypeCode::all_codes() {
                println!("{}  {}", code.as_str(), code.name());
            }
        }
    }
}

/// Print the analysis-completeness report (`--completeness`). SARIF is a
/// findings format, so it falls back to the JSON coverage document; human mode
/// prints a readable summary. The report is telemetry, never a gate — this
/// always precedes an exit 0.
fn print_completeness(report: &m1_typecheck::completeness::CompletenessReport, format: Format) {
    match format {
        Format::Json | Format::Sarif => println!("{}", output::render_completeness(report)),
        Format::Human => {
            let r = report;
            println!("analysis completeness report");
            println!(
                "  scripts:         {} analysed ({} total, {} syntax-error, {} too-deep)",
                r.scripts_analysed(),
                r.scripts_total,
                r.scripts_with_syntax_errors,
                r.scripts_skipped_deep
            );
            println!(
                "  expressions:     {}/{} typed ({:.1}% known)",
                r.expressions_typed,
                r.expressions_total,
                r.typed_percent()
            );
            println!(
                "  references:      {}/{} resolved ({:.1}%); {} opaque, {} unresolved",
                r.references_resolved,
                r.references_total,
                r.resolved_percent(),
                r.references_opaque,
                r.references_unresolved
            );
            println!(
                "  intrinsic calls: {}/{} unmodelled (method not in the catalogue)",
                r.intrinsic_calls_unmodelled, r.intrinsic_calls_total
            );
            println!(
                "  when subjects:   {}/{} with unverifiable enum membership",
                r.when_subjects_incomplete, r.when_subjects_total
            );
            println!(
                "  inputs:          .m1cfg {}, .m1dbc {}",
                if r.cfg_loaded { "loaded" } else { "absent" },
                if r.dbc_loaded { "loaded" } else { "absent" },
            );
            println!(
                "  catalogue:       {} (firmware/manual target)",
                r.catalogue_target
            );
            if !r.cfg_loaded {
                println!("  note: no .m1cfg — calibration coverage (T041) not checked");
            }
            if !r.dbc_loaded {
                println!("  note: no .m1dbc — CAN signal checks (T042/T108–T110) not exercised");
            }
        }
    }
}

/// Trim each `--select`/`--ignore` token and drop the empty ones, so an empty
/// value (`--ignore ""`), a whitespace-only value, or an empty entry from a
/// trailing/double comma (`T002,`) is treated as "no code" rather than an
/// unknown empty code. Mirrors m1-lint's `split_codes` tolerance. A genuinely
/// unknown non-empty code still reaches `validate_codes` and errors.
fn normalize_codes(codes: &mut Option<Vec<String>>) {
    if let Some(v) = codes {
        v.iter_mut().for_each(|c| *c = c.trim().to_string());
        v.retain(|c| !c.is_empty());
    }
}

/// Reject unknown T-codes in `--select`/`--ignore` up front; a silent typo used
/// to disable nothing (#111). Exits with code 2 on the first unknown code.
fn validate_codes(args: &Args) {
    let known: std::collections::HashSet<&str> =
        TypeCode::all_codes().iter().map(|c| c.as_str()).collect();
    for code in args
        .select
        .iter()
        .flatten()
        .chain(args.ignore.iter().flatten())
    {
        if !known.contains(code.as_str()) {
            eprintln!("m1-typecheck: unknown diagnostic code `{code}` (see --rules)");
            std::process::exit(2);
        }
    }
}

/// Buffered diagnostics for `--format json`: per-file results plus project-level
/// audits, rendered as one document at the end.
#[derive(Default)]
struct JsonBuf {
    files: Vec<JsonFile>,
    project: Vec<TypeDiagnostic>,
}

/// Load the project model (when a `Project.m1prj` was located), layering in the
/// `parameters.m1cfg` and any discovered `.m1dbc` files, then emit the degraded-run
/// notes (project-less / no-cfg / no-dbc) so a green CI run is not mistaken for
/// "all clean". A bad project/config exits with code 2; a malformed DBC is warned
/// and skipped without blanking the model.
///
/// Returns the loaded project (if any) alongside whether a `.m1dbc` was
/// discovered — the completeness report needs that flag and it cannot be
/// re-derived after loading (the DBC symbols merge indistinguishably into the
/// table).
fn load_project(
    project_path: Option<&PathBuf>,
    config_path: Option<&PathBuf>,
) -> (Option<Project>, bool) {
    // Track whether any `.m1dbc` was discovered so a project that loads but finds
    // none can announce that T042 is skipped (mirroring the cfg/project notes).
    let mut dbc_found = false;
    let project = project_path.map(|path| match Project::load(path) {
        Ok(mut p) => {
            if let Some(cfg) = config_path {
                p = p.with_config(cfg).unwrap_or_else(|e| {
                    eprintln!("m1-typecheck: config {}: {e}", cfg.display());
                    process::exit(2);
                });
            }
            // Auto-load `.m1dbc` files under the project directory so CAN signals
            // resolve and the T042 range check applies. Sorted for stable output.
            if let Some(root) = path.parent() {
                for dbc in m1_workspace::find_dbc_files(root) {
                    dbc_found = true;
                    let rel = dbc
                        .strip_prefix(root)
                        .unwrap_or(&dbc)
                        .to_string_lossy()
                        .into_owned();
                    // A malformed/unreadable DBC must not blank the whole model:
                    // warn and skip just that file, keeping every other symbol.
                    if let Err(e) = p.augment_dbc(&dbc, &rel) {
                        eprintln!("m1-typecheck: dbc {} skipped: {e}", dbc.display());
                    }
                }
            }
            p
        }
        Err(e) => {
            eprintln!("m1-typecheck: project {}: {e}", path.display());
            process::exit(2);
        }
    });
    if project.is_none() {
        eprintln!("m1-typecheck: no project found; running in project-less mode (T001 disabled)");
    } else {
        // A project loaded but no cfg/dbc was discovered: the cfg/dbc-dependent
        // audits (T041 calibration-coverage, T042 dbc-signal-range) then quietly
        // produce nothing. Announce the degraded run so CI green isn't mistaken
        // for "all clean" — mirrors the project-less note above (#87).
        if config_path.is_none() {
            eprintln!(
                "m1-typecheck: note: no parameters.m1cfg found; T041 (calibration-coverage) skipped"
            );
        }
        if !dbc_found {
            eprintln!("m1-typecheck: note: no .m1dbc found; T042 (dbc-signal-range) skipped");
        }
    }
    (project, dbc_found)
}

/// Print or buffer source results returned by the reusable project pipeline.
/// Returns `true` if any syntax or kept error-severity diagnostic was seen.
fn emit_source_checks(
    args: &Args,
    project_path: Option<&Path>,
    checks: Vec<SourceCheck>,
    mut json_buf: Option<&mut JsonBuf>,
) -> bool {
    let json = json_buf.is_some();
    let mut had_error = false;
    for check in checks {
        let path = check
            .path
            .as_deref()
            .map(Path::display)
            .map(|display| display.to_string())
            .unwrap_or_else(|| "<inline>".to_string());
        // Syntax errors are not T-coded and always fail the run; the filter only
        // governs the T-code diagnostics.
        for diagnostic in &check.syntax_errors {
            had_error = true;
            if !json {
                println!(
                    "{}:{}:{}: error[syntax]: {}",
                    path,
                    diagnostic.range.start.line + 1,
                    diagnostic.range.start.column + 1,
                    diagnostic.message
                );
            }
        }
        let kept: Vec<TypeDiagnostic> = check
            .diagnostics
            .into_iter()
            // --no-warnings drops warning findings from both output and JSON (#199).
            .filter(|diagnostic| !finding_disposition(args, diagnostic.inner.severity).0)
            .collect();
        for diagnostic in &kept {
            had_error |= finding_disposition(args, diagnostic.inner.severity).1;
            if !json {
                println!(
                    "{}:{}:{}: {}[{}]: {}",
                    path,
                    diagnostic.inner.range.start.line + 1,
                    diagnostic.inner.range.start.column + 1,
                    severity_str(diagnostic.inner.severity),
                    diagnostic.code.as_str(),
                    diagnostic.inner.message
                );
                // Two-location diagnostics carry the other end (#200) — the
                // declaration in the project or DBC file, rustc-note style.
                for related in &diagnostic.related {
                    let project_label = project_path
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "<project>".to_string());
                    let (label, line) = related_path_and_line(&related.place, &project_label);
                    println!("    note: {}: {label}:{}", related.message, line + 1);
                }
            }
        }
        if let Some(buf) = json_buf.as_deref_mut() {
            buf.files.push(JsonFile {
                path,
                syntax_errors: check.syntax_errors,
                diagnostics: kept,
            });
        }
    }
    had_error
}

/// Render the buffered JSON document to stdout (JSON mode only). Human-mode
/// diagnostics are printed as they are produced, so this is a no-op there.
fn emit_output(project_path: Option<&PathBuf>, format: Format, json_buf: &JsonBuf) {
    if format == Format::Human {
        return;
    }
    let label = project_path
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<project>".into());
    let doc = match format {
        Format::Json => render_json(&json_buf.files, &json_buf.project, &label),
        Format::Sarif => render_sarif(&json_buf.files, &json_buf.project, &label),
        Format::Human => unreachable!(),
    };
    println!("{doc}");
}

/// `--explain <CHANNEL>`: print the channel's solved invalid-value status —
/// the backward provenance chain when tainted, a clean note otherwise. A
/// query, not a check: exits 0 either way; 2 when the project or the channel
/// is missing.
/// `--explain-units`: print a channel's base unit and the units of symbols
/// directly assigned into it across the supplied scripts (#144).
fn explain_units(
    project: Option<&Project>,
    scripts: &[m1_typecheck::parsed::ParsedScript],
    channel: &str,
) {
    let Some(p) = project else {
        eprintln!("m1-typecheck: --explain-units needs a project");
        process::exit(2);
    };
    let canonical = if channel.starts_with("Root.") {
        channel.to_string()
    } else {
        format!("Root.{channel}")
    };
    let Some(sym) = p.symbols().get(&canonical) else {
        eprintln!("m1-typecheck: unknown channel `{channel}`");
        process::exit(2);
    };
    match &sym.unit {
        Some(u) => println!("{canonical}: base unit {u}"),
        None => println!("{canonical}: unitless (no Qty declared)"),
    }
    // Each contributing symbol is listed once. A dotted path's CST yields both
    // the full `MemberExpression` and its inner segments (a bare property like
    // `Sensor` resolves group-relative to the same symbol), so resolving every
    // descendant node would emit a contributor more than once (#223).
    let mut seen = std::collections::HashSet::new();
    for script in scripts {
        let file = &script.name;
        let cst = &script.cst;
        if !cst.syntax_diagnostics().is_empty() {
            continue;
        }
        let group = p.group_for_script(file);
        let fn_symbol = p.function_symbol_for_script(file);
        let scope = m1_typecheck::resolve::Scope {
            locals: std::collections::HashMap::new(),
            group,
            project: Some(p),
            fn_symbol,
        };
        for n in cst.root().descendants() {
            if n.kind() != m1_core::Kind::AssignmentStatement {
                continue;
            }
            let kids = n.named_children();
            let Some(target) = kids.iter().find(|c| m1_typecheck::typer::is_expr(c.kind())) else {
                continue;
            };
            let resolved =
                m1_typecheck::resolve::resolve(&m1_typecheck::typer::path_text(*target), &scope);
            let m1_typecheck::resolve::Resolution::Symbol(t) = resolved else {
                continue;
            };
            if t.path != canonical {
                continue;
            }
            // List every symbol read on the value side with its unit.
            for c in kids.iter().filter(|c| !std::ptr::eq(*c, target)) {
                // `descendants()` is pre-order and already yields `c` itself,
                // so iterate it alone (chaining `once(*c)` in front double-
                // counted `c`); `seen` collapses the remaining segment-level
                // duplicates (#223).
                for d in c.descendants() {
                    if !matches!(
                        d.kind(),
                        m1_core::Kind::Identifier | m1_core::Kind::MemberExpression
                    ) {
                        continue;
                    }
                    if let m1_typecheck::resolve::Resolution::Symbol(s) =
                        m1_typecheck::resolve::resolve(&m1_typecheck::typer::path_text(d), &scope)
                    {
                        let line = format!(
                            "  <- {} ({}) in {} line {}",
                            s.path,
                            s.unit.as_deref().unwrap_or("unitless"),
                            file,
                            n.range().start.line + 1
                        );
                        if seen.insert(line.clone()) {
                            println!("{line}");
                        }
                    }
                }
            }
        }
    }
}

fn explain_channel(project: Option<&Project>, taints: &ChannelTaints, channel: &str, json: bool) {
    let Some(p) = project else {
        eprintln!("m1-typecheck: --explain needs a project (no Project.m1prj found)");
        process::exit(2);
    };
    let Some(ex) = cross_script::explain(p, taints, channel) else {
        eprintln!("m1-typecheck: --explain: unknown channel `{channel}` (not in the project)");
        process::exit(2);
    };
    if json {
        let chain: &[String] = ex.taint.as_ref().map(|t| t.chain.as_slice()).unwrap_or(&[]);
        let mut s = format!(
            "{{\"version\":1,\"explain\":{{\"channel\":{},\"invalid\":{},\"latched\":{},\"chain\":[",
            json_str(&ex.channel),
            ex.taint.is_some(),
            ex.taint.as_ref().is_some_and(|t| t.latched),
        );
        for (i, step) in chain.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&json_str(step));
        }
        s.push_str("]}}");
        println!("{s}");
    } else {
        match &ex.taint {
            None => println!(
                "{}: no invalid-value (NaN/Inf) path found — finite as far as the \
                 cross-script analysis can see",
                ex.channel
            ),
            Some(t) => {
                println!("{}: can be NaN/Inf", ex.channel);
                for step in &t.chain {
                    println!("  {step}");
                }
                if t.latched {
                    println!(
                        "  latched: the value flows through a stateful function \
                         (Filter/Integral/Derivative) that holds NaN after the input recovers"
                    );
                }
            }
        }
    }
}

fn find_project(args: &Args) -> Option<PathBuf> {
    if let Some(p) = &args.project {
        return Some(p.clone());
    }
    if let Some(p) = std::env::var_os("M1_PROJECT") {
        return Some(PathBuf::from(p));
    }
    let start = args.files.first()?;
    m1_workspace::find_project_file(start.parent()?)
}

/// Locate the `parameters.m1cfg` that augments parameter symbols with concrete
/// value types and units. Explicit `--config` (or `$M1_CONFIG`) wins; otherwise
/// auto-discover the first `*.m1cfg` by walking UP from the project file's parent
/// directory through its ancestors (nearest wins), stopping at the filesystem
/// root — mirroring `find_project`. Real projects keep `parameters.m1cfg` at the
/// repository root while `Project.m1prj` is nested several directories deeper, so
/// a sibling-only search would miss it.
fn find_config(args: &Args, project_path: Option<&PathBuf>) -> Option<PathBuf> {
    if let Some(p) = &args.config {
        return Some(p.clone());
    }
    if let Some(p) = std::env::var_os("M1_CONFIG") {
        return Some(PathBuf::from(p));
    }
    m1_workspace::find_config_file(project_path?.parent()?)
}

fn main() {
    let mut args = Args::parse();

    // `--rules` prints the catalogue and exits, before any project work.
    if args.rules {
        print_rules(args.format);
        return;
    }

    // Drop empty/whitespace-only filter tokens (empty value, trailing comma)
    // before validation so they are no-ops rather than "unknown empty code".
    normalize_codes(&mut args.select);
    normalize_codes(&mut args.ignore);

    // Reject unknown T-codes in --select/--ignore up front (#111).
    validate_codes(&args);

    // Reject an unknown `--firmware` target up front: an unrecognised target
    // must fail loud (listing the known ones), never silently fall back to the
    // default and check against the wrong firmware (#260).
    if let Some(target) = &args.firmware
        && let Err(msg) = m1_typecheck::intrinsics::resolve_target(target)
    {
        eprintln!("m1-typecheck: {msg}");
        process::exit(2);
    }

    // Locate the project and its augmenting config.
    let project_path = find_project(&args);
    let config_path = find_config(&args, project_path.as_ref());

    // Cross-source T-code filter: nearest `m1-tools.toml` `[diagnostics]` overlaid
    // by explicit --select/--ignore (the LSP applies the same `[diagnostics]`
    // semantics). Discovered from the project's dir, else the first file's dir.
    let filter_root = project_path
        .as_ref()
        .and_then(|p| p.parent())
        .or_else(|| args.files.first().and_then(|f| f.parent()))
        .map(Path::to_path_buf);
    let filter = DiagFilter::resolve_with_symbols(
        filter_root.as_deref(),
        args.select.clone(),
        args.ignore.clone(),
        args.ignore_symbol.clone(),
    );

    // In JSON/SARIF mode diagnostics are buffered and rendered as one document
    // at the end; human mode prints as it goes.
    let json = args.format != Format::Human;
    let mut json_buf = JsonBuf::default();

    // Load the project model (config + DBCs) and emit degraded-run notes.
    let (mut project, dbc_loaded) = load_project(project_path.as_ref(), config_path.as_ref());
    let cfg_loaded = config_path.is_some();

    // Read requested sources tolerantly once. MoTeC `.m1scr` files may carry
    // Windows-1252 bytes, which the shared decoder handles. Unreadable requested
    // files fail the run; unreadable background project files remain skipped by
    // `gather_project_scripts`, matching the previous CLI behavior.
    let mut had_unreadable = false;
    let source_files: Vec<(PathBuf, String)> = args
        .files
        .iter()
        .filter_map(|path| match m1_workspace::read_text(path) {
            Ok(source) => Some((path.clone(), source)),
            Err(error) => {
                eprintln!("m1-typecheck: {}: {error}", path.display());
                had_unreadable = true;
                None
            }
        })
        .collect();
    let scripts: Vec<(String, String)> = source_files
        .iter()
        .filter_map(|(path, source)| {
            Some((path.file_name()?.to_str()?.to_string(), source.clone()))
        })
        .collect();

    // The project-wide cross-script passes (return-type inference, invalid-value
    // solve, scheduling T088/T089, usage T093/T094) are only sound over the WHOLE
    // project's scripts: a channel written by a script the user didn't pass on the
    // command line would otherwise look never-assigned (a false positive). So for
    // those passes use every `.m1scr` under the project's revision directory, not
    // just the files in `args.files`. Per-file diagnostics still use `args.files`.
    // Falls back to the command-line files when there is no project (or none found).
    let all_scripts: Vec<(String, String)> = match project_path.as_ref() {
        Some(p) => {
            let walked = gather_project_scripts(p);
            if walked.is_empty() {
                scripts.clone()
            } else {
                walked
            }
        }
        None => scripts.clone(),
    };

    // Parse every project script exactly once and share the CSTs across all the
    // project-wide passes below (return-type inference, the cross-script solve,
    // and the scheduling/usage checks). Previously each pass reparsed every
    // source independently — 5+ parses per script per run (#192).
    let parsed_scripts = m1_typecheck::parsed::parse_all(&all_scripts);

    // Query modes need the inferred model before they return. Normal checking
    // performs this preparation inside the shared project pipeline below.
    if (args.completeness || args.explain.is_some() || args.explain_units.is_some())
        && let Some(project) = project.as_mut()
    {
        project.infer_return_types(&parsed_scripts);
    }

    // `--completeness`: report how much of the project the analysis could pin
    // down (not findings), then exit 0. A query mode like `--rules`/`--explain`:
    // it runs after the model is loaded and return types inferred, so the
    // coverage reflects the full analysis the normal run would perform.
    if args.completeness {
        let report = m1_typecheck::completeness::analyze(
            project.as_ref(),
            &parsed_scripts,
            cfg_loaded,
            dbc_loaded,
        );
        print_completeness(&report, args.format);
        return;
    }

    // `--explain <CHANNEL>`: report that channel's solved status and exit.
    if let Some(channel) = &args.explain {
        let channel_taints = project
            .as_ref()
            .map(|project| cross_script::solve(project, &parsed_scripts))
            .unwrap_or_default();
        explain_channel(
            project.as_ref(),
            &channel_taints,
            channel,
            args.format == Format::Json,
        );
        return;
    }

    // `--explain-units <CHANNEL>`: the channel's quantity and every direct
    // contributor's unit (#144).
    if let Some(channel) = &args.explain_units {
        explain_units(project.as_ref(), &parsed_scripts, channel);
        return;
    }

    if args.audit_names && project.is_none() {
        eprintln!("m1-typecheck: --audit-names needs a project; skipping");
    }

    // One reusable entry point owns the complete pass list and applies the same
    // resolved diagnostics policy to source and project findings.
    let source_inputs: Vec<SourceInput<'_>> = source_files
        .iter()
        .map(|(path, source)| SourceInput::at_path(path, source))
        .collect();
    let result = m1_typecheck::project_check::check(
        project.as_mut(),
        &parsed_scripts,
        &source_inputs,
        &ProjectCheckOptions {
            filter: filter.clone(),
            audit_names: args.audit_names,
        },
    );
    let project_had_error = emit_project_diags(
        &args,
        result.project_diagnostics,
        &filter,
        project_path.as_deref(),
        json,
        &mut json_buf,
    );
    let had_error = had_unreadable
        || emit_source_checks(
            &args,
            project_path.as_deref(),
            result.sources,
            json.then_some(&mut json_buf),
        );

    // Emit the buffered JSON document (no-op in human mode).
    emit_output(project_path.as_ref(), args.format, &json_buf);

    if had_error || project_had_error {
        process::exit(1);
    }
}
