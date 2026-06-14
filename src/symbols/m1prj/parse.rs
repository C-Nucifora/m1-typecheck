//! The `.m1prj` parse pass: walk the project XML and build the `SymbolTable`,
//! enum types, file→group map, and module-root set.

use super::Parsed;
use super::component::{parse_enums, symbol_from_component};
use super::index::ProjectXmlIndex;
use crate::symbols::{SymbolTable, XmlParseError};
use m1_workspace::LineIndex;
use std::collections::{HashMap, HashSet};

pub fn parse(xml: &str) -> Result<Parsed, XmlParseError> {
    let doc = roxmltree::Document::parse(xml).map_err(XmlParseError::in_context(".m1prj"))?;
    // Map byte offsets to 0-based source lines in O(log N) for per-component
    // goto-definition, instead of roxmltree's `text_pos_at` (documented "very
    // expensive, use only for errors" — its per-component use made parsing
    // O(N^2), #95). Shared with the other XML loaders via `m1_workspace`.
    let line_index = LineIndex::new(xml);
    let mut table = SymbolTable::default();
    let mut file_to_group = HashMap::new();
    let mut module_roots = HashSet::new();

    // Pre-pass: index every component's path -> Classname and -> declared tags,
    // so the component pass can type/tag a node regardless of document order.
    let index = ProjectXmlIndex::build(&doc);

    // Register all enum types (firmware-supplied "open" enums plus project-local
    // `<Type Storage="enum">` declarations) before any component is typed.
    parse_enums(&doc, &mut table);

    for node in doc.descendants() {
        match node.tag_name().name() {
            "File" if node.parent().map(|p| p.tag_name().name()) == Some("SelectedModuleSets") => {
                if let Some(name) = node.attribute("Name") {
                    module_roots.insert(name.to_string());
                }
            }
            "Component" => {
                if let Some(sym) =
                    symbol_from_component(node, &index, &table, &line_index, &mut file_to_group)
                {
                    table.push(sym);
                }
            }
            _ => {}
        }
    }
    Ok(Parsed {
        table,
        file_to_group,
        module_roots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::SymbolKind;
    use crate::types::ValueType;

    #[test]
    fn symbols_record_their_m1prj_definition_line() {
        // 0-based: line 0 = <?xml>, 1 = <Project>, 2 = Root.A, 3 = Root.B.
        let xml = "<?xml version=\"1.0\"?>\n<Project>\n  <Component Classname=\"BuiltIn.Channel\" Name=\"Root.A\"><Props Type=\"u32\"/></Component>\n  <Component Classname=\"BuiltIn.Channel\" Name=\"Root.B\"><Props Type=\"s32\"/></Component>\n</Project>";
        let parsed = parse(xml).unwrap();
        assert_eq!(parsed.table.get("Root.A").unwrap().def_line, Some(2));
        assert_eq!(parsed.table.get("Root.B").unwrap().def_line, Some(3));
    }

    // #171: a channel's `<Props DefaultLogRate="5MS">` records the default
    // logging period; we expose it as a rate in Hz (5 ms period => 200 Hz) for
    // the hover badge. Channels with no DefaultLogRate carry `None`.
    #[test]
    fn channel_records_default_log_rate_as_hz() {
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Channel" Name="Root.A"><Props Type="u32" DefaultLogRate="5MS"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.B"><Props Type="u32" DefaultLogRate="10MS"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.C"><Props Type="u32"/></Component>
</Project>"#;
        let parsed = parse(xml).unwrap();
        assert_eq!(parsed.table.get("Root.A").unwrap().log_rate_hz, Some(200.0));
        assert_eq!(parsed.table.get("Root.B").unwrap().log_rate_hz, Some(100.0));
        assert_eq!(parsed.table.get("Root.C").unwrap().log_rate_hz, None);
    }

    // #170: a channel records its `<Props SelectedTags="…">` (space-separated),
    // and inherits the tags of every ancestor group (the manual: "tags assigned
    // to a group node are inherited by the children of the group"). The union is
    // queryable by tag via the tag index.
    #[test]
    fn channel_records_and_inherits_selected_tags() {
        use std::collections::HashSet;
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Demo"><Props SelectedTags="Engine"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Demo.Widget Count"><Props Type="u32" SelectedTags="Normal Vehicle"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Demo.Mode"><Props Type="u32"/></Component>
</Project>"#;
        let parsed = parse(xml).unwrap();
        // Own tags come first, in declared order; the inherited group tag follows.
        assert_eq!(
            parsed.table.get("Root.Demo.Widget Count").unwrap().tags,
            vec!["Normal", "Vehicle", "Engine"]
        );
        // No own tags -> inherits the group's tag set.
        assert_eq!(
            parsed.table.get("Root.Demo.Mode").unwrap().tags,
            vec!["Engine"]
        );
        // Tag index returns the group itself and both descendants for `Engine`.
        let engine: HashSet<&str> = parsed
            .table
            .symbols_with_tag("Engine")
            .iter()
            .map(|s| s.path.as_str())
            .collect();
        assert!(engine.contains("Root.Demo"));
        assert!(engine.contains("Root.Demo.Widget Count"));
        assert!(engine.contains("Root.Demo.Mode"));
        // A tag only on the channel is not attributed to the parent group.
        let vehicle: Vec<&str> = parsed
            .table
            .symbols_with_tag("Vehicle")
            .iter()
            .map(|s| s.path.as_str())
            .collect();
        assert_eq!(vehicle, vec!["Root.Demo.Widget Count"]);
    }

    #[test]
    fn channel_gets_enum_type_from_props_this_reference() {
        // A channel declared with `<Props Type="::This.<EnumName>">` must be
        // associated with the enum defined in the project's <DataTypes>.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <DataTypes>
    <Type Name="Drive State" Storage="enum" Default="Off">
      <Enum Name="Off" ContainerOrder="0"/>
      <Enum Name="On" ContainerOrder="1"/>
    </Type>
  </DataTypes>
  <Component Classname="BuiltIn.Channel" Name="Root.Control.State">
    <Props Type="::This.Drive State"/>
  </Component>
</Project>"#;
        let parsed = parse(xml).unwrap();
        let id = parsed
            .table
            .enum_by_name("Drive State")
            .expect("enum loaded");
        let sym = parsed.table.get("Root.Control.State").expect("channel");
        assert_eq!(sym.value_type, ValueType::Enum(id));
        assert_eq!(sym.enum_assoc, Some(id));
    }

    #[test]
    fn channel_gets_primitive_type_from_props() {
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Channel" Name="Root.A"><Props Type="u32"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.B"><Props Type="s32"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.C"><Props Type="f32"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.D"><Props Type="bool"/></Component>
</Project>"#;
        let parsed = parse(xml).unwrap();
        assert_eq!(
            parsed.table.get("Root.A").unwrap().value_type,
            ValueType::Unsigned
        );
        assert_eq!(
            parsed.table.get("Root.B").unwrap().value_type,
            ValueType::Integer
        );
        assert_eq!(
            parsed.table.get("Root.C").unwrap().value_type,
            ValueType::Float
        );
        assert_eq!(
            parsed.table.get("Root.D").unwrap().value_type,
            ValueType::Boolean
        );
    }

    #[test]
    fn unresolvable_props_type_stays_unknown() {
        // A derived-type template (`$(…)`) and a typeless channel are left
        // Unknown rather than guessed. (Firmware enums — `MoTeC Types.*` /
        // `::Hardware.*` — are handled separately, see
        // `firmware_enum_props_type_resolves_to_open_enum`.)
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Channel" Name="Root.N"><Props Type="$(Parent.Value:Type)"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.O"/>
</Project>"#;
        let parsed = parse(xml).unwrap();
        assert_eq!(
            parsed.table.get("Root.N").unwrap().value_type,
            ValueType::Unknown
        );
        assert_eq!(
            parsed.table.get("Root.O").unwrap().value_type,
            ValueType::Unknown
        );
    }

    #[test]
    fn firmware_enum_props_type_resolves_to_open_enum() {
        // A channel typed with a firmware-supplied enum resolves to an Enum
        // value type (so T021 and enum hover work) rather than Unknown. A type
        // the builtin help-capture catalogue documents (`MoTeC Types.Mode
        // Enumeration`) registers CLOSED with its members; an undocumented one
        // (`::Hardware.relay.state`) stays an "open" placeholder, so
        // membership-based checks (T020/T070) must not fire on it.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Channel" Name="Root.M"><Props Type="MoTeC Types.Mode Enumeration"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.H"><Props Type="::Hardware.relay.state"/></Component>
</Project>"#;
        let parsed = parse(xml).unwrap();

        let m = parsed.table.get("Root.M").unwrap();
        let mid = m.enum_assoc.expect("firmware enum associated");
        assert_eq!(m.value_type, ValueType::Enum(mid));
        assert_eq!(parsed.table.enum_type(mid).name, "Mode Enumeration");
        assert!(
            !parsed.table.enum_is_open(mid),
            "catalogue-documented firmware enum is closed"
        );
        assert!(parsed.table.enum_has_member(mid, "Disabled"));
        assert!(parsed.table.enum_has_member(mid, "Enabled"));

        let h = parsed.table.get("Root.H").unwrap();
        let hid = h.enum_assoc.expect("firmware enum associated");
        assert_eq!(h.value_type, ValueType::Enum(hid));
        assert_eq!(parsed.table.enum_type(hid).name, "relay.state");
        assert!(parsed.table.enum_is_open(hid), "undocumented stays open");
    }

    #[test]
    fn documented_firmware_enum_registers_closed_under_display_name() {
        // A firmware enum whose membership the M1 Development Manual documents
        // (`sw_state` → "Universal Switch State", Off/On) registers CLOSED under
        // its script-facing display name, so T020/T070 fire on it exactly as
        // M1 Build does (Errors 1352/1306) — see #167/#168.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Channel" Name="Root.S"><Props Type="::Hardware.av_switch.sw_state"/></Component>
</Project>"#;
        let parsed = parse(xml).unwrap();
        let s = parsed.table.get("Root.S").unwrap();
        let id = s.enum_assoc.expect("documented firmware enum associated");
        assert_eq!(s.value_type, ValueType::Enum(id));
        let e = parsed.table.enum_type(id);
        assert_eq!(e.name, "Universal Switch State");
        assert!(!e.open, "documented membership is authoritative");
        assert_eq!(
            e.members,
            vec![("Off".to_string(), 0), ("On".to_string(), 1)]
        );
    }

    #[test]
    fn channel_with_qty_but_no_type_is_float() {
        // A measured channel declares a physical quantity (Qty) and no explicit
        // storage Type; it is stored as Float. (A bare channel with neither
        // stays Unknown.)
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Channel" Name="Root.Speed"><Props Qty="rad/s" Security="Tune"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Volts"><Props Qty="V"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Bare"/>
</Project>"#;
        let parsed = parse(xml).unwrap();
        assert_eq!(
            parsed.table.get("Root.Speed").unwrap().value_type,
            ValueType::Float
        );
        assert_eq!(
            parsed.table.get("Root.Volts").unwrap().value_type,
            ValueType::Float
        );
        assert_eq!(
            parsed.table.get("Root.Bare").unwrap().value_type,
            ValueType::Unknown
        );
    }

    #[test]
    fn channel_unit_is_the_quantity_base_unit_not_the_display_unit() {
        // The authoritative unit is the *base unit* of the object's quantity
        // (`Props@Qty`, an SI unit string), NOT the `<Locale><Default Unit>`
        // display unit. Per the manual, base units are SI except Angle (degrees)
        // and Temperature (Celsius), so `rad/s` is stored as `deg/s` regardless
        // of a `rpm` display unit. A channel with no quantity is unitless.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Channel" Name="Root.Motor Speed">
    <Props Qty="rad/s" Security="Tune"><Locale><Default Unit="rpm"/></Locale></Props>
  </Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Coolant Temp">
    <Props Qty="K"><Locale><Default Unit="C"/></Locale></Props>
  </Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Battery">
    <Props Qty="V"/>
  </Component>
  <Component Classname="BuiltIn.Channel" Name="Root.NoQty"><Props Security="Tune"/></Component>
</Project>"#;
        let parsed = parse(xml).unwrap();
        let unit = |p: &str| parsed.table.get(p).unwrap().unit.clone();
        assert_eq!(unit("Root.Motor Speed").as_deref(), Some("deg/s"));
        assert_eq!(unit("Root.Coolant Temp").as_deref(), Some("C"));
        assert_eq!(unit("Root.Battery").as_deref(), Some("V"));
        assert_eq!(unit("Root.NoQty"), None);
        // The raw quantity and *display* unit are captured separately (for the
        // T095 display-unit check): `unit` is the base unit, `display_unit` is
        // verbatim from `<Locale><Default Unit>`, and `qty` is the raw `Props@Qty`.
        let sym = |p: &str| parsed.table.get(p).unwrap();
        assert_eq!(sym("Root.Motor Speed").qty.as_deref(), Some("rad/s"));
        assert_eq!(sym("Root.Motor Speed").display_unit.as_deref(), Some("rpm"));
        assert_eq!(sym("Root.Coolant Temp").qty.as_deref(), Some("K"));
        assert_eq!(sym("Root.Coolant Temp").display_unit.as_deref(), Some("C"));
        // A quantity with no explicit display unit carries `None` (shows in base).
        assert_eq!(sym("Root.Battery").qty.as_deref(), Some("V"));
        assert_eq!(sym("Root.Battery").display_unit, None);
        assert_eq!(sym("Root.NoQty").qty, None);
    }

    #[test]
    fn channel_security_comes_from_props() {
        // Security/access level lives on `<Props Security="…">` (#77); a channel
        // without it has none.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Channel" Name="Root.Tuned"><Props Qty="rad/s" Security="Master Calibration"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Open"><Props Qty="V"/></Component>
</Project>"#;
        let parsed = parse(xml).unwrap();
        assert_eq!(
            parsed.table.get("Root.Tuned").unwrap().security.as_deref(),
            Some("Master Calibration")
        );
        assert_eq!(parsed.table.get("Root.Open").unwrap().security, None);
    }

    #[test]
    fn function_gets_call_rate_from_selected_trigger() {
        // A function/method runs at the rate named by the EventKernel its
        // SelectedTrigger points at; one with no trigger has no rate (#76).
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.EventKernel" Name="Root.Events.On 100Hz"/>
  <Component Classname="BuiltIn.MethodUser" Name="Root.Engine.Control"><Props SelectedTrigger="Parent.Parent.Events.On 100Hz"/></Component>
  <Component Classname="BuiltIn.FuncUser" Name="Root.Engine.Init"><Props SelectedTrigger="Parent.Parent.Events.On Startup"/></Component>
  <Component Classname="BuiltIn.MethodUser" Name="Root.Engine.Untriggered"/>
</Project>"#;
        let parsed = parse(xml).unwrap();
        assert_eq!(
            parsed
                .table
                .get("Root.Engine.Control")
                .unwrap()
                .call_rate_hz,
            Some(100.0)
        );
        assert_eq!(
            parsed.table.get("Root.Engine.Init").unwrap().call_rate_hz,
            None
        );
        assert_eq!(
            parsed
                .table
                .get("Root.Engine.Untriggered")
                .unwrap()
                .call_rate_hz,
            None
        );
    }

    #[test]
    fn parses_object_with_class_and_members() {
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="MoTeC Input.Sensor" Name="Root.Throttle"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Throttle.Value"/>
  <Component Classname="BuiltIn.MethodUser" Name="Root.Throttle.Calculation" Filename="Calc.m1scr"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Throttle.Diagnostic.Value"/>
</Project>"#;
        let parsed = parse(xml).unwrap();
        let obj = parsed.table.get("Root.Throttle").expect("object symbol");
        assert_eq!(obj.kind, SymbolKind::Object);
        assert_eq!(obj.class.as_deref(), Some("MoTeC Input.Sensor"));
        // The raw classname is stored for every component (#47), not just objects:
        // the object's matches `class`, and a BuiltIn channel carries its own.
        assert_eq!(obj.classname.as_deref(), Some("MoTeC Input.Sensor"));
        let ch = parsed
            .table
            .get("Root.Throttle.Value")
            .expect("channel symbol");
        assert_eq!(ch.class, None, "non-object has no user-facing `class`");
        assert_eq!(ch.classname.as_deref(), Some("BuiltIn.Channel"));

        // Immediate members are Value and Calculation (not the nested Diagnostic.Value).
        let mut members: Vec<&str> = parsed
            .table
            .immediate_children("Root.Throttle")
            .iter()
            .map(|s| s.path.as_str())
            .collect();
        members.sort();
        assert_eq!(
            members,
            ["Root.Throttle.Calculation", "Root.Throttle.Value"]
        );
    }

    #[test]
    fn maps_scripts_to_groups_by_filename_convention() {
        // No `Filename=` attributes (the real-corpus shape): the file → group
        // map must still be derived, from the path-encoding convention, so that
        // group-relative references in scripts resolve.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Control"/>
  <Component Classname="BuiltIn.MethodUser" Name="Root.Control.Update"/>
  <Component Classname="BuiltIn.FuncUser" Name="Root.Control.Helper" Filename="custom.m1scr"/>
</Project>"#;
        let parsed = parse(xml).unwrap();
        // Convention-derived: drop `Root.`, append `.m1scr`; group is the parent.
        assert_eq!(
            parsed
                .file_to_group
                .get("Control.Update.m1scr")
                .map(String::as_str),
            Some("Root.Control")
        );
        // An explicit `Filename=` still wins over the convention.
        assert_eq!(
            parsed.file_to_group.get("custom.m1scr").map(String::as_str),
            Some("Root.Control")
        );
        assert!(
            !parsed.file_to_group.contains_key("Control.Helper.m1scr"),
            "the explicit Filename should be the key, not the derived one"
        );
    }

    #[test]
    fn untyped_channel_typed_from_owner_class() {
        // A channel with no inline Type/Qty inherits a type from its owning
        // object's class (#25): MoTeC Input.* sensors, `_IOMethod.ratio`/`abs`
        // and table outputs are FloatingPoint; a switched output is Boolean.
        // Genuinely indeterminate classes (CAN bus, references) stay Unknown.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="MoTeC Input.Temperature" Name="Root.Coolant"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Coolant.Value"/>
  <Component Classname="_IOMethod.ratio" Name="Root.Brake Bias"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Brake Bias.Output"/>
  <Component Classname="_IOMethod.abs" Name="Root.Yaw"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Yaw.Magnitude"/>
  <Component Classname="MoTeC Output.Switched Output" Name="Root.Fan"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Fan.State"/>
  <Component Classname="BuiltIn.Table" Name="Root.Tbl"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Tbl.Out"/>
</Project>"#;
        let parsed = parse(xml).unwrap();
        assert_eq!(
            parsed.table.get("Root.Yaw.Magnitude").unwrap().value_type,
            ValueType::Float,
            "_IOMethod.abs output should be Float"
        );
        assert_eq!(
            parsed.table.get("Root.Fan.State").unwrap().value_type,
            ValueType::Boolean,
            "Switched Output channel should be Boolean"
        );
        assert_eq!(
            parsed.table.get("Root.Coolant.Value").unwrap().value_type,
            ValueType::Float,
            "MoTeC Input.* sensor channel should be Float"
        );
        assert_eq!(
            parsed
                .table
                .get("Root.Brake Bias.Output")
                .unwrap()
                .value_type,
            ValueType::Float,
            "_IOMethod.ratio output should be Float"
        );
        assert_eq!(
            parsed.table.get("Root.Tbl.Out").unwrap().value_type,
            ValueType::Float,
            "table output channel should be Float (#25 case 1)"
        );
    }

    #[test]
    fn owner_class_walks_through_subchannels_to_the_owning_object() {
        // #103: a sub-channel chain (`.Value.Filtered`) under a typed object must
        // inherit the object's type, not stop at its immediate (channel) parent.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="MoTeC Input.Temperature" Name="Root.Coolant"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Coolant.Value"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Coolant.Value.Filtered"/>
</Project>"#;
        let parsed = parse(xml).unwrap();
        assert_eq!(
            parsed
                .table
                .get("Root.Coolant.Value.Filtered")
                .unwrap()
                .value_type,
            ValueType::Float,
            "a sub-channel of a MoTeC Input object's .Value should still be Float"
        );
    }

    #[test]
    fn owner_class_does_not_cross_a_group_boundary() {
        // A channel under a GroupCompound that is itself under a sensor must NOT
        // inherit the sensor's type — groups regroup heterogeneous children, so
        // crossing them would mis-type (e.g. a diagnostic status). Stays Unknown.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="MoTeC Input.Temperature" Name="Root.Coolant"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Coolant.Diagnostics"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Coolant.Diagnostics.Status"/>
</Project>"#;
        let parsed = parse(xml).unwrap();
        assert_eq!(
            parsed
                .table
                .get("Root.Coolant.Diagnostics.Status")
                .unwrap()
                .value_type,
            ValueType::Unknown,
            "must not inherit a sensor's type across a GroupCompound"
        );
    }

    #[test]
    fn leaf_name_fallback_types_physical_quantities_as_float() {
        // #103: when class-based inference yields Unknown, a leaf naming an
        // unambiguous physical quantity (Voltage/Current/…) is a measured Float.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Acc"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Acc.Battery Voltage"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Acc.Drive Mode"/>
</Project>"#;
        let parsed = parse(xml).unwrap();
        assert_eq!(
            parsed
                .table
                .get("Root.Acc.Battery Voltage")
                .unwrap()
                .value_type,
            ValueType::Float,
            "a `… Voltage` channel should be inferred Float"
        );
        assert_eq!(
            parsed.table.get("Root.Acc.Drive Mode").unwrap().value_type,
            ValueType::Unknown,
            "`… Mode` is not a physical quantity — stays Unknown (no false Float)"
        );
    }

    #[test]
    fn reference_captures_target_verbatim_else_none() {
        // A `BuiltIn.Reference` aliases the path in its `<Props Target="…">`;
        // capture it verbatim (m1-doc#29 cross-references) — no resolution.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Reference" Name="Root.X.Ref"><Props Target="This.Value"/></Component>
  <Component Classname="BuiltIn.Reference" Name="Root.X.NoTarget"><Props/></Component>
  <Component Classname="BuiltIn.Reference" Name="Root.X.NoProps"/>
  <Component Classname="BuiltIn.Channel" Name="Root.X.Chan"><Props Type="u32" Target="This.Value"/></Component>
</Project>"#;
        let parsed = parse(xml).unwrap();
        // A reference with a Target captures it verbatim.
        assert_eq!(
            parsed.table.get("Root.X.Ref").unwrap().reference_target,
            Some("This.Value".to_string())
        );
        // A reference whose `<Props>` declares no Target -> None.
        assert_eq!(
            parsed
                .table
                .get("Root.X.NoTarget")
                .unwrap()
                .reference_target,
            None
        );
        // A reference with no `<Props>` at all -> None.
        assert_eq!(
            parsed.table.get("Root.X.NoProps").unwrap().reference_target,
            None
        );
        // A non-reference component never captures a Target, even if present.
        assert_eq!(
            parsed.table.get("Root.X.Chan").unwrap().reference_target,
            None
        );
    }
}
