//! Cross-script invalid-value propagation (#78 P3) — the project-wide half of
//! the T080/T081 analysis in [`crate::invalid_value`].
//!
//! Per script, the intra-script walker also returns a *write summary*: for
//! each assignment targeting a project symbol (a channel, or `Out` → the
//! script's backing function symbol), the local taint of the written value
//! plus the canonical paths of the project symbols it reads — the cross-script
//! dependency edges, each carrying a "passes through a latching stateful
//! function" flag. [`solve`] iterates the writer→reader channel graph to a
//! fixpoint — the lattice (untainted < tainted < latched) is finite and the
//! join is monotone, so feedback loops between scripts terminate — and renders
//! a cycle-safe backward provenance chain per tainted channel. The per-file
//! reporting pass then seeds the solved [`ChannelTaints`] into the expression
//! walker, so T080/T081 fire at the sink with the full cross-script story.

use crate::invalid_value::Taint;
use crate::project::Project;
use crate::resolve::Scope;
use std::collections::{BTreeMap, HashSet};

/// Solved per-channel invalid-value state, keyed by canonical symbol path
/// (`Root.Sensors.Yaw`). Presence in the map means "can be NaN/Inf".
#[derive(Debug, Clone, Default)]
pub struct ChannelTaints {
    map: std::collections::BTreeMap<String, ChannelTaint>,
}

/// One solved channel's state: how the invalid value gets there.
#[derive(Debug, Clone)]
pub struct ChannelTaint {
    /// The taint flowed through a stateful function (`Filter.*`/`Integral.*`/
    /// `Derivative.*`) somewhere along the chain and stays latched after the
    /// input recovers.
    pub latched: bool,
    /// Backward provenance, sink-most step first.
    pub chain: Vec<String>,
}

impl ChannelTaint {
    /// The chain as one human-readable sentence fragment.
    pub fn provenance(&self) -> String {
        self.chain.join("; ")
    }
}

impl ChannelTaints {
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn get(&self, canonical_path: &str) -> Option<&ChannelTaint> {
        self.map.get(canonical_path)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &ChannelTaint)> {
        self.map.iter()
    }

    /// The expression-walker seed for reading `canonical_path`.
    pub(crate) fn read_taint(&self, canonical_path: &str) -> Taint {
        match self.map.get(canonical_path) {
            Some(t) => {
                let mut s = Taint::source(t.provenance());
                s.latched = t.latched;
                s
            }
            None => Taint::default(),
        }
    }
}

/// One channel's solved invalid-value status, as reported by [`explain`]
/// (the `--explain` CLI query). `taint: None` means no invalid-value path
/// reaches the channel as far as the cross-script analysis can see.
pub struct Explanation {
    /// The canonical (`Root.`-qualified) channel path.
    pub channel: String,
    pub taint: Option<ChannelTaint>,
}

/// Look up `channel` — canonical `Root.`-qualified or bare spelling — in the
/// project and report its solved status. `None` when no such symbol exists.
pub fn explain(project: &Project, taints: &ChannelTaints, channel: &str) -> Option<Explanation> {
    let canonical = if project.symbols().get(channel).is_some() {
        channel.to_string()
    } else {
        let q = m1_workspace::qualify_root(channel).into_owned();
        project.symbols().get(&q)?;
        q
    };
    let taint = taints.get(&canonical).cloned();
    Some(Explanation {
        channel: canonical,
        taint,
    })
}

/// One summarised write: which script performed it and the taint of the
/// written value (whose `deps` carry the cross-script read edges).
struct Writer {
    script: String,
    taint: Taint,
}

/// Solve the project-wide channel taint graph. `scripts` pairs each script's
/// file name with its source — same convention as
/// [`Project::infer_return_types`]. Scripts with syntax errors (or
/// pathologically deep CSTs, #94) are skipped; everything else contributes
/// its write summary, and the writer→reader graph is iterated to a fixpoint.
pub fn solve(project: &Project, scripts: &[(String, String)]) -> ChannelTaints {
    let mut writers: BTreeMap<String, Vec<Writer>> = BTreeMap::new();
    for (file_name, source) in scripts {
        let cst = m1_core::parse(source);
        if !cst.syntax_diagnostics().is_empty() {
            continue;
        }
        // #94 parity: the summary walker recurses over the CST, so probe the
        // (iteratively computed) depth and skip an over-deep file.
        if cst.root().max_depth() > m1_core::MAX_RECURSION_DEPTH {
            continue;
        }
        let group = project.group_for_script(file_name);
        let fn_symbol = project.function_symbol_for_script(file_name);
        let scope = Scope {
            locals: crate::rules::collect_locals(cst.root(), Some(project), group.as_deref()),
            group,
            project: Some(project),
            fn_symbol: fn_symbol.clone(),
        };
        let anns = m1_core::annotations(&cst, &m1_core::Registry::seed());
        // Per-file diagnostics are produced by the (later) reporting pass;
        // here only the write summary matters.
        let mut discard = Vec::new();
        let writes = crate::invalid_value::check_with_channels(
            cst.root(),
            &scope,
            &anns,
            &ChannelTaints::default(),
            fn_symbol.as_deref(),
            &mut discard,
        );
        for (channel, taint) in writes {
            writers.entry(channel).or_default().push(Writer {
                script: file_name.clone(),
                taint,
            });
        }
    }
    fixpoint(&writers)
}

