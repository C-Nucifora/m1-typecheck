//! Cross-source T-code diagnostic filter, mirroring the LSP's `[diagnostics]`
//! handling (see `m1-lsp/src/config.rs::DiagFilter`).
//!
//! The standalone CLI honours the same `m1-tools.toml` `[diagnostics]` section the
//! editors do — `ignore` drops the listed T-codes, a non-empty `select` keeps only
//! the listed ones — so `m1-typecheck` filters identically whether invoked from CI
//! or the editor. Explicit `--select`/`--ignore` flags overlay (replace) whatever
//! the file specifies, matching m1-lint's flag-over-config precedence.
use std::collections::HashSet;
use std::path::Path;

/// Cross-source diagnostic filter. Codes are matched against each diagnostic's
/// T-code string (`T001`, `T041`, …). Semantics identical to the LSP's filter so
/// the CLI and editors agree.
#[derive(Debug, Clone, Default)]
pub struct DiagFilter {
    pub ignore: HashSet<String>,
    pub select: HashSet<String>,
    /// Symbol-scoped suppressions (`[diagnostics] ignore_symbols` /
    /// `--ignore-symbol`): `(code, symbol_path)` pairs that drop a
    /// project-level diagnostic for exactly that symbol (#151) — the
    /// suppression channel for the zero-range audit family that
    /// `@m1:allow` can't reach.
    pub ignore_symbols: HashSet<(String, String)>,
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

    /// [`Self::allows`] plus the symbol-scoped check: a `(code, subject)` pair
    /// listed in `ignore_symbols` is dropped. Diagnostics without a subject are
    /// unaffected by `ignore_symbols`.
    pub fn allows_subject(&self, code: &str, subject: Option<&str>) -> bool {
        if !self.allows(code) {
            return false;
        }
        match subject {
            Some(s) => !self
                .ignore_symbols
                .contains(&(code.to_string(), s.to_string())),
            None => true,
        }
    }

    /// True when no filtering is configured (the common case — skip the walk).
    pub fn is_empty(&self) -> bool {
        self.ignore.is_empty() && self.select.is_empty() && self.ignore_symbols.is_empty()
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
        Self::resolve_with_symbols(start, select, ignore, None)
    }

    /// [`Self::resolve`] plus the `--ignore-symbol` overlay (each flag entry is
    /// `CODE:Symbol.Path`; when supplied, the flag list replaces the file's
    /// `ignore_symbols`, matching the other flag-over-config fields).
    pub fn resolve_with_symbols(
        start: Option<&Path>,
        select: Option<Vec<String>>,
        ignore: Option<Vec<String>>,
        ignore_symbols: Option<Vec<String>>,
    ) -> DiagFilter {
        let parse_pairs = |entries: Vec<String>| -> HashSet<(String, String)> {
            entries
                .iter()
                .filter_map(|e| m1_workspace::parse_ignore_symbol(e))
                .map(|(c, p)| (c.to_string(), p.to_string()))
                .collect()
        };
        let mut f = DiagFilter::default();
        if let Some(s) = start
            && let Some(tc) = m1_workspace::config::M1ToolsConfig::discover(s)
        {
            if let Some(ig) = tc.diagnostics.ignore {
                f.ignore = ig.into_iter().collect();
            }
            if let Some(se) = tc.diagnostics.select {
                f.select = se.into_iter().collect();
            }
            if let Some(is) = tc.diagnostics.ignore_symbols {
                f.ignore_symbols = parse_pairs(is);
            }
        }
        if let Some(se) = select {
            f.select = se.into_iter().collect();
        }
        if let Some(ig) = ignore {
            f.ignore = ig.into_iter().collect();
        }
        if let Some(is) = ignore_symbols {
            f.ignore_symbols = parse_pairs(is);
        }
        f
    }
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
    fn ignore_symbols_suppresses_one_subject_only() {
        let dir = std::env::temp_dir().join(format!("m1tc_igs_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("m1-tools.toml"),
            "[diagnostics]\nignore_symbols = [\"T050:Root.Engine.Speed\"]\n",
        )
        .unwrap();

        let f = DiagFilter::resolve_with_symbols(Some(&dir), None, None, None);
        assert!(!f.is_empty());
        // The listed (code, symbol) pair is dropped…
        assert!(!f.allows_subject("T050", Some("Root.Engine.Speed")));
        // …but the same code on another symbol, another code on the same
        // symbol, and subject-less diagnostics all pass.
        assert!(f.allows_subject("T050", Some("Root.Engine.Other")));
        assert!(f.allows_subject("T071", Some("Root.Engine.Speed")));
        assert!(f.allows_subject("T050", None));

        // The --ignore-symbol flag replaces the file's list.
        let g = DiagFilter::resolve_with_symbols(
            Some(&dir),
            None,
            None,
            Some(vec!["T041:Root.X".into()]),
        );
        assert!(g.allows_subject("T050", Some("Root.Engine.Speed")));
        assert!(!g.allows_subject("T041", Some("Root.X")));

        let _ = std::fs::remove_dir_all(&dir);
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
