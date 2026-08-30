//! Reusable orchestration for a complete type-check run.
//!
//! The CLI, MCP server, and future library consumers must use this module
//! instead of maintaining their own lists of project-wide passes. It owns the
//! order and default/opt-in policy for return-type inference, cross-script
//! taint solving, source rules, scheduling, usage, DBC, calibration, project
//! metadata, and flash-persistence diagnostics.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use m1_core::Diagnostic;

use crate::cross_script;
use crate::diagnostics::TypeDiagnostic;
use crate::filter::DiagFilter;
use crate::parsed::ParsedScript;
use crate::project::Project;
use crate::rules::{Registry, check_script_with_channels};

/// One source buffer to check after the project-wide model has been prepared.
///
/// `path` is optional for a genuinely inline buffer. When present, only its file
/// name is used for project group/backing-function resolution; this function
/// never reads from or writes to the path.
#[derive(Debug, Clone, Copy)]
pub struct SourceInput<'a> {
    pub path: Option<&'a Path>,
    pub source: &'a str,
}

impl<'a> SourceInput<'a> {
    pub fn inline(source: &'a str) -> Self {
        Self { path: None, source }
    }

    pub fn at_path(path: &'a Path, source: &'a str) -> Self {
        Self {
            path: Some(path),
            source,
        }
    }
}

/// Diagnostics for one requested source buffer.
#[derive(Debug, Clone)]
pub struct SourceCheck {
    pub path: Option<PathBuf>,
    /// Parser diagnostics are not T-coded and are never filtered.
    pub syntax_errors: Vec<Diagnostic>,
    /// T-coded findings after the resolved diagnostics filter is applied.
    pub diagnostics: Vec<TypeDiagnostic>,
}

/// Policy for one complete project check.
#[derive(Debug, Clone, Default)]
pub struct ProjectCheckOptions {
    /// Resolved `[diagnostics]` policy, optionally overlaid by caller flags.
    pub filter: DiagFilter,
    /// Run the opt-in T050 project naming audit (the CLI's `--audit-names`).
    pub audit_names: bool,
}

impl ProjectCheckOptions {
    /// Resolve the nearest `m1-tools.toml` `[diagnostics]` section by walking up
    /// from `start`. Callers with explicit flags can construct an options value
    /// from [`DiagFilter::resolve_with_symbols`] instead.
    pub fn discover(start: Option<&Path>) -> Self {
        Self {
            filter: DiagFilter::resolve_with_symbols(start, None, None, None),
            audit_names: false,
        }
    }
}

/// Complete, filtered diagnostics from one orchestration run.
#[derive(Debug, Clone, Default)]
pub struct ProjectCheckResult {
    pub sources: Vec<SourceCheck>,
    pub project_diagnostics: Vec<TypeDiagnostic>,
}

/// Run the same complete source and project pipeline used by the CLI.
///
/// `all_scripts` must contain the complete project script set. Those parsed
/// trees back return-type inference and every cross-script pass. `sources` is
/// the (possibly smaller) set of buffers for which source-anchored findings are
/// requested. A caller checking an unsaved buffer should place that source in
/// both collections so project-wide analyses do not consume stale disk text.
pub fn check(
    mut project: Option<&mut Project>,
    all_scripts: &[ParsedScript],
    sources: &[SourceInput<'_>],
    options: &ProjectCheckOptions,
) -> ProjectCheckResult {
    if let Some(project) = project.as_deref_mut() {
        project.infer_return_types(all_scripts);
    }

    let channel_taints = project
        .as_deref()
        .map(|project| cross_script::solve(project, all_scripts))
        .unwrap_or_default();

    let mut project_diagnostics = Vec::new();
    if let Some(project) = project.as_deref() {
        // Scheduling: T088 and T097 are default-on; T089 is opt-in.
        project_diagnostics.extend(crate::schedule::check(
            project,
            all_scripts,
            true,
            options.filter.select.contains("T089"),
            true,
        ));

        // Flash-backed channels require reachable System.Preserve() code.
        project_diagnostics.extend(crate::schedule::check_flash_preserve(project, all_scripts));

        // Complete default-on usage and assignment audits.
        project_diagnostics.extend(crate::schedule::check_usage(
            project,
            all_scripts,
            true,
            true,
        ));
        project_diagnostics.extend(crate::schedule::check_multi_writers(project, all_scripts));
        project_diagnostics.extend(crate::schedule::check_cross_fn_assignment(
            project,
            all_scripts,
        ));
        project_diagnostics.extend(crate::schedule::check_reachability(project, all_scripts));
        project_diagnostics.extend(crate::dbc_init::check(project, all_scripts));

        // Model audits that do not need script bodies.
        project_diagnostics.extend(project.missing_cfg_parameters());
        project_diagnostics.extend(project.audit_tags());
        project_diagnostics.extend(project.audit_display_units());
        if options.audit_names {
            project_diagnostics.extend(project.audit());
        }
    }
    project_diagnostics.retain(|diagnostic| {
        options
            .filter
            .allows_subject(diagnostic.code.as_str(), diagnostic.subject.as_deref())
    });

    let enabled_opt_in: HashSet<String> = Registry::opt_in_codes()
        .iter()
        .map(|code| code.as_str().to_string())
        .filter(|code| options.filter.select.contains(code))
        .collect();
    let sources = sources
        .iter()
        .map(|input| {
            let mut result = check_script_with_channels(
                &enabled_opt_in,
                project.as_deref(),
                input.path,
                input.source,
                &channel_taints,
            );
            result
                .diagnostics
                .retain(|diagnostic| options.filter.allows(diagnostic.code.as_str()));
            SourceCheck {
                path: input.path.map(Path::to_path_buf),
                syntax_errors: result.syntax_errors,
                diagnostics: result.diagnostics,
            }
        })
        .collect();

    ProjectCheckResult {
        sources,
        project_diagnostics,
    }
}
