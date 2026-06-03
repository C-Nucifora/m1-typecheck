//! Authoritative M1 base units.
//!
//! Per the M1 Development Manual ("Quantities and Base Units"): every floating
//! point object is assigned a *quantity*, and each quantity defines a *base
//! unit* — the unit in which the value is actually stored and in which all
//! calculations are performed. The `.m1prj` records the quantity on
//! `<Props Qty="…">` as an **SI** unit string (e.g. `rad/s`, `K`, `Pa`).
//!
//! The base unit equals the SI quantity unit, with the manual's two documented
//! exceptions: **Angle uses degrees, not radians**, and **Temperature uses
//! Celsius, not Kelvin**. The `<Default Unit="…">` on an object is only the
//! *display* unit (e.g. `rpm`, `kPa`) and must NOT be treated as the stored unit.

/// Map one unit token through the M1 SI→base-unit exceptions.
fn map_token(tok: &str) -> &str {
    match tok {
        "rad" => "deg",
        "K" => "C",
        "dK" => "dC",
        other => other,
    }
}

/// The authoritative M1 base unit for a `<Props Qty="…">` SI unit string.
///
/// Returns `None` for an empty quantity or a `$(…)` template expression (which
/// is an inherited reference, not a literal unit).
pub fn base_unit_of_qty(qty: &str) -> Option<String> {
    let q = qty.trim();
    if q.is_empty() || q.contains("$(") {
        return None;
    }
    // Walk the unit expression, mapping each maximal alphanumeric token through
    // the SI→base exceptions while preserving separators (`/`, `.`).
    let mut out = String::new();
    let mut tok = String::new();
    for ch in q.chars() {
        if ch.is_ascii_alphanumeric() {
            tok.push(ch);
        } else {
            if !tok.is_empty() {
                out.push_str(map_token(&tok));
                tok.clear();
            }
            out.push(ch);
        }
    }
    if !tok.is_empty() {
        out.push_str(map_token(&tok));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_quantities_use_degrees_not_radians() {
        assert_eq!(base_unit_of_qty("rad").as_deref(), Some("deg"));
        assert_eq!(base_unit_of_qty("rad/s").as_deref(), Some("deg/s"));
        assert_eq!(base_unit_of_qty("rad/s/s").as_deref(), Some("deg/s/s"));
    }

    #[test]
    fn temperature_quantities_use_celsius_not_kelvin() {
        assert_eq!(base_unit_of_qty("K").as_deref(), Some("C"));
        assert_eq!(base_unit_of_qty("dK").as_deref(), Some("dC"));
    }

    #[test]
    fn other_si_quantities_are_unchanged() {
        for q in [
            "Pa", "m", "m/s", "m/s/s", "V", "W", "J", "ohm", "N.m", "Hz", "s", "N", "N/m", "A",
            "A.s", "m3/s", "kg", "ratio",
        ] {
            assert_eq!(
                base_unit_of_qty(q).as_deref(),
                Some(q),
                "qty {q} should be unchanged"
            );
        }
    }

    #[test]
    fn lowercase_k_is_not_kelvin() {
        // `kg` (kilogram) and `kPa` must not have their lowercase k rewritten.
        assert_eq!(base_unit_of_qty("kg").as_deref(), Some("kg"));
        assert_eq!(base_unit_of_qty("kPa").as_deref(), Some("kPa"));
    }

    #[test]
    fn empty_and_template_quantities_are_none() {
        assert_eq!(base_unit_of_qty(""), None);
        assert_eq!(base_unit_of_qty("  "), None);
        assert_eq!(base_unit_of_qty("$(Parent.Value:Qty)"), None);
    }
}
