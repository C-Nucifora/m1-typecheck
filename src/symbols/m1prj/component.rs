//! Enum registration and per-`<Component>` symbol construction for the `.m1prj`
//! parse pass: derive each component's value type, unit, security, rates and
//! tags from its `<Props>`, and build the [`Symbol`].

use super::index::ProjectXmlIndex;
use super::parent_group;
use super::type_inference::{type_from_leaf_name, type_from_owner_class};
use crate::symbols::{EnumId, EnumType, Symbol, SymbolKind, SymbolTable, TableAxis, TableMeta};
use crate::types::{ValueType, primitive_type};
use m1_workspace::LineIndex;
use std::collections::HashMap;

/// Register every enum type a `.m1prj` defines into `table`, in two passes:
///
///   * firmware-supplied enums (`MoTeC Types.<N>` / `::Hardware.<N>`) declared
///     on any channel's `<Props Type>`. Their member lists live in the firmware,
///     not the project, so they are modelled "open" — channels typed by them
///     resolve to `Enum(id)` (enabling T021 and enum hover) while T020
///     enum-non-member is suppressed (a name we don't have can't be proven a
///     non-member).
///   * project-local enums (`<Type Storage="enum">` in `<DataTypes>`), whose
///     member list *is* present and therefore authoritative (`open: false`).
///
/// Both run before the component pass so the component pass can resolve these
/// names regardless of document order.
pub(super) fn parse_enums(doc: &roxmltree::Document, table: &mut SymbolTable) {
    // Firmware-supplied enums declared on a channel's `<Props Type>`.
    for node in doc.descendants() {
        if !node.has_tag_name("Component") {
            continue;
        }
        let Some(name) = node
            .children()
            .find(|c| c.has_tag_name("Props"))
            .and_then(|p| p.attribute("Type"))
            .and_then(firmware_enum_name)
        else {
            continue;
        };
        if table.enum_by_name(name).is_none() {
            table.add_enum(EnumType {
                name: name.to_string(),
                members: Vec::new(),
                default: None,
                open: true,
            });
        }
    }

    // Project-local `<Type Storage="enum">` declarations.
    for node in doc.descendants() {
        if !(node.has_tag_name("Type") && node.attribute("Storage") == Some("enum")) {
            continue;
        }
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
            open: false, // project-local enum: member list is authoritative
        });
    }
}

/// The per-component fields derived from a `<Component>` node and its `<Props>`.
struct ComponentProps {
    value_type: ValueType,
    enum_assoc: Option<EnumId>,
    declared_type: Option<String>,
    unit: Option<String>,
    security: Option<String>,
    call_rate_hz: Option<f64>,
    log_rate_hz: Option<f64>,
    class: Option<String>,
    table_meta: Option<TableMeta>,
    tags: Vec<String>,
    /// Declared return type from `<Signature ReturnType="…">` (#110).
    return_type: Option<ValueType>,
    /// Declared parameters from `<Signature><Params>` (#110).
    in_params: Option<Vec<(String, ValueType)>>,
}

