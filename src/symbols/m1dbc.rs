//! Parse a `.m1dbc` (MoTeC CAN DBC) file and add its objects to a SymbolTable.
//!
//! A `.m1dbc` declares CAN objects in a `<ComponentStream><List>` of
//! `<Component Classname="BuiltIn.CAN.{DBC,Message,Signal}" Name="…">` nodes.
//! Scripts initialise and use these (e.g. `Balls3EV25.Init(0)`), so they must
//! be known to the project for resolution, hover, goto, and signal types.
//!
//! Mapping:
//!   * `BuiltIn.CAN.DBC`     → [`SymbolKind::Object`] (the DBC root)
//!   * `BuiltIn.CAN.Message` → [`SymbolKind::Object`]
//!   * `BuiltIn.CAN.Signal`  → [`SymbolKind::Channel`] (typed from `Props/@Type`)
//!
//! DBC/Message are objects so their built-in methods (`.Init`, `.Transmit`, …)
//! resolve as opaque accessors rather than being flagged.
use super::{Symbol, SymbolKind, SymbolTable};
use crate::types::{ValueType, primitive_type};

#[derive(Debug)]
pub enum DbcError {
    Xml(roxmltree::Error),
}
impl std::fmt::Display for DbcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbcError::Xml(e) => write!(f, "invalid .m1dbc XML: {e}"),
        }
    }
}
impl std::error::Error for DbcError {}

/// Classify a `BuiltIn.CAN.*` DBC component classname. Returns `None` for
/// classnames that are not CAN objects (organisation wrappers, etc.).
fn classify(classname: &str) -> Option<SymbolKind> {
    match classname {
        "BuiltIn.CAN.DBC" | "BuiltIn.CAN.Message" => Some(SymbolKind::Object),
        "BuiltIn.CAN.Signal" => Some(SymbolKind::Channel),
        _ => None,
    }
}

/// Parse `xml` and push its CAN objects into `table`. `rel_filename` is the
/// `.m1dbc` path relative to the project root, stored on each symbol so goto
/// can open the defining file.
pub fn augment(table: &mut SymbolTable, xml: &str, rel_filename: &str) -> Result<(), DbcError> {
    let doc = roxmltree::Document::parse(xml).map_err(DbcError::Xml)?;
    for node in doc.descendants().filter(|n| n.has_tag_name("Component")) {
        let (Some(classname), Some(name)) = (node.attribute("Classname"), node.attribute("Name"))
        else {
            continue;
        };
        let Some(kind) = classify(classname) else {
            continue;
        };
        let props = node.children().find(|c| c.has_tag_name("Props"));
        let value_type = if kind == SymbolKind::Channel {
            props
                .and_then(|p| p.attribute("Type"))
                .and_then(primitive_type)
                .unwrap_or(ValueType::Unknown)
        } else {
            ValueType::Unknown
        };
        // A signal's physical value range, derived from its bit Length + Type
        // (raw range) scaled by Multiplier/Offset. Used by T042.
        let dbc_range = if kind == SymbolKind::Channel {
            props.and_then(signal_range)
        } else {
            None
        };
        let class = (kind == SymbolKind::Object).then(|| classname.to_string());
        table.push(Symbol {
            path: name.to_string(),
            kind,
            value_type,
            declared_type: None,
            unit: None,
            security: None,
            filename: Some(rel_filename.to_string()),
            enum_assoc: None,
            class,
            def_line: None,
            dbc_range,
        });
    }
    Ok(())
}

/// Derive a CAN signal's physical `(min, max)` range from its `.m1dbc` `<Props>`:
/// the raw range from `Length` + `Type` (unsigned `0..2^n-1`, signed
/// `-2^(n-1)..2^(n-1)-1`, `bool` `0..1`), scaled by `Multiplier` (default 1) and
/// shifted by `Offset` (default 0). `None` for float signals (`f32`/`f64` —
/// unbounded for this check) or a missing/zero `Length`.
fn signal_range(props: roxmltree::Node<'_, '_>) -> Option<(f64, f64)> {
    let ty = props.attribute("Type")?;
    let length: i32 = props.attribute("Length")?.parse().ok()?;
    if length <= 0 {
        return None;
    }
    let mult: f64 = props
        .attribute("Multiplier")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let offset: f64 = props
        .attribute("Offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let (raw_min, raw_max) = match ty {
        "bool" => (0.0, 1.0),
        t if t.starts_with('u') => (0.0, 2f64.powi(length) - 1.0),
        t if t.starts_with('s') => {
            let half = 2f64.powi(length - 1);
            (-half, half - 1.0)
        }
        _ => return None, // f32/f64 — no bounded integer range
    };
    let (a, b) = (raw_min * mult + offset, raw_max * mult + offset);
    Some((a.min(b), a.max(b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DBC: &str = r#"<?xml version="1.0"?>
<DBC>
 <ComponentStream>
  <List>
   <Component Classname="BuiltIn.CAN.DBC" Name="Balls3EV25"/>
   <Component Classname="BuiltIn.CAN.Message" Name="Balls3EV25.DashVals"/>
   <Component Classname="BuiltIn.CAN.Signal" Name="Balls3EV25.DashVals.Inverter Error">
    <Props Type="u32" StartBit="0" Length="10"/>
   </Component>
   <Component Classname="BuiltIn.CAN.Signal" Name="Balls3EV25.DashVals.Aux Switch">
    <Props Type="bool" StartBit="10" Length="1"/>
   </Component>
  </List>
 </ComponentStream>
</DBC>"#;

    #[test]
    fn parses_dbc_objects_and_signal_types() {
        let mut table = SymbolTable::default();
        augment(&mut table, DBC, "dbc/Balls3EV25.m1dbc").unwrap();

        let dbc = table.get("Balls3EV25").expect("DBC root symbol");
        assert_eq!(dbc.kind, SymbolKind::Object);
        assert_eq!(dbc.filename.as_deref(), Some("dbc/Balls3EV25.m1dbc"));

        let msg = table.get("Balls3EV25.DashVals").expect("message symbol");
        assert_eq!(msg.kind, SymbolKind::Object);

        let sig = table
            .get("Balls3EV25.DashVals.Inverter Error")
            .expect("signal symbol");
        assert_eq!(sig.kind, SymbolKind::Channel);
        assert_eq!(sig.value_type, ValueType::Unsigned); // u32

        let boolsig = table
            .get("Balls3EV25.DashVals.Aux Switch")
            .expect("bool signal");
        assert_eq!(boolsig.value_type, ValueType::Boolean);
    }
}
