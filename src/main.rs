mod output;

use clap::{Parser, ValueEnum};
use m1_core::Severity;
use m1_typecheck::diagnostics::{TypeCode, TypeDiagnostic};
use m1_typecheck::filter::DiagFilter;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script_with;
use output::{JsonFile, json_str, render_json, severity_str};
use std::path::{Path, PathBuf};
use std::process;

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
    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Human)]
    format: Format,
    /// Print the T-code catalogue (honours --format json) and exit.
    #[arg(long)]
    rules: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Format {
    Human,
    Json,
}

/// Print the T-code catalogue (the single source of truth tools enumerate),
/// mirroring `m1-lint --rules`. JSON: `{"version":1,"rules":[{code,name},…]}`.
fn print_rules(format: Format) {
    match format {
        Format::Json => {
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
    let args = Args::parse();

    // `--rules` prints the catalogue and exits, before any project work.
    if args.rules {
        print_rules(args.format);
        return;
    }

    let mut had_error = false;

    // Reject unknown T-codes in --select/--ignore up front (a silent typo used to
    // disable nothing) (#111).
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
    let filter = DiagFilter::resolve(
        filter_root.as_deref(),
        args.select.clone(),
        args.ignore.clone(),
    );

    // Opt-in rules (e.g. T064) run only when explicitly selected. A code is
    // "enabled" if it is opt-in AND named in the resolved `select` set.
    let enabled_opt_in: std::collections::HashSet<String> =
        m1_typecheck::rules::Registry::opt_in_codes()
            .iter()
            .map(|c| c.as_str().to_string())
            .filter(|c| filter.select.contains(c))
            .collect();

    // In JSON mode diagnostics are buffered and rendered as one document at the
    // end; human mode prints as it goes.
    let json = args.format == Format::Json;
    let mut json_files: Vec<JsonFile> = Vec::new();
    let mut json_project: Vec<TypeDiagnostic> = Vec::new();
    // Track whether any `.m1dbc` was discovered so a project that loads but finds
    // none can announce that T042 is skipped (mirroring the cfg/project notes).
    let mut dbc_found = false;
    let mut project = project_path.clone().map(|path| match Project::load(&path) {
        Ok(mut p) => {
            if let Some(cfg) = &config_path {
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

    // Infer user-function/method return types from the script bodies on the
    // command line, so call sites in other scripts type-check (#110). Scripts that
    // back no function symbol are simply ignored by the pass.
    if let Some(p) = project.as_mut() {
        let scripts: Vec<(String, String)> = args
            .files
            .iter()
            .filter_map(|f| {
                let name = f.file_name()?.to_str()?.to_string();
                let src = m1_workspace::read_text(f).ok()?;
                Some((name, src))
            })
            .collect();
        p.infer_return_types(&scripts);
    }

    for file in &args.files {
        // Read tolerantly: MoTeC `.m1scr` sources may carry Windows-1252 bytes
        // (e.g. `°` in a unit comment), which strict UTF-8 would reject (#86).
        let src = match m1_workspace::read_text(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("m1-typecheck: {}: {e}", file.display());
                had_error = true;
                continue;
            }
        };
        let result = check_script_with(
            &enabled_opt_in,
            project.as_ref(),
            Some(file.as_path()),
            &src,
        );
        // Syntax errors are not T-coded and always fail the run; the filter only
        // governs the T-code diagnostics.
        for d in &result.syntax_errors {
            had_error = true;
            if !json {
                println!(
                    "{}:{}:{}: error[syntax]: {}",
                    file.display(),
                    d.range.start.line + 1,
                    d.range.start.column + 1,
                    d.message
                );
            }
        }
        let kept: Vec<&TypeDiagnostic> = result
            .diagnostics
            .iter()
            .filter(|d| filter.allows(d.code.as_str()))
            .collect();
        for d in &kept {
            if d.inner.severity == Severity::Error {
                had_error = true;
            }
            if !json {
                println!(
                    "{}:{}:{}: {}[{}]: {}",
                    file.display(),
                    d.inner.range.start.line + 1,
                    d.inner.range.start.column + 1,
                    severity_str(d.inner.severity),
                    d.code.as_str(),
                    d.inner.message
                );
            }
        }
        if json {
            json_files.push(JsonFile {
                path: file.display().to_string(),
                syntax_errors: result.syntax_errors,
                diagnostics: kept.into_iter().cloned().collect(),
            });
        }
    }

    // Project-level audit: parameters declared in the .m1prj but missing from the
    // loaded .m1cfg (T041). Emitted on every run that has a project + cfg so CI
    // validates calibration coverage. Warning severity — annotates without failing
    // the build unless the caller opts into fail-on-warning.
    if let Some(p) = &project {
        let path_label = project_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<project>".into());
        for d in p.missing_cfg_parameters() {
            if !filter.allows(d.code.as_str()) {
                continue;
            }
            if json {
                json_project.push(d);
            } else {
                println!(
                    "{}: warning[{}]: {}",
                    path_label,
                    d.code.as_str(),
                    d.inner.message
                );
            }
        }
    }

    if args.audit_names {
        match &project {
            Some(p) => {
                let path_label = project_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<project>".into());
                for d in p.audit() {
                    if !filter.allows(d.code.as_str()) {
                        continue;
                    }
                    if json {
                        json_project.push(d);
                    } else {
                        println!(
                            "{}: {}[{}]: {}",
                            path_label,
                            severity_str(d.inner.severity),
                            d.code.as_str(),
                            d.inner.message
                        );
                    }
                }
            }
            None => eprintln!("m1-typecheck: --audit-names needs a project; skipping"),
        }
    }

    if json {
        let label = project_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<project>".into());
        println!("{}", render_json(&json_files, &json_project, &label));
    }

    if had_error {
        process::exit(1);
    }
}
