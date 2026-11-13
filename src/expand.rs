//! Compile-time `expand (VAR = lo to hi) { … }` loop expansion (manual p.33).
//!
//! M1 Build text-substitutes the body for each value of `VAR`, replacing every
//! `$(VAR)` interpolation, *before* it resolves the resulting object names. Two
//! analyses need the same expansion to match that behaviour: the schedule I/O
//! audit (#170 — `$(VAR)`-templated channel access must count toward every
//! substituted channel) and the unresolved-reference rule (#246 — a
//! `$(VAR)`-templated assignment/reference target must be checked against the
//! concrete object names, not the literal `$(VAR)` text).
use m1_core::{Field, Kind, Node};

/// Active `expand` variable bindings, innermost last: `(name, start, end)`.
pub(crate) type ExpandBindings = Vec<(String, i64, i64)>;

/// The `(variable, start, end)` binding of an `expand` statement whose bounds
/// are integer literals. Constant-named bounds (allowed by the manual) are not
/// evaluated — their bodies keep the pre-#170 behaviour (templated paths stay
/// unresolved).
pub(crate) fn expand_binding(n: &Node) -> Option<(String, i64, i64)> {
    let var = n.child_by_field(Field::Variable)?.text().trim().to_string();
    let lo = literal_bound(n.child_by_field(Field::Start)?)?;
    let hi = literal_bound(n.child_by_field(Field::End)?)?;
    // A degenerate or huge range is not worth enumerating (T084 flags bad
    // bounds; real corpora use single-digit ranges).
    (lo <= hi && hi.checked_sub(lo)? < 256).then_some((var, lo, hi))
}

fn literal_bound(n: Node) -> Option<i64> {
    let text = n.text().trim().trim_end_matches(['u', 'U']);
    let value = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()?
    } else {
        text.parse().ok()?
    };
    (value >= 0).then_some(value)
}

/// Every compile-time substitution of `$(VAR)` templates in `text` under the
/// active `expand` bindings — `expand` is text substitution (manual p.33), so
/// `Seg $(SEG).Volt` under `expand (SEG = 1 to 2)` accesses `Seg 1.Volt` AND
/// `Seg 2.Volt`. Text without templates passes through as itself; a runaway
/// combination count returns empty (treated as unresolvable).
pub(crate) fn substituted(text: &str, bindings: &ExpandBindings) -> Vec<String> {
    if !text.contains("$(") {
        return vec![text.to_string()];
    }
    let mut variants = vec![text.to_string()];
    for (var, lo, hi) in bindings {
        let needle = format!("$({var})");
        if !variants[0].contains(&needle) {
            continue;
        }
        let Some(count) = hi
            .checked_sub(*lo)
            .and_then(|n| n.checked_add(1))
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|n| variants.len().checked_mul(n))
            .filter(|&n| n > 0 && n <= 4096)
        else {
            return Vec::new();
        };
        let mut next = Vec::with_capacity(count);
        for v in &variants {
            for val in *lo..=*hi {
                next.push(v.replace(&needle, &val.to_string()));
            }
        }
        variants = next;
    }
    variants
}

/// The `expand` bindings in scope at `node`, gathered by walking up its
/// ancestors. Only integer-literal-bounded loops contribute a binding; a
/// constant/non-literal bound yields none, so a `$(VAR)` it governs stays
/// un-substituted — the caller then treats the still-templated path as
/// unprovable rather than a miss. Mirrors the innermost-last order that
/// [`crate::schedule`] threads top-down, though substitution is order-independent
/// (each variable is replaced by its own distinct `$(VAR)` needle).
pub(crate) fn enclosing_expand_bindings(node: &Node) -> ExpandBindings {
    let mut bindings = ExpandBindings::new();
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == Kind::ExpandStatement
            && let Some(b) = expand_binding(&n)
        {
            bindings.push(b);
        }
        cur = n.parent();
    }
    bindings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_bounds_are_checked_without_overflow() {
        for (lo, hi, accepted) in [
            (i64::MIN, i64::MAX, false),
            (-1, 1, false),
            (2, 1, false),
            (0, 255, true),
            (0, 256, false),
            (i64::MAX - 1, i64::MAX, true),
        ] {
            let cst = m1_core::parse(&format!("expand (I = {lo} to {hi}) {{ x = 1; }}"));
            let node = cst
                .root()
                .descendants()
                .find(|n| n.kind() == Kind::ExpandStatement)
                .unwrap();
            assert_eq!(expand_binding(&node).is_some(), accepted, "{lo}..{hi}");
        }
    }

    #[test]
    fn substitution_product_is_bounded_before_enumeration() {
        assert_eq!(
            substituted("$(I).$(J)", &vec![("I".into(), 0, 63), ("J".into(), 0, 63)]).len(),
            4096
        );
        assert!(
            substituted("$(I).$(J)", &vec![("I".into(), 0, 63), ("J".into(), 0, 64)]).is_empty()
        );
        for (lo, hi) in [(i64::MIN, i64::MAX), (0, i64::MAX), (2, 1)] {
            assert!(substituted("$(I)", &vec![("I".into(), lo, hi)]).is_empty());
        }
        assert_eq!(
            substituted("plain", &vec![("I".into(), 0, i64::MAX)]),
            ["plain"]
        );
    }
}
