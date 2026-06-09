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
