//! Parse Project.m1prj into a SymbolTable, enum types, and a file->group map.
use super::{EnumId, EnumType, Symbol, SymbolKind, SymbolTable};
use crate::types::{ValueType, primitive_type};
use std::collections::{HashMap, HashSet};

pub struct Parsed {
    pub table: SymbolTable,
    pub file_to_group: HashMap<String, String>,
    pub module_roots: HashSet<String>,
}

#[derive(Debug)]
pub enum ParseError {
    Xml(roxmltree::Error),
}
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Xml(e) => write!(f, "invalid .m1prj XML: {e}"),
        }
    }
}
impl std::error::Error for ParseError {}

pub fn classify(classname: &str) -> SymbolKind {
    use SymbolKind::*;
    if classname.starts_with("BuiltIn.Channel") || classname == "BuiltIn.CalibrationChannel" {
        Channel
    } else if classname.starts_with("BuiltIn.Parameter")
        || classname == "BuiltIn.CalibrationParameter"
        || classname == "BuiltIn.IOResourceParameter"
    {
        Parameter
    } else if classname.starts_with("BuiltIn.Constant") || classname == "BuiltIn.IOResourceConstant"
    {
        Constant
    } else if classname.starts_with("BuiltIn.FuncUser")
        || classname.starts_with("BuiltIn.CalFuncUser")
        || classname.starts_with("BuiltIn.FuncGenerated")
    {
        Function
    } else if classname == "BuiltIn.MethodUser" || classname == "BuiltIn.EventKernel" {
        Method
    } else if classname.starts_with("BuiltIn.Table") || classname == "BuiltIn.CalibrationTable" {
        Table
    } else if classname == "BuiltIn.GroupCompound" {
        Group
    } else if classname == "BuiltIn.Reference" {
        Reference
    } else if !classname.starts_with("BuiltIn.") && !classname.is_empty() {
        // Any non-`BuiltIn.*` component is an instance of a package-defined
        // object class (e.g. "MoTeC Input.Sensor", "MoTeC Output.Switched
        // Output", "_IOMethod.abs"). Its members are separate components whose
        // path is prefixed by this object's path.
        Object
    } else {
        Other
    }
}

/// Enclosing group path for a fully-qualified symbol path (drop the last segment,
/// honouring space-containing segments — segments are split on '.').
fn parent_group(path: &str) -> Option<String> {
    let idx = path.rfind('.')?;
    Some(path[..idx].to_string())
}

/// Parse a script's call rate (Hz) from its `<Props SelectedTrigger="…">`. The
/// trigger is a (parent-relative) path to a `BuiltIn.EventKernel` whose leaf
/// name encodes the clock, e.g. `Parent.Parent.Events.On 500Hz` → `500`. Returns
/// `None` for `On Startup` (startup-only functions), `$(…)` parameter-reference
/// triggers that aren't statically resolvable, and any leaf not of the form
/// `On <N>Hz`. The rate lives entirely in the kernel leaf, so the relative path
/// prefix need not be resolved to read it.
fn event_rate_hz(trigger: &str) -> Option<f64> {
    if trigger.contains("$(") {
        return None;
    }
    let leaf = trigger.rsplit('.').next().unwrap_or(trigger);
    let n = leaf.strip_prefix("On ")?.strip_suffix("Hz")?;
    n.trim().parse::<f64>().ok()
}

