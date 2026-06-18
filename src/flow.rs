//! Control-flow analysis: single-assignment-per-channel (T040), and the
//! per-function channel-assignment *facts* the cross-function check (T102,
//! `schedule::check_cross_fn_assignment`) composes across call edges.
//!
//! For each statement subtree we compute, per channel path, the maximum number
//! of assignments on any single execution path (`max_per_path`). Sequential
//! statements add; mutually-exclusive `if`/`else` arms take the per-key max. A
//! channel whose top-level `max_per_path` reaches 2 is assigned twice on one path.
//!
//! A user-function call contributes the callee's facts *in sequence* (a call
//! always runs when its statement runs). The intra-procedural T040 uses a no-op
//! expander (calls contribute nothing); the inter-procedural T102 supplies an
//! expander that returns each callee's (memoised, transitively-inlined) facts.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use crate::typer::{is_expr, path_text};
use m1_core::{Kind, Node};
use std::collections::HashMap;

/// Channel path -> max assignments on a single path through a subtree.
pub(crate) type ChannelCounts = HashMap<String, u32>;

#[derive(Default, Clone)]
struct Facts {
    max_per_path: ChannelCounts,
}

/// Returns the extra facts a user-function call contributes, keyed by the
/// resolved callee symbol path. The intra-procedural case passes a closure that
/// returns `None` for everything (calls add nothing).
pub(crate) trait CallExpander {
    fn facts_for(&self, callee_path: &str) -> Option<ChannelCounts>;
}

/// No call contributes anything — the intra-procedural T040 view.
struct NoCalls;
impl CallExpander for NoCalls {
    fn facts_for(&self, _: &str) -> Option<ChannelCounts> {
        None
    }
}

impl<F: Fn(&str) -> Option<ChannelCounts>> CallExpander for F {
    fn facts_for(&self, callee_path: &str) -> Option<ChannelCounts> {
        self(callee_path)
    }
}

/// Entry point: run over the whole script, append T040 diagnostics.
pub fn single_assignment(root: Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    let facts = seq_children(root, scope, &NoCalls);
    for (path, n) in &facts.max_per_path {
        if *n >= 2 {
            // Point at the FIRST conflicting write (the earlier assignment that
            // establishes the conflict) so the diagnostic / quick-fix lands at
            // the write the user typically removes or guards.
            if let Some(node) = nth_assignment(root, scope, path, 1) {
                out.push(make(
                    TypeCode::T040,
                    &node,
                    m1_core::Severity::Warning,
                    format!("channel `{path}` is assigned more than once on a single code path"),
                ));
            }
        }
    }
}

/// The per-channel `max_per_path` assignment counts for one function body, with
/// `expand` supplying each user-function call's contribution (so T102 can inline
/// callee facts). Reused by `schedule::check_cross_fn_assignment`.
pub(crate) fn channel_counts(
    root: Node,
    scope: &Scope,
    expand: &dyn CallExpander,
) -> ChannelCounts {
    seq_children(root, scope, expand).max_per_path
}

/// Compose the facts of a node's *statement* children in sequence.
fn seq_children(node: Node, scope: &Scope, expand: &dyn CallExpander) -> Facts {
    let mut acc = Facts::default();
    for child in node.child_nodes() {
        let f = facts_of(child, scope, expand);
        acc = merge_seq(acc, f);
    }
    acc
}

