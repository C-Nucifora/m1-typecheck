//! Value-type inference for `.m1prj` channels that carry no explicit `<Props
//! Type>`/`Qty`: derive a type from the owning object's package class, falling
//! back to a conservative leaf-name physical-quantity heuristic.

use crate::types::ValueType;
use std::collections::HashMap;

use super::parent_group;

/// The known output [`ValueType`] for a package class, or `None` when the class
/// doesn't determine one (a group, a CAN/comms container, an unmapped class) —
/// in which case the ancestor walk stops and falls back to the leaf name. Each
/// mapping is a known output type for that package class; classes whose output is
/// genuinely indeterminate (`MoTeC Comms.CAN Bus`, `BuiltIn.Reference`, the odd
/// built-ins) stay `Unknown`.
fn class_value_type(class: &str) -> Option<ValueType> {
    match class {
        // Physical-measurement sensors and float-producing IO methods.
        // `MoTeC Input.*` = temperature/acceleration/pressure/…; `ratio` =
        // dimensionless; `abs` = magnitude of a signal — all FloatingPoint.
        c if c.starts_with("MoTeC Input.") => Some(ValueType::Float),
        "_IOMethod.ratio" | "_IOMethod.abs" => Some(ValueType::Float),
        // A switched output is on/off.
        "MoTeC Output.Switched Output" => Some(ValueType::Boolean),
        // A table's auto-created `.Value` output is always a floating-point
        // quantity — the interpolated result is real-valued regardless of the
        // table's tune (#25, case 1).
        "BuiltIn.Table" | "BuiltIn.CalibrationTable" => Some(ValueType::Float),
        _ => None,
    }
}

/// Leaf names whose final word is an unambiguous physical quantity (always a
/// measured `FloatingPoint`). Deliberately conservative — only words that are
/// never an enum/integer *state* — so this heuristic can't introduce false
/// type-mismatch diagnostics on real code (#103).
const FLOAT_QUANTITY_WORDS: &[&str] = &[
    "Voltage",
    "Current",
    "Temperature",
    "Pressure",
    "Frequency",
    "Power",
    "Torque",
    "Force",
    "Acceleration",
    "Velocity",
    "Resistance",
    "Energy",
    "Humidity",
    "Inductance",
    "Capacitance",
];

/// Infer `FloatingPoint` from a channel's leaf name when its last space-separated
/// word is an unambiguous physical quantity; otherwise `Unknown`.
pub(super) fn type_from_leaf_name(path: &str) -> ValueType {
    let leaf = path.rsplit('.').next().unwrap_or(path);
    let last_word = leaf.rsplit(' ').next().unwrap_or(leaf);
    if FLOAT_QUANTITY_WORDS
        .iter()
        .any(|q| q.eq_ignore_ascii_case(last_word))
    {
        ValueType::Float
    } else {
        ValueType::Unknown
    }
}

/// Derive an untyped channel's value type from the class of the object that owns
/// it. Walks up through any intervening *sub-channel* ancestors (`.Value`,
/// `.Filtered`, …) to the nearest owning object class, so a sub-channel inherits
/// its object's type rather than stopping at its immediate channel parent (#103,
/// extending #25). The walk does NOT cross a GroupCompound (groups regroup
/// heterogeneous children — crossing would mis-type). When the class doesn't
/// determine a type, falls back to a conservative leaf-name quantity heuristic.
pub(super) fn type_from_owner_class(
    path: &str,
    classname_by_path: &HashMap<String, String>,
) -> ValueType {
    let mut cur = parent_group(path);
    while let Some(p) = cur {
        match classname_by_path.get(&p).map(String::as_str) {
            // A sub-channel link in the chain: keep walking up to the object.
            Some(c) if c.starts_with("BuiltIn.Channel") || c == "BuiltIn.CalibrationChannel" => {
                cur = parent_group(&p);
            }
            // The owning object's class decides (or stops the walk).
            Some(c) => return class_value_type(c).unwrap_or_else(|| type_from_leaf_name(path)),
            // No component at this ancestor path — fall back to the leaf name.
            None => return type_from_leaf_name(path),
        }
    }
    type_from_leaf_name(path)
}