/// How a channel became tainted — the provenance link the chain renderer walks.
enum Cause {
    /// The writer's own expression is an invalid-value source.
    Local { script: String, why: String },
    /// The taint arrived from an upstream channel the writing script reads.
    Upstream { channel: String, script: String },
}

struct State {
    latched: bool,
    cause: Cause,
}

/// Iterate the channel graph to a fixpoint. The per-channel lattice
/// (untainted < tainted < latched) is finite and [`upgrade`] is monotone, so
/// the sweep terminates even with feedback loops between scripts.
fn fixpoint(writers: &BTreeMap<String, Vec<Writer>>) -> ChannelTaints {
    let mut state: BTreeMap<String, State> = BTreeMap::new();
    loop {
        let mut changed = false;
        for (channel, ws) in writers {
            for w in ws {
                if w.taint.invalid && !w.taint.sanitised {
                    let why = w
                        .taint
                        .why
                        .clone()
                        .unwrap_or_else(|| "an invalid-value source".into());
                    upgrade(
                        &mut state,
                        channel,
                        w.taint.latched,
                        || Cause::Local {
                            script: w.script.clone(),
                            why,
                        },
                        &mut changed,
                    );
                }
                // A sanitised write contributes nothing (its deps were cleared
                // too): `@m1:sanitizes` is the cross-script taint barrier.
                for (dep, latch_en_route) in &w.taint.deps {
                    let Some(up_latched) = state.get(dep).map(|s| s.latched) else {
                        continue;
                    };
                    upgrade(
                        &mut state,
                        channel,
                        up_latched || *latch_en_route,
                        || Cause::Upstream {
                            channel: dep.clone(),
                            script: w.script.clone(),
                        },
                        &mut changed,
                    );
                }
            }
        }
        if !changed {
            break;
        }
    }
    render(&state)
}

/// Monotone join into `state[channel]`. The first cause to taint a channel is
/// kept (deterministic: the sweep order is the BTreeMap order); a later latch
/// escalation flips the bit without rewriting the provenance.
fn upgrade(
    state: &mut BTreeMap<String, State>,
    channel: &str,
    latched: bool,
    cause: impl FnOnce() -> Cause,
    changed: &mut bool,
) {
    match state.get_mut(channel) {
        None => {
            state.insert(
                channel.to_string(),
                State {
                    latched,
                    cause: cause(),
                },
            );
            *changed = true;
        }
        Some(s) if latched && !s.latched => {
            s.latched = true;
            *changed = true;
        }
        _ => {}
    }
}

/// Render each tainted channel's backward provenance chain. First-cause-wins
/// makes the cause graph a forest rooted at `Local` causes, so the walk is
/// acyclic; the visited guard is defensive belt-and-braces.
fn render(state: &BTreeMap<String, State>) -> ChannelTaints {
    let mut map = BTreeMap::new();
    for (channel, s) in state {
        let mut chain = Vec::new();
        let mut cur = channel.as_str();
        let mut visited: HashSet<&str> = HashSet::new();
        loop {
            if !visited.insert(cur) {
                chain.push(format!("`{cur}` closes a feedback loop"));
                break;
            }
            match state.get(cur).map(|s| &s.cause) {
                Some(Cause::Local { script, why }) => {
                    chain.push(format!("`{cur}` is written by `{script}`: {why}"));
                    break;
                }
                Some(Cause::Upstream {
                    channel: up,
                    script,
                }) => {
                    chain.push(format!("`{cur}` is written by `{script}` from `{up}`"));
                    cur = up;
                }
                None => break,
            }
        }
        map.insert(
            channel.clone(),
            ChannelTaint {
                latched: s.latched,
                chain,
            },
        );
    }
    ChannelTaints { map }
}