/// Facts for a single statement node.
fn facts_of(node: Node, scope: &Scope, expand: &dyn CallExpander) -> Facts {
    match node.kind() {
        Kind::AssignmentStatement => {
            let mut f = Facts::default();
            if let Some(path) = assigned_channel(node, scope) {
                f.max_per_path.insert(path, 1);
            }
            // A user-function call anywhere in the statement (RHS, target index,
            // …) runs when the statement runs — sequence in its inlined facts.
            merge_seq(f, call_facts(node, scope, expand))
        }
        Kind::IfStatement => {
            // then-block facts vs else-block facts -> per-key max (mutual exclusion).
            let blocks: Vec<_> = node
                .child_nodes()
                .filter(|c| c.kind() == Kind::Block || c.kind() == Kind::ElseClause)
                .collect();
            let then_f = blocks
                .first()
                .map(|b| seq_children(*b, scope, expand))
                .unwrap_or_default();
            let else_f = blocks
                .iter()
                .find(|b| b.kind() == Kind::ElseClause)
                .map(|b| seq_children(*b, scope, expand))
                .unwrap_or_default();
            merge_alt(then_f, else_f)
        }
        Kind::WhenStatement => {
            // A `when` body is a set of `is` clauses (the CST is
            // WhenStatement -> IsClause -> Block). The clauses are mutually
            // exclusive — only the matching state runs per tick — so take the
            // per-channel MAX across them (merge_alt), not the sequential sum
            // (#20). Two writes in different arms are fine; two in one arm still
            // flag T040 (seq_children sums within a single Block).
            node.child_nodes()
                .filter(|c| c.kind() == Kind::IsClause)
                .map(|clause| {
                    clause
                        .child_nodes()
                        .find(|c| c.kind() == Kind::Block)
                        .map(|b| seq_children(b, scope, expand))
                        .unwrap_or_default()
                })
                .fold(Facts::default(), merge_alt)
        }
        Kind::ExpandStatement => {
            // Treated as a no-else conditional body (see spec §5.2).
            let body = node.child_nodes().find(|c| c.kind() == Kind::Block);
            let body_f = body
                .map(|b| seq_children(b, scope, expand))
                .unwrap_or_default();
            merge_alt(body_f, Facts::default())
        }
        Kind::Block | Kind::ElseClause => seq_children(node, scope, expand),
        // Any other statement (a bare call `Helper();`, a `local x = f();`, …)
        // contributes only its user-function calls' inlined facts. These nodes
        // hold no nested *statements* (M1 expressions can't contain statements),
        // so collecting calls over the whole subtree never double-counts a
        // nested control-flow body.
        _ => call_facts(node, scope, expand),
    }
}

/// The sequential sum of every user-function call's contributed facts within
/// `node`'s subtree (expression position only — see the `facts_of` default arm).
fn call_facts(node: Node, scope: &Scope, expand: &dyn CallExpander) -> Facts {
    let mut acc = Facts::default();
    for callee in user_callees(node, scope) {
        if let Some(counts) = expand.facts_for(&callee) {
            acc = merge_seq(
                acc,
                Facts {
                    max_per_path: counts,
                },
            );
        }
    }
    acc
}

/// Every resolved user-function/method callee path called within `node`'s
/// subtree, in document order (duplicates kept — two calls run twice).
pub(crate) fn user_callees(node: Node, scope: &Scope) -> Vec<String> {
    let mut out = Vec::new();
    for n in node.descendants() {
        if n.kind() != Kind::CallExpression {
            continue;
        }
        let Some(callee) = n
            .named_children()
            .into_iter()
            .find(|c| matches!(c.kind(), Kind::Identifier | Kind::MemberExpression))
        else {
            continue;
        };
        if let Resolution::Symbol(s) = resolve(&path_text(callee), scope)
            && matches!(s.kind, SymbolKind::Function | SymbolKind::Method)
        {
            out.push(s.path.clone());
        }
    }
    out
}

fn merge_seq(mut a: Facts, b: Facts) -> Facts {
    for (k, v) in b.max_per_path {
        let entry = a.max_per_path.entry(k).or_insert(0);
        *entry += v;
    }
    a
}

fn merge_alt(a: Facts, b: Facts) -> Facts {
    let mut out = Facts::default();
    for (k, v) in a.max_per_path.iter().chain(b.max_per_path.iter()) {
        let e = out.max_per_path.entry(k.clone()).or_insert(0);
        *e = (*e).max(*v);
    }
    out
}

/// If `node` is `target = …` to a Channel symbol, return its canonical path.
fn assigned_channel(node: Node, scope: &Scope) -> Option<String> {
    if !node.child_nodes().any(|c| c.kind() == Kind::Assign) {
        return None; // skip += etc. (the "set once" rule targets plain assignment)
    }
    let target = node
        .named_children()
        .into_iter()
        .find(|c| is_expr(c.kind()))?;
    match resolve(&path_text(target), scope) {
        Resolution::Symbol(s) if s.kind == SymbolKind::Channel => Some(s.path.clone()),
        _ => None,
    }
}

/// Find the n-th (1-based) assignment to `path` in document order, for the span.
fn nth_assignment<'a>(root: Node<'a>, scope: &Scope, path: &str, n: u32) -> Option<Node<'a>> {
    fn walk<'a>(
        node: Node<'a>,
        scope: &Scope,
        path: &str,
        n: u32,
        count: &mut u32,
    ) -> Option<Node<'a>> {
        if node.kind() == Kind::AssignmentStatement
            && assigned_channel(node, scope).as_deref() == Some(path)
        {
            *count += 1;
            if *count == n {
                return Some(node);
            }
        }
        for c in node.child_nodes() {
            if let Some(found) = walk(c, scope, path, n, count) {
                return Some(found);
            }
        }
        None
    }
    let mut count = 0;
    walk(root, scope, path, n, &mut count)
}
