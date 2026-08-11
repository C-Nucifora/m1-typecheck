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
use super::{CanDirection, CanMeta, Symbol, SymbolKind, SymbolTable, XmlParseError};
use crate::types::{ValueType, primitive_type};
use m1_workspace::LineIndex;

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
pub fn augment(
    table: &mut SymbolTable,
    xml: &str,
    rel_filename: &str,
) -> Result<(), XmlParseError> {
    let doc = roxmltree::Document::parse(xml).map_err(XmlParseError::in_context(".m1dbc"))?;
    // Map a component's byte offset to its 0-based source line so Go to Definition
    // opens the `.m1dbc` at the declaration, not line 0 (#169). Built once over the
    // whole parse — roxmltree's own `text_pos_at` rescans from byte 0 each call.
    let line_index = LineIndex::new(xml);
    for node in doc.descendants().filter(|n| n.has_tag_name("Component")) {
        let (Some(classname), Some(name)) = (node.attribute("Classname"), node.attribute("Name"))
        else {
            continue;
        };
        let Some(kind) = classify(classname) else {
            continue;
        };
        let props = node.children().find(|c| c.has_tag_name("Props"));
        let is_signal = kind == SymbolKind::Channel;
        let value_type = if is_signal {
            props
                .and_then(|p| p.attribute("Type"))
                .and_then(primitive_type)
                .unwrap_or(ValueType::Unknown)
        } else {
            ValueType::Unknown
        };
        // A signal's physical value range, derived from its bit Length + Type
        // (raw range) scaled by Multiplier/Offset. Used by T042.
        let dbc_range = if is_signal {
            props.and_then(signal_range)
        } else {
            None
        };
        // Signals carry their engineering unit in `Qty` (e.g. "deg", "m"); the
        // .m1prj never types DBC signals, so this is the only unit source.
        let unit = is_signal
            .then(|| props.and_then(|p| p.attribute("Qty")))
            .flatten()
            .filter(|q| !q.is_empty())
            .map(str::to_string);
        // CAN layout metadata for hover (#80): message frame (CANId/DLC) on
        // Message objects, bit layout + scaling on Signal channels.
        let can = props.and_then(|p| can_meta(p, classname));
        let class = (kind == SymbolKind::Object).then(|| classname.to_string());
        // Only the fields a `.m1dbc` actually sources are spelled here; every
        // other field (qty/display_unit/security/enum_assoc/call+log rate/tags/
        // return_type/in_params/table_meta/reference_target) is its inert
        // `Symbol::default()` — a CAN entry carries none of them.
        table.push(Symbol {
            path: name.to_string(),
            kind,
            value_type,
            unit,
            filename: Some(rel_filename.to_string()),
            class,
            classname: Some(classname.to_string()),
            def_line: Some(line_index.line_at(node.range().start) as u32),
            dbc_range,
            can,
            ..Symbol::default()
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
    let length = hex_uint(props.attribute("Length")?)? as i32;
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

/// Parse a `.m1dbc` integer attribute. MoTeC writes them in **hexadecimal
/// without a prefix** — the corpora prove it (`CANId="4B3"`, `StartBit="3F"`,
/// and AMK's 16-bit Speed field stored as `StartBit="10" Length="10"`) — so a
/// digit-only value like `133` is 0x133, not 133. An explicit `0x` prefix is
/// tolerated for hand-authored files.
fn hex_uint(s: &str) -> Option<u32> {
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u32::from_str_radix(s, 16).ok()
}

/// Retain the CAN layout fields a hover wants to show. For a
/// `BuiltIn.CAN.Message` that is the frame's `CANId` + `DLC`; for a
/// `BuiltIn.CAN.Signal` it is the `StartBit`/`Length` bit layout and the
/// `Multiplier`/`Offset` scaling. Returns `None` (rather than an empty struct)
/// when no relevant attribute is present, and for other classnames.
fn can_meta(props: roxmltree::Node<'_, '_>, classname: &str) -> Option<CanMeta> {
    let uint = |attr: &str| props.attribute(attr).and_then(hex_uint);
    let float = |attr: &str| props.attribute(attr).and_then(|s| s.parse::<f64>().ok());
    let meta = match classname {
        "BuiltIn.CAN.Message" => CanMeta {
            can_id: uint("CANId"),
            extended: props.attribute("IdType") == Some("Extended"),
            dlc: uint("DLC"),
            // `Transmit="RX"|"TX"` is the message's direction; anything else
            // (or absent) leaves it unknown so the direction check stays silent.
            transmit: props.attribute("Transmit").and_then(|t| match t {
                "RX" => Some(CanDirection::Rx),
                "TX" => Some(CanDirection::Tx),
                _ => None,
            }),
            ..Default::default()
        },
        "BuiltIn.CAN.Signal" => CanMeta {
            start_bit: uint("StartBit"),
            length: uint("Length"),
            multiplier: float("Multiplier"),
            offset: float("Offset"),
            ..Default::default()
        },
        _ => return None,
    };
    (meta != CanMeta::default()).then_some(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DBC: &str = r#"<?xml version="1.0"?>
<DBC>
 <ComponentStream>
  <List>
   <Component Classname="BuiltIn.CAN.DBC" Name="Balls3EV25"/>
   <Component Classname="BuiltIn.CAN.Message" Name="Balls3EV25.DashVals">
    <Props CANId="291" DLC="8" Transmit="RX"/>
   </Component>
   <Component Classname="BuiltIn.CAN.Signal" Name="Balls3EV25.DashVals.Inverter Error">
    <Props Type="u32" Qty="deg" StartBit="0" Length="10" Multiplier="0.5" Offset="2.0"/>
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
        let msg_can = msg.can.as_ref().expect("message CAN meta");
        // `CANId="291"` is hex: 0x291.
        assert_eq!(msg_can.can_id, Some(0x291));
        assert_eq!(msg_can.dlc, Some(8));
        // `Transmit="RX"` is captured as the receive direction (T109 input).
        assert_eq!(msg_can.transmit, Some(CanDirection::Rx));

        let sig = table
            .get("Balls3EV25.DashVals.Inverter Error")
            .expect("signal symbol");
        assert_eq!(sig.kind, SymbolKind::Channel);
        assert_eq!(sig.value_type, ValueType::Unsigned); // u32
        assert_eq!(sig.unit.as_deref(), Some("deg")); // from Qty
        let sig_can = sig.can.as_ref().expect("signal CAN meta");
        assert_eq!(sig_can.start_bit, Some(0));
        // `Length="10"` is hex: a 16-bit field.
        assert_eq!(sig_can.length, Some(0x10));
        assert_eq!(sig_can.multiplier, Some(0.5));
        assert_eq!(sig_can.offset, Some(2.0));

        let boolsig = table
            .get("Balls3EV25.DashVals.Aux Switch")
            .expect("bool signal");
        assert_eq!(boolsig.value_type, ValueType::Boolean);
    }

    // A message's `Transmit` direction is captured (or left `None`) so the
    // message-method-vs-direction check (T109) has an input: `RX` → Rx, `TX` →
    // Tx, and any other/absent value stays unknown (never guessed).
    #[test]
    fn captures_message_transmit_direction() {
        let xml = r#"<?xml version="1.0"?>
<DBC>
 <ComponentStream>
  <List>
   <Component Classname="BuiltIn.CAN.DBC" Name="Bus"/>
   <Component Classname="BuiltIn.CAN.Message" Name="Bus.RxMsg"><Props CANId="1" DLC="8" Transmit="RX"/></Component>
   <Component Classname="BuiltIn.CAN.Message" Name="Bus.TxMsg"><Props CANId="2" DLC="8" Transmit="TX"/></Component>
   <Component Classname="BuiltIn.CAN.Message" Name="Bus.NoDir"><Props CANId="3" DLC="8"/></Component>
  </List>
 </ComponentStream>
</DBC>"#;
        let mut table = SymbolTable::default();
        augment(&mut table, xml, "dbc/Bus.m1dbc").unwrap();
        assert_eq!(
            table
                .get("Bus.RxMsg")
                .unwrap()
                .can
                .as_ref()
                .unwrap()
                .transmit,
            Some(CanDirection::Rx)
        );
        assert_eq!(
            table
                .get("Bus.TxMsg")
                .unwrap()
                .can
                .as_ref()
                .unwrap()
                .transmit,
            Some(CanDirection::Tx)
        );
        // No `Transmit` → unknown direction; the CAN meta still exists (CANId/DLC).
        assert_eq!(
            table
                .get("Bus.NoDir")
                .unwrap()
                .can
                .as_ref()
                .unwrap()
                .transmit,
            None
        );
    }

    // `.m1dbc` integer attributes are HEXADECIMAL without a prefix — the real
    // corpora prove it: the DTI inverter files declare `CANId="4B3"`/`"4D4"`
    // (impossible as decimal) whose ids only satisfy the DTI `command<<5|node`
    // scheme read as hex, and the AMK file's Speed signal is
    // `StartBit="10" Length="10"` — the datasheet's 16-bit field at bit 16.
    #[test]
    fn parses_can_id_as_hex() {
        let xml = r#"<?xml version="1.0"?>
<DBC>
 <ComponentStream>
  <List>
   <Component Classname="BuiltIn.CAN.DBC" Name="DTI"/>
   <Component Classname="BuiltIn.CAN.Message" Name="DTI.Lettered"><Props CANId="4B3" DLC="8"/></Component>
   <Component Classname="BuiltIn.CAN.Message" Name="DTI.DigitsOnly"><Props CANId="133" DLC="8"/></Component>
  </List>
 </ComponentStream>
</DBC>"#;
        let mut table = SymbolTable::default();
        augment(&mut table, xml, "dbc/DTI.m1dbc").unwrap();
        // A hex-lettered id must not be dropped …
        assert_eq!(
            table
                .get("DTI.Lettered")
                .unwrap()
                .can
                .as_ref()
                .unwrap()
                .can_id,
            Some(0x4B3)
        );
        // … and a digit-only id is still hex: `133` is 0x133 = 307, not 133.
        assert_eq!(
            table
                .get("DTI.DigitsOnly")
                .unwrap()
                .can
                .as_ref()
                .unwrap()
                .can_id,
            Some(0x133)
        );
    }

    #[test]
    fn parses_signal_layout_as_hex() {
        let xml = r#"<?xml version="1.0"?>
<DBC>
 <ComponentStream>
  <List>
   <Component Classname="BuiltIn.CAN.DBC" Name="AMK"/>
   <Component Classname="BuiltIn.CAN.Message" Name="AMK.Actual Values 1"><Props CANId="284" DLC="8"/></Component>
   <Component Classname="BuiltIn.CAN.Signal" Name="AMK.Actual Values 1.Speed">
    <Props Type="s32" StartBit="10" Length="10" Multiplier="1.00000000000000000e+00" Offset="0.00000000000000000e+00"/>
   </Component>
   <Component Classname="BuiltIn.CAN.Signal" Name="AMK.Actual Values 1.Last Bit">
    <Props Type="bool" StartBit="3F" Length="1"/>
   </Component>
  </List>
 </ComponentStream>
</DBC>"#;
        let mut table = SymbolTable::default();
        augment(&mut table, xml, "dbc/AMK.m1dbc").unwrap();
        let speed = table.get("AMK.Actual Values 1.Speed").unwrap();
        let speed_can = speed.can.as_ref().unwrap();
        // AMK's actual-speed field: 16 bits (0x10) at bit 16 (0x10).
        assert_eq!(speed_can.start_bit, Some(16));
        assert_eq!(speed_can.length, Some(16));
        // The T042 range follows the 16-bit width: s16 raw −32768..32767.
        assert_eq!(speed.dbc_range, Some((-32768.0, 32767.0)));
        // Bit 0x3F = 63, the last bit of an 8-byte frame — droppable as decimal 3F.
        assert_eq!(
            table
                .get("AMK.Actual Values 1.Last Bit")
                .unwrap()
                .can
                .as_ref()
                .unwrap()
                .start_bit,
            Some(63)
        );
    }

    // `IdType="Extended"` marks a 29-bit id (the corpora only write IdType when
    // extended); its absence means a standard 11-bit frame.
    #[test]
    fn captures_extended_id_type() {
        let xml = r#"<?xml version="1.0"?>
<DBC>
 <ComponentStream>
  <List>
   <Component Classname="BuiltIn.CAN.DBC" Name="Steering"/>
   <Component Classname="BuiltIn.CAN.Message" Name="Steering.Status"><Props IdType="Extended" CANId="2968" DLC="8"/></Component>
   <Component Classname="BuiltIn.CAN.Message" Name="Steering.Std"><Props CANId="171" DLC="8"/></Component>
  </List>
 </ComponentStream>
</DBC>"#;
        let mut table = SymbolTable::default();
        augment(&mut table, xml, "dbc/Steering.m1dbc").unwrap();
        let ext = table.get("Steering.Status").unwrap().can.clone().unwrap();
        assert_eq!(ext.can_id, Some(0x2968));
        assert!(ext.extended);
        assert!(
            !table
                .get("Steering.Std")
                .unwrap()
                .can
                .as_ref()
                .unwrap()
                .extended
        );
    }

    // #169 gap 1: each DBC symbol records the 0-based source line of its
    // `<Component>` element, so Go to Definition opens the `.m1dbc` at the
    // declaration instead of always at line 0.
    #[test]
    fn captures_def_line_for_dbc_components() {
        let mut table = SymbolTable::default();
        augment(&mut table, DBC, "dbc/Balls3EV25.m1dbc").unwrap();
        // Lines are 0-based; the DBC/Message/Signal <Component> elements sit on
        // their respective rows in DBC above.
        assert_eq!(table.get("Balls3EV25").unwrap().def_line, Some(4));
        assert_eq!(table.get("Balls3EV25.DashVals").unwrap().def_line, Some(5));
        assert_eq!(
            table
                .get("Balls3EV25.DashVals.Inverter Error")
                .unwrap()
                .def_line,
            Some(8)
        );
    }
}
