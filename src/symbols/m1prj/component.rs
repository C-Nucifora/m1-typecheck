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

/// Register every enum type a `.m1prj` can reference into `table`, in three
/// passes:
///
///   * project-local enums (`<Type Storage="enum">` in `<DataTypes>`), whose
///     member list *is* present and therefore authoritative (`open: false`);
///   * the builtin firmware/module enumeration catalogue from the intrinsics
///     (M1 Build help captures) — every documented MoTeC enum, closed under
///     its script-facing display name, so literals like
///     `Output Drive Enumeration.High Side` resolve and the membership checks
///     M1 Build enforces (Errors 1306/1329/1352) fire here too;
///   * firmware enum types declared on a channel's `<Props Type>` (or implied
///     by a package class) that the catalogue does NOT document: registered as
///     open placeholders — channels typed by them resolve to `Enum(id)`
///     (enabling T021 and enum hover) while T020/T070 stay suppressed (a name
///     we don't have can't be proven a non-member).
///
/// Project-local declarations run first so a project's own `<Type>` of the same
/// name wins. All run before the component pass so the component pass can
/// resolve these names regardless of document order.
pub(super) fn parse_enums(doc: &roxmltree::Document, table: &mut SymbolTable) {
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

    // Builtin firmware/module enumerations (the intrinsics help-capture
    // catalogue): registered closed under their script-facing display names,
    // AFTER project-local declarations so a project's own redefinition wins.
    // Members deliberately bypass the member index — see `add_builtin_enum`.
    for def in &crate::intrinsics::get().enums {
        if table.enum_by_name(&def.name).is_none() {
            table.add_builtin_enum(EnumType {
                name: def.name.clone(),
                members: def
                    .members
                    .iter()
                    .map(|m| (m.name.clone(), m.value))
                    .collect(),
                default: None,
                open: false, // capture membership is authoritative
            });
        }
    }

    // Firmware-supplied enums declared on a channel's `<Props Type>`, or
    // carried by a package object's class (`_IOMethod.av_switch` → its value
    // is the switch state, #173). Documented hardware types already resolved
    // above via their display name; anything else registers as an open
    // placeholder (membership unknown, T020/T070 suppressed).
    for node in doc.descendants() {
        if !node.has_tag_name("Component") {
            continue;
        }
        let Some(name) = node
            .children()
            .find(|c| c.has_tag_name("Props"))
            .and_then(|p| p.attribute("Type"))
            .and_then(firmware_enum_name)
            .or_else(|| node.attribute("Classname").and_then(class_state_enum))
        else {
            continue;
        };
        let registered = hardware_display_name(name).unwrap_or(name);
        if table.enum_by_name(registered).is_none() {
            table.add_enum(EnumType {
                name: registered.to_string(),
                members: Vec::new(),
                default: None,
                open: true,
            });
        }
    }
}

/// The script-facing display name of a documented hardware (`::Hardware.*`)
/// enum type, keyed by the type path's leaf segment (the part after the
/// package, e.g. `sw_state` in `::Hardware.av_switch.sw_state`).
///
/// M1 Build resolves these from the firmware system file (which is signed and
/// not parseable); the membership itself comes from the intrinsics builtin
/// catalogue under the display name, so this map only bridges the hardware
/// path spelling to that catalogue entry.
fn hardware_display_name(name: &str) -> Option<&'static str> {
    match name.rsplit('.').next().unwrap_or(name) {
        // Switch input state — Off / On (probe-verified, #167/#168).
        "sw_state" => Some("Universal Switch State"),
        _ => None,
    }
}

/// The documented hardware enum a package class's *value* carries, when known:
/// an `_IOMethod.av_switch` switch object — read directly (`Driver.ASMS eq …`)
/// or via its auto-created `State` sub-channel — is the switch state, which
/// M1 Build types as "Universal Switch State" and enforces comparisons on
/// (Error 1329, #173). Returns the [`hardware_display_name`] key.
fn class_state_enum(class: &str) -> Option<&'static str> {
    match class {
        "_IOMethod.av_switch" => Some("sw_state"),
        _ => None,
    }
}

/// The registered enum id of `class`'s documented state enum, if any (the
/// pre-pass registers it whenever such a component exists).
fn state_enum_id(class: &str, table: &SymbolTable) -> Option<EnumId> {
    let key = class_state_enum(class)?;
    let display = hardware_display_name(key)?;
    table.enum_by_name(display)
}

/// [`state_enum_id`] of the package object that owns `path` (for its
/// auto-created `State` sub-channel).
fn owner_state_enum_id(path: &str, index: &ProjectXmlIndex, table: &SymbolTable) -> Option<EnumId> {
    let owner = parent_group(path)?;
    let class = index.classname_by_path.get(&owner)?;
    state_enum_id(class, table)
}

