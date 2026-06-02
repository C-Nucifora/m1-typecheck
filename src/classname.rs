//! Component `Classname` schema (#47).
//!
//! Every `.m1prj` `<Component>` declares a `Classname` (`BuiltIn.Channel`,
//! `BuiltIn.Parameter`, `BuiltIn.CAN.Signal`, `MoTeC Input.Sensor`, …) that the
//! parser already stores verbatim on [`crate::symbols::Symbol::classname`]. This
//! module adds the *structural* half: which parent classes a given child class is
//! allowed to nest under.
//!
//! ## Deliberately conservative
//!
//! The real vehicle corpus shows a rich, varied parent→child class graph — group
//! compounds, sensor/output objects (`MoTeC Input.Sensor`, `_IOMethod.*`,
//! `_CalibrationSchema.*`) and tables all legitimately contain channels,
//! references and methods. Hard-coding a full "valid children per class" matrix
//! would fire false positives across that corpus, so [`valid_parent_classes`]
//! returns `None` (= *unconstrained*, never flagged) for every class whose nesting
//! is not an unambiguous invariant.
//!
//! The only constraints encoded today are the CAN frame hierarchy, which *is*
//! unambiguous (a signal lives in a message; a message lives in a DBC) and whose
//! constrained classes (`BuiltIn.CAN.Signal`, `BuiltIn.CAN.Message`) do not appear
//! as authored components anywhere in the corpus — so the check is provably
//! zero-false-positive on real projects while still catching a hand-authored
//! mistake like a `CAN.Signal` placed directly under a `GroupCompound`.

/// Whether `classname` is a built-in M1 component class (`BuiltIn.*`) as opposed
/// to a package-defined object class (`MoTeC Input.Sensor`, `_IOMethod.abs`, …).
pub fn is_builtin(classname: &str) -> bool {
    classname.starts_with("BuiltIn.")
}

/// The set of parent component classes a child of class `child` is allowed to
/// nest directly under, or `None` when the class is *unconstrained* (any parent
/// is acceptable — the common case, see the module docs).
///
/// A returned slice is exhaustive: a parent class outside it is a violation.
pub fn valid_parent_classes(child: &str) -> Option<&'static [&'static str]> {
    match child {
        // A CAN signal is a leaf of exactly one CAN message.
        "BuiltIn.CAN.Signal" => Some(&["BuiltIn.CAN.Message"]),
        // A CAN message belongs to a CAN database root.
        "BuiltIn.CAN.Message" => Some(&["BuiltIn.CAN.DBC", "BuiltIn.CAN.DBCRoot"]),
        _ => None,
    }
}

/// Check a single parent/child class pairing. Returns `None` when the pairing is
/// valid (or the child class is unconstrained), or `Some(expected)` — the list of
/// allowed parent classes — when `parent` is not among them.
pub fn parent_violation(child: &str, parent: &str) -> Option<&'static [&'static str]> {
    match valid_parent_classes(child) {
        Some(allowed) if !allowed.contains(&parent) => Some(allowed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_detection() {
        assert!(is_builtin("BuiltIn.Channel"));
        assert!(is_builtin("BuiltIn.CAN.Signal"));
        assert!(!is_builtin("MoTeC Input.Sensor"));
        assert!(!is_builtin("_IOMethod.abs"));
    }

    #[test]
    fn unconstrained_classes_never_flag() {
        // The overwhelmingly common classes carry no parent constraint, so they
        // can sit under any container without a T010.
        for c in [
            "BuiltIn.Channel",
            "BuiltIn.Parameter",
            "BuiltIn.GroupCompound",
            "BuiltIn.Table",
            "MoTeC Input.Sensor",
            "_IOMethod.abs",
        ] {
            assert!(
                valid_parent_classes(c).is_none(),
                "{c} should be unconstrained"
            );
            assert!(parent_violation(c, "BuiltIn.GroupCompound").is_none());
        }
    }

    #[test]
    fn can_signal_must_be_under_a_message() {
        // Valid placement: no violation.
        assert!(parent_violation("BuiltIn.CAN.Signal", "BuiltIn.CAN.Message").is_none());
        // The issue's example: a signal directly under a group is a violation.
        let expected = parent_violation("BuiltIn.CAN.Signal", "BuiltIn.GroupCompound");
        assert_eq!(expected, Some(&["BuiltIn.CAN.Message"][..]));
    }

    #[test]
    fn can_message_must_be_under_a_dbc() {
        assert!(parent_violation("BuiltIn.CAN.Message", "BuiltIn.CAN.DBC").is_none());
        assert!(parent_violation("BuiltIn.CAN.Message", "BuiltIn.CAN.DBCRoot").is_none());
        assert!(parent_violation("BuiltIn.CAN.Message", "BuiltIn.GroupCompound").is_some());
    }
}
