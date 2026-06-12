//! Parse-once sharing of script CSTs across the project-wide passes (#192).
//!
//! A full run feeds the same set of script sources through several independent
//! project-wide analyses — return-type inference ([`crate::project::Project::infer_return_types`]),
//! the cross-script taint solve ([`crate::cross_script::solve`]), and the
//! scheduling/usage checks ([`crate::schedule`]). Each used to call
//! `m1_core::parse` on every source itself, so a 50-script project parsed each
//! file five-plus times. Parsing is the pipeline's most expensive step, and the
//! LSP runs several of these passes per diagnostics poll.
//!
//! [`parse_all`] parses each source exactly once into a [`ParsedScript`]; the
//! passes take a `&[ParsedScript]` slice and read the shared [`Cst`] instead of
//! reparsing. Order is preserved (the scheduling graph indexes into the slice),
//! and each pass keeps its own syntax-error / depth guard, reading them off the
//! shared CST.

use m1_core::Cst;

/// One script's file name (the bare `Filename`, e.g. `"Engine.Update.m1scr"`)
/// paired with its already-parsed CST. Built once per run by [`parse_all`] and
/// shared by reference across the project-wide passes.
pub struct ParsedScript {
    /// The script's file name as the project records it.
    pub name: String,
    /// The parsed concrete syntax tree (owns its source text).
    pub cst: Cst,
}

/// Parse every `(file_name, source)` pair once, preserving input order. The
/// result is shared by reference with every project-wide pass so each source is
/// parsed a single time per run.
pub fn parse_all(scripts: &[(String, String)]) -> Vec<ParsedScript> {
    scripts
        .iter()
        .map(|(name, source)| ParsedScript {
            name: name.clone(),
            cst: m1_core::parse(source),
        })
        .collect()
}