/// The per-component fields derived from a `<Component>` node and its `<Props>`.
struct ComponentProps {
    value_type: ValueType,
    enum_assoc: Option<EnumId>,
    declared_type: Option<String>,
    unit: Option<String>,
    /// Raw `<Props Qty>` (the quantity key for the display-unit check).
    qty: Option<String>,
    /// Raw `<Props><Locale><Default Unit>` (the display unit to validate).
    display_unit: Option<String>,
    security: Option<String>,
    call_rate_hz: Option<f64>,
    /// True when `<Props SelectedTrigger="…">` is present and non-empty — the
    /// function is bound to a schedule event (any clock, `On Startup`, or a
    /// `$(…)` trigger). Broader than `call_rate_hz`; powers the T104 root set.
    scheduled: bool,
    log_rate_hz: Option<f64>,
    class: Option<String>,
    table_meta: Option<TableMeta>,
    tags: Vec<String>,
    /// Declared return type from `<Signature ReturnType="…">` (#110).
    return_type: Option<ValueType>,
    /// Declared parameters from `<Signature><Params>` (#110).
    in_params: Option<Vec<(String, ValueType)>>,
    /// Raw `<Props Target="…">` of a `BuiltIn.Reference` component, verbatim;
    /// `None` for non-references or references without a `Target`.
    reference_target: Option<String>,
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
    } else if kind == SymbolKind::Object
        && let Some(id) = state_enum_id(classname, table)
    {
        // A package object whose value is a documented state enum, read
        // directly in scripts (`Driver.ASMS eq Universal Switch State.Off`):
        // typed as that enum so T021/T030 fire as M1 Build does (#173).
        (ValueType::Enum(id), Some(id))
    } else if kind == SymbolKind::Channel
        && name.rsplit('.').next() == Some("State")
        && let Some(id) = owner_state_enum_id(name, index, table)
    {
        // The auto-created `State` sub-channel of such an object carries the
        // same enum (its other children — Voltage, Diagnostic — do not).
        (ValueType::Enum(id), Some(id))
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
    // Raw quantity + display unit for the T095 display-unit check (Error 1017).
    // `Qty` is the quantity key (matches a `<Quantity Symbol>` in MoTeC's units
    // database); skip `$(…)` template references (inherited, not a literal). The
    // display unit lives on the nested `<Locale><Default Unit="…">`.
    let qty = props
        .and_then(|p| p.attribute("Qty"))
        .map(str::trim)
        .filter(|q| !q.is_empty() && !q.contains("$("))
        .map(str::to_string);
    let display_unit = props
        .and_then(|p| p.children().find(|c| c.has_tag_name("Locale")))
        .and_then(|loc| loc.children().find(|c| c.has_tag_name("Default")))
        .and_then(|d| d.attribute("Unit"))
        .map(str::trim)
        .filter(|u| !u.is_empty() && !u.contains("$("))
        .map(str::to_string);
    // Security / access level (#77): `<Props Security="Tune">`.
    let security = props
        .and_then(|p| p.attribute("Security"))
        .map(str::to_string);
    // Call rate (#76): a script/function runs on the clock named by
    // its `SelectedTrigger`, a `BuiltIn.EventKernel` like `On 100Hz`.
    let call_rate_hz = props
        .and_then(|p| p.attribute("SelectedTrigger"))
        .and_then(event_rate_hz);
    // Bound to *any* schedule event (clock, `On Startup`, or a `$(…)` trigger),
    // regardless of whether the rate is statically resolvable — the entry-point
    // predicate for the T104 reachability check (M1 Build Error 1642).
    let scheduled = props
        .and_then(|p| p.attribute("SelectedTrigger"))
        .is_some_and(|t| !t.trim().is_empty());
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
    // Reference target (m1-doc#29 cross-references): a `BuiltIn.Reference`
    // aliases the symbol/path named by its `<Props Target="…">`. Captured raw
    // and verbatim for the reference kind only — never resolved or validated
    // here; downstream tools document what it points at. `None` for other kinds
    // and for references that declare no `Target`.
    let reference_target = if kind == SymbolKind::Reference {
        props
            .and_then(|p| p.attribute("Target"))
            .map(str::to_string)
    } else {
        None
    };
    ComponentProps {
        value_type,
        enum_assoc,
        declared_type,
        unit,
        qty,
        display_unit,
        security,
        call_rate_hz,
        scheduled,
        log_rate_hz,
        class,
        return_type,
        in_params,
        reference_target,
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
        qty: props.qty,
        display_unit: props.display_unit,
        security: props.security,
        filename,
        enum_assoc: props.enum_assoc,
        class: props.class,
        classname: Some(classname.to_string()),
        def_line,
        dbc_range: None,
        can: None,
        call_rate_hz: props.call_rate_hz,
        scheduled: props.scheduled,
        log_rate_hz: props.log_rate_hz,
        tags: props.tags,
        return_type: props.return_type,
        in_params: props.in_params,
        table_meta: props.table_meta,
        reference_target: props.reference_target,
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
    // in the pre-pass (closed under its display name when documented, otherwise
    // open under the type path), so the lookup resolves to its `Enum(id)`.
    if let Some(name) = firmware_enum_name(type_attr) {
        let registered = hardware_display_name(name).unwrap_or(name);
        if let Some(id) = table.enum_by_name(registered) {
            return (ValueType::Enum(id), Some(id));
        }
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