/// Derive an untyped channel's value type from the class of the object that
/// owns it (its parent in the component tree). Each mapping is a known output
/// type for that package class; classes whose output is genuinely indeterminate
/// (`MoTeC Comms.CAN Bus`, `BuiltIn.Reference`, the odd built-ins) stay
/// `Unknown` — no worse than before.
/// The known output [`ValueType`] for a package class, or `None` when the class
/// doesn't determine one (a group, a CAN/comms container, an unmapped class) —
/// in which case the ancestor walk stops and falls back to the leaf name.
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
fn type_from_leaf_name(path: &str) -> ValueType {
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
fn type_from_owner_class(path: &str, classname_by_path: &HashMap<String, String>) -> ValueType {
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

/// Byte offsets of every line start in `xml` (offset 0, then each byte after a
/// `\n`), kept sorted. `line_at` binary-searches this to map a byte offset to its
/// 0-based source line in O(log N), so per-component line lookup is amortized O(1)
/// over the whole parse — replacing roxmltree's `text_pos_at`, which rescans from
/// byte 0 on every call and is documented "very expensive, use only for errors"
/// (its per-component use made parsing O(N^2), #95).
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(xml: &str) -> Self {
        let mut starts = vec![0usize];
        starts.extend(
            xml.bytes()
                .enumerate()
                .filter(|&(_, b)| b == b'\n')
                .map(|(i, _)| i + 1),
        );
        LineIndex { starts }
    }

    /// 0-based source line containing byte offset `pos`.
    fn line_at(&self, pos: usize) -> usize {
        // partition_point yields the count of line-starts <= pos; subtract one for
        // the 0-based line number (there is always the offset-0 entry, so it's >=1).
        self.starts.partition_point(|&s| s <= pos) - 1
    }
}

pub fn parse(xml: &str) -> Result<Parsed, ParseError> {
    let doc = roxmltree::Document::parse(xml).map_err(ParseError::Xml)?;
    let line_index = LineIndex::new(xml);
    let mut table = SymbolTable::default();
    let mut file_to_group = HashMap::new();
    let mut module_roots = HashSet::new();

    // Pre-pass: map every component's path -> its Classname, so a channel that
    // carries no inline `<Props Type>`/`Qty` can be typed from the class of the
    // object that owns it (its parent). M1 Build derives these the same way —
    // the object's class is the type source (#25).
    let classname_by_path: HashMap<String, String> = doc
        .descendants()
        .filter(|n| n.has_tag_name("Component"))
        .filter_map(|n| {
            Some((
                n.attribute("Name")?.to_string(),
                n.attribute("Classname")?.to_string(),
            ))
        })
        .collect();

    for node in doc.descendants() {
        match node.tag_name().name() {
            "File" if node.parent().map(|p| p.tag_name().name()) == Some("SelectedModuleSets") => {
                if let Some(name) = node.attribute("Name") {
                    module_roots.insert(name.to_string());
                }
            }
            "Type" if node.attribute("Storage") == Some("enum") => {
                let name = node.attribute("Name").unwrap_or("").to_string();
                let default = node.attribute("Default").map(str::to_string);
                let members = node
                    .children()
                    .filter(|c| c.has_tag_name("Enum"))
                    .map(|c| {
                        (
                            c.attribute("Name").unwrap_or("").to_string(),
                            c.attribute("ContainerOrder")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0),
                        )
                    })
                    .collect();
                table.add_enum(EnumType {
                    name,
                    members,
                    default,
                });
            }
            "Component" => {
                let (Some(classname), Some(name)) =
                    (node.attribute("Classname"), node.attribute("Name"))
                else {
                    continue;
                };
                let kind = classify(classname);
                let filename = node.attribute("Filename").map(str::to_string);
                if matches!(kind, SymbolKind::Function | SymbolKind::Method)
                    && let Some(g) = parent_group(name)
                {
                    // Map the script to its enclosing group, anchoring
                    // group-relative resolution in scripts. Prefer an explicit
                    // `Filename`; otherwise derive the conventional basename
                    // (`Root.Engine.Update` -> `Engine.Update.m1scr`), since real
                    // projects omit the attribute and name script files by this
                    // path convention. (`filename` on the symbol stays
                    // attribute-only — it locates the backing file, and the
                    // basename can't carry a subdirectory.)
                    let key = filename.clone().unwrap_or_else(|| {
                        format!("{}.m1scr", name.strip_prefix("Root.").unwrap_or(name))
                    });
                    file_to_group.insert(key, g);
                }
                let props = node.children().find(|c| c.has_tag_name("Props"));
                // Value type: Constants from their @Value literal; everything
                // else from the declared @Type (primitive or `::This.<enum>`).
                let (value_type, enum_assoc) = if kind == SymbolKind::Constant {
                    let vt = props
                        .and_then(|p| p.attribute("Value"))
                        .map(constant_value_type)
                        .unwrap_or(ValueType::Unknown);
                    (vt, None)
                } else if let Some(t) = props.and_then(|p| p.attribute("Type")) {
                    // Explicit storage type: a primitive (`u32`, `s16`, …) or a
                    // `::This.<enum>` reference.
                    resolve_props_type(t, &table)
                } else if props.and_then(|p| p.attribute("Qty")).is_some() {
                    // No explicit storage type but a physical quantity (`Qty`,
                    // e.g. `rad/s`, `V`, `K`): a measured channel, stored as Float.
                    (ValueType::Float, None)
                } else if kind == SymbolKind::Channel {
                    // Auto-created output channels carry no inline type/quantity;
                    // derive it from the owning object's class (#25).
                    (type_from_owner_class(name, &classname_by_path), None)
                } else {
                    (ValueType::Unknown, None)
                };
                let class = if kind == SymbolKind::Object {
                    Some(classname.to_string())
                } else {
                    None
                };
                // Authoritative unit: the *base unit* of the object's quantity.
                // `<Props Qty="…">` records the quantity as an SI unit string
                // (`rad/s`, `K`, `Pa`, …); the stored/base unit is that with the
                // manual's two exceptions applied (Angle→degrees, Temperature→
                // Celsius). The `<Locale><Default Unit>` is only the *display*
                // unit (`rpm`, `kPa`) and is deliberately NOT used here. A `$(…)`
                // template `Qty` is an inherited reference, not a literal unit.
                let unit = props
                    .and_then(|p| p.attribute("Qty"))
                    .and_then(crate::units::base_unit_of_qty);
                // Security / access level (#77): `<Props Security="Tune">`.
                let security = props
                    .and_then(|p| p.attribute("Security"))
                    .map(str::to_string);
                // Call rate (#76): a script/function runs on the clock named by
                // its `SelectedTrigger`, a `BuiltIn.EventKernel` like `On 100Hz`.
                let call_rate_hz = props
                    .and_then(|p| p.attribute("SelectedTrigger"))
                    .and_then(event_rate_hz);
                let def_line = Some(line_index.line_at(node.range().start) as u32);
                table.push(Symbol {
                    path: name.to_string(),
                    kind,
                    value_type,
                    declared_type: props
                        .and_then(|p| p.attribute("Type"))
                        .filter(|t| !t.is_empty())
                        .map(str::to_string),
                    unit,
                    security,
                    filename,
                    enum_assoc,
                    class,
                    classname: Some(classname.to_string()),
                    def_line,
                    dbc_range: None,
                    can: None,
                    call_rate_hz,
                    table_meta: None,
                });
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

/// Resolve a component's declared `<Props Type="…">` to a value type (and the
/// enum id when it names a project enum). Recognised forms:
///
///   * primitive — `u32`, `s16`, `f32`, `bool`, …
///   * `::This.<EnumName>` — an enum defined in this project's `<DataTypes>`
///
/// Anything else (cross-module enums like `MoTeC Types.…`, derived `$(…)`
/// expressions, an unknown enum name) is left `Unknown` rather than guessed.
/// Enums are registered earlier in the document than the components that use
/// them, so they are already present in `table` when this is called.
fn resolve_props_type(type_attr: &str, table: &SymbolTable) -> (ValueType, Option<EnumId>) {
    if let Some(vt) = primitive_type(type_attr) {
        return (vt, None);
    }
    if let Some(name) = type_attr.strip_prefix("::This.")
        && let Some(id) = table.enum_by_name(name)
    {
        return (ValueType::Enum(id), Some(id));
    }
    (ValueType::Unknown, None)
}

fn constant_value_type(value: &str) -> ValueType {
    let v = value.trim();
    if v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("false") {
        ValueType::Boolean
    } else if v.parse::<i64>().is_ok() {
        ValueType::Integer
    } else if v.parse::<f64>().is_ok() {
        ValueType::Float
    } else {
        ValueType::Unknown // e.g. an enum member name like "CAN Bus 2"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_package_objects_and_keeps_builtins() {
        assert_eq!(classify("MoTeC Input.Sensor"), SymbolKind::Object);
        assert_eq!(classify("MoTeC Output.Switched Output"), SymbolKind::Object);
        assert_eq!(classify("_IOMethod.abs"), SymbolKind::Object);
        // BuiltIn.* primitives are unchanged.
        assert_eq!(classify("BuiltIn.Channel"), SymbolKind::Channel);
        assert_eq!(classify("BuiltIn.MethodUser"), SymbolKind::Method);
        // An unhandled BuiltIn.* stays Other, not Object.
        assert_eq!(classify("BuiltIn.IOCharacteristic"), SymbolKind::Other);
    }

    #[test]
    fn symbols_record_their_m1prj_definition_line() {
        // 0-based: line 0 = <?xml>, 1 = <Project>, 2 = Root.A, 3 = Root.B.
        let xml = "<?xml version=\"1.0\"?>\n<Project>\n  <Component Classname=\"BuiltIn.Channel\" Name=\"Root.A\"><Props Type=\"u32\"/></Component>\n  <Component Classname=\"BuiltIn.Channel\" Name=\"Root.B\"><Props Type=\"s32\"/></Component>\n</Project>";
        let parsed = parse(xml).unwrap();
        assert_eq!(parsed.table.get("Root.A").unwrap().def_line, Some(2));
        assert_eq!(parsed.table.get("Root.B").unwrap().def_line, Some(3));
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
        // Cross-module enum (not defined in this file) and derived types are
        // left Unknown rather than guessed.
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Channel" Name="Root.M"><Props Type="MoTeC Types.Mode Enumeration"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.N"><Props Type="$(Parent.Value:Type)"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.O"/>
</Project>"#;
        let parsed = parse(xml).unwrap();
        assert_eq!(
            parsed.table.get("Root.M").unwrap().value_type,
            ValueType::Unknown
        );
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
    fn event_rate_hz_parses_kernel_leaf() {
        assert_eq!(event_rate_hz("Parent.Parent.Events.On 500Hz"), Some(500.0));
        assert_eq!(event_rate_hz("Root.Events.On 10Hz"), Some(10.0));
        // Startup-only functions have no rate.
        assert_eq!(event_rate_hz("Parent.Events.On Startup"), None);
        // Parameter-reference triggers aren't statically resolvable.
        assert_eq!(event_rate_hz("$(Parent.Calc:SelectedTrigger)"), None);
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
}