/// Extract the derived fields for one component from its node, `<Props>`, the
/// pre-pass index and the (already-populated) enum table.
fn component_props(
    node: roxmltree::Node,
    name: &str,
    classname: &str,
    kind: SymbolKind,
    index: &ProjectXmlIndex,
    table: &SymbolTable,
) -> ComponentProps {
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
        resolve_props_type(t, table)
    } else if props.and_then(|p| p.attribute("Qty")).is_some() {
        // A physical quantity (`Qty`, e.g. `rad/s`, `V`, `K`, `Pa`) with
        // no explicit storage type: a measured channel, stored as Float.
        // This is *exact*, not a guess — the manual states an integer,
        // unsigned-integer or enumeration channel can only carry the
        // unitless quantity, so a channel with a real physical quantity
        // is necessarily Float. Conversely an integer-stored channel is
        // unitless and its storage type is *not* recoverable from the
        // `.m1prj` alone — it is typed from the `.m1cfg` `<Cell Type>`
        // by `m1cfg::augment` (which overwrites this fallback). So
        // T030/T003 on integer channels requires a `.m1cfg`; we do not
        // guess an integral type from a unitless `Qty`, which would
        // false-positive on unitless floats (ratios) (#107).
        (ValueType::Float, None)
    } else if kind == SymbolKind::Channel {
        // Auto-created output channels carry no inline type/quantity;
        // derive it from the owning object's class (#25), which itself
        // falls back to the leaf-name heuristic.
        (type_from_owner_class(name, &index.classname_by_path), None)
    } else {
        // Parameters (and any other typeless symbol) with no Type/Qty
        // and no .m1cfg coverage: a conservative leaf-name heuristic —
        // a leaf ending in a float-quantity word (Voltage/Temperature/
        // Frequency/…) is Float by M1 convention, recovering T030/T003
        // that would otherwise be silenced (#106).
        (type_from_leaf_name(name), None)
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
    // Default log rate (#171): `<Props DefaultLogRate="5MS">` — a
    // logging *period*, surfaced as a rate in Hz for hover.
    let log_rate_hz = props
        .and_then(|p| p.attribute("DefaultLogRate"))
        .and_then(self::log_rate_hz);
    let declared_type = props
        .and_then(|p| p.attribute("Type"))
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    // Declared function signature (#110): `<Signature ReturnType="bool">` with
    // `<Params><Param Name="Timeout" Type="f32">…`. Present on user functions
    // and the auto-created FuncGenerated components in real projects.
    let signature = node.children().find(|c| c.has_tag_name("Signature"));
    let return_type = signature
        .and_then(|sig| sig.attribute("ReturnType"))
        .filter(|t| !t.is_empty())
        .and_then(crate::types::primitive_type);
    let in_params = signature.map(|sig| {
        sig.children()
            .find(|c| c.has_tag_name("Params"))
            .map(|params| {
                params
                    .children()
                    .filter(|c| c.has_tag_name("Param"))
                    .filter_map(|p| {
                        let name = p.attribute("Name")?;
                        let ty = p
                            .attribute("Type")
                            .and_then(crate::types::primitive_type)
                            .unwrap_or(ValueType::Unknown);
                        Some((name.to_string(), ty))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    ComponentProps {
        value_type,
        enum_assoc,
        declared_type,
        unit,
        security,
        call_rate_hz,
        log_rate_hz,
        class,
        return_type,
        in_params,
        // A Table's shape is encoded in the .m1prj `<Axis>` children
        // (`<X MaxSites/>`, `<Y …/>`, `<Z …/>`); surface it so hover
        // shows the shape even before a `.m1cfg` exists. When a cfg is
        // present, its richer per-axis units override this in augment().
        table_meta: if kind == SymbolKind::Table {
            table_meta_from_axis(node)
        } else {
            None
        },
        tags: effective_tags(name, &index.selected_tags_by_path),
    }
}

/// Build a [`Symbol`] for a `<Component>` node, returning `None` when the node
/// lacks the required `Classname`/`Name` attributes.
///
/// As a side effect, scripts (Function/Method kinds) are mapped to their
/// enclosing group in `file_to_group`, anchoring group-relative resolution.
pub(super) fn symbol_from_component(
    node: roxmltree::Node,
    index: &ProjectXmlIndex,
    table: &SymbolTable,
    line_index: &LineIndex,
    file_to_group: &mut HashMap<String, String>,
) -> Option<Symbol> {
    let (classname, name) = (node.attribute("Classname")?, node.attribute("Name")?);
    let kind = SymbolKind::from_classname(classname);
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
        let key = filename
            .clone()
            .unwrap_or_else(|| format!("{}.m1scr", name.strip_prefix("Root.").unwrap_or(name)));
        file_to_group.insert(key, g);
    }
    let props = component_props(node, name, classname, kind, index, table);
    let def_line = Some(line_index.line_at(node.range().start) as u32);
    Some(Symbol {
        path: name.to_string(),
        kind,
        value_type: props.value_type,
        declared_type: props.declared_type,
        unit: props.unit,
        security: props.security,
        filename,
        enum_assoc: props.enum_assoc,
        class: props.class,
        classname: Some(classname.to_string()),
        def_line,
        dbc_range: None,
        can: None,
        call_rate_hz: props.call_rate_hz,
        log_rate_hz: props.log_rate_hz,
        tags: props.tags,
        return_type: props.return_type,
        in_params: props.in_params,
        table_meta: props.table_meta,
    })
}

/// A `BuiltIn.Table`'s shape from its `.m1prj` `<Axis>` element: each of the
/// `<X>` / `<Y>` / `<Z>` children contributes one axis whose breakpoint count is
/// its `MaxSites`. Returns `None` when there is no `<Axis>` or no sized axis.
/// The `.m1cfg` `augment()` pass overrides this with richer per-axis units when a
/// calibration export is present (#165).
fn table_meta_from_axis(node: roxmltree::Node) -> Option<TableMeta> {
    let axis = node.children().find(|c| c.has_tag_name("Axis"))?;
    let axes: Vec<TableAxis> = ["X", "Y", "Z"]
        .iter()
        .filter_map(|dim| {
            axis.children()
                .find(|c| c.has_tag_name(*dim))?
                .attribute("MaxSites")?
                .parse::<u32>()
                .ok()
                .map(|size| TableAxis { size, unit: None })
        })
        .collect();
    (!axes.is_empty()).then_some(TableMeta {
        axes,
        output_unit: None,
    })
}

/// Effective tag set for `path`: its own `SelectedTags` (declared order) unioned
/// with every ancestor group's tags (#170). The manual states a group node's tags
/// are inherited by its children, so we walk the parent chain and append each
/// ancestor's tags not already present. `own_tags` maps a component path to the
/// space-split tags it declared directly.
fn effective_tags(path: &str, own_tags: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut tags: Vec<String> = own_tags.get(path).cloned().unwrap_or_default();
    let mut cur = parent_group(path);
    while let Some(p) = cur {
        if let Some(inherited) = own_tags.get(&p) {
            for t in inherited {
                if !tags.contains(t) {
                    tags.push(t.clone());
                }
            }
        }
        cur = parent_group(&p);
    }
    tags
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

/// Parse a `.m1prj` `DefaultLogRate` value into a rate in Hz. M1 Build writes the
/// channel's default logging *period* as `<number><unit>` — `"5MS"` (5 ms → 200 Hz),
/// `"10MS"` (100 Hz), `"1S"` (1 Hz), `"200US"` — or, defensively, a rate `"<n>HZ"`.
/// Returns `None` for an unrecognised unit or a non-positive value.
fn log_rate_hz(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    let split = raw.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = raw.split_at(split);
    let n: f64 = num.trim().parse().ok()?;
    if n <= 0.0 {
        return None;
    }
    match unit.trim().to_ascii_uppercase().as_str() {
        "MS" => Some(1000.0 / n),      // period in milliseconds
        "US" => Some(1_000_000.0 / n), // period in microseconds
        "S" => Some(1.0 / n),          // period in seconds
        "HZ" => Some(n),               // already a rate
        _ => None,
    }
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
    // Firmware-supplied enum (`MoTeC Types.<N>` / `::Hardware.<N>`): registered
    // open in the pre-pass, so the lookup resolves to its `Enum(id)`.
    if let Some(name) = firmware_enum_name(type_attr)
        && let Some(id) = table.enum_by_name(name)
    {
        return (ValueType::Enum(id), Some(id));
    }
    (ValueType::Unknown, None)
}

/// The firmware enum name a `<Props Type>` string names, if it is one: a
/// `MoTeC Types.<N>` or `::Hardware.<N>` reference resolves to firmware enum
/// `<N>`. Returns `None` for primitives, `::This.` refs and `$(…)` templates.
fn firmware_enum_name(type_attr: &str) -> Option<&str> {
    type_attr
        .strip_prefix("MoTeC Types.")
        .or_else(|| type_attr.strip_prefix("::Hardware."))
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
    fn log_rate_hz_parses_period_units() {
        assert_eq!(log_rate_hz("5MS"), Some(200.0));
        assert_eq!(log_rate_hz("10MS"), Some(100.0));
        assert_eq!(log_rate_hz("1S"), Some(1.0));
        assert_eq!(log_rate_hz("200US"), Some(5000.0));
        assert_eq!(log_rate_hz("50HZ"), Some(50.0));
        // Unrecognised unit, zero, or junk -> None (no guessed rate).
        assert_eq!(log_rate_hz("5PS"), None);
        assert_eq!(log_rate_hz("0MS"), None);
        assert_eq!(log_rate_hz(""), None);
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
}
