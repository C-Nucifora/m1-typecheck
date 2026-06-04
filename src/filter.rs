//! Cross-source T-code diagnostic filter, mirroring the LSP's `[diagnostics]`
//! handling (see `m1-lsp/src/config.rs::DiagFilter`).
//!
//! The standalone CLI honours the same `m1-tools.toml` `[diagnostics]` section the
//! editors do — `ignore` drops the listed T-codes, a non-empty `select` keeps only
//! the listed ones — so `m1-typecheck` filters identically whether invoked from CI
//! or the editor. Explicit `--select`/`--ignore` flags overlay (replace) whatever
//! the file specifies, matching m1-lint's flag-over-config precedence.
use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

/// Cross-source diagnostic filter. Codes are matched against each diagnostic's
/// T-code string (`T001`, `T041`, …). Semantics identical to the LSP's filter so
/// the CLI and editors agree.
#[derive(Debug, Clone, Default)]
pub struct DiagFilter {
    pub ignore: HashSet<String>,
    pub select: HashSet<String>,
}

impl DiagFilter {
    /// Whether a diagnostic with this code should be reported. A non-empty
    /// `select` keeps only the listed codes; `ignore` drops codes from any source.
    pub fn allows(&self, code: &str) -> bool {
        if !self.select.is_empty() && !self.select.contains(code) {
            return false;
        }
        !self.ignore.contains(code)
    }

    /// True when no filtering is configured (the common case — skip the walk).
    pub fn is_empty(&self) -> bool {
        self.ignore.is_empty() && self.select.is_empty()
    }

    /// Resolve the effective filter for a run rooted near `start`: read the
    /// `[diagnostics]` section of the nearest `m1-tools.toml` (walking up from
    /// `start`), then overlay explicit `--select`/`--ignore` flags (each replaces
    /// the corresponding file field when supplied), matching m1-lint's UX.
    pub fn resolve(
        start: Option<&Path>,
        select: Option<Vec<String>>,
        ignore: Option<Vec<String>>,
    ) -> DiagFilter {
        let mut f = DiagFilter::default();
        if let Some(s) = start
            && let Some(text) = discover(s, "m1-tools.toml")
            && let Ok(raw) = toml::from_str::<Raw>(&text)
        {
            if let Some(ig) = raw.diagnostics.ignore {
                f.ignore = ig.into_iter().collect();
            }
            if let Some(se) = raw.diagnostics.select {
                f.select = se.into_iter().collect();
            }
        }
        if let Some(se) = select {
            f.select = se.into_iter().collect();
        }
        if let Some(ig) = ignore {
            f.ignore = ig.into_iter().collect();
        }
        f
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Raw {
    diagnostics: RawDiag,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawDiag {
    ignore: Option<Vec<String>>,
    select: Option<Vec<String>>,
}

/// Walk up from `start` looking for a file named `name`; return its contents.
fn discover(start: &Path, name: &str) -> Option<String> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let candidate = d.join(name);
        if candidate.is_file() {
            return std::fs::read_to_string(&candidate).ok();
        }
        dir = d.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_honours_ignore_and_select() {
        let mut f = DiagFilter::default();
        assert!(f.is_empty());
        assert!(f.allows("T001"));
        f.ignore.insert("T041".into());
        assert!(!f.allows("T041"));
        assert!(f.allows("T001"));

        let mut g = DiagFilter::default();
        g.select.insert("T002".into());
        assert!(g.allows("T002"));
        assert!(!g.allows("T001"));
    }

    #[test]
    fn flags_override_toml_fields() {
        let dir = std::env::temp_dir().join(format!("m1tc_filter_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("m1-tools.toml"),
            "[diagnostics]\nignore = [\"T041\"]\n",
        )
        .unwrap();

        // File alone: T041 ignored.
        let f = DiagFilter::resolve(Some(&dir), None, None);
        assert!(!f.allows("T041"));

        // --ignore replaces the file's ignore list.
        let g = DiagFilter::resolve(Some(&dir), None, Some(vec!["T002".into()]));
        assert!(g.allows("T041"));
        assert!(!g.allows("T002"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
