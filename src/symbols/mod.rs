//! Symbol model loaded from .m1prj / .m1cfg.
pub mod enums;
pub mod m1cfg;
pub mod m1dbc;
pub mod m1prj;

use crate::types::ValueType;
pub use enums::{EnumId, EnumType};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Channel,
    Parameter,
    Constant,
    Function,
    Method,
    Table,
    Group,
    Reference,
    /// An instance of a package-defined object class (a non-`BuiltIn.*`
    /// component, e.g. `MoTeC Input.Sensor`). Its class name is on
    /// [`Symbol::class`]; its members are the symbols whose path is prefixed by
    /// this object's path (see [`SymbolTable::immediate_children`]).
    Object,
    Other,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub path: String,
    pub kind: SymbolKind,
    pub value_type: ValueType,
    /// The raw storage type declared on the `.m1prj` `<Props Type="…">`, verbatim
    /// — a primitive (`f32`, `u32`, `s16`, …) or an enum reference (`::This.<Enum>`,
    /// `MoTeC Types.<Enum>`, `::Hardware.<…>`). `None` when the project declares no
    /// type. Unlike [`Symbol::value_type`], this is *not* lost for package-qualified
    /// enum types the model can't resolve to a specific [`EnumId`] — it preserves what
    /// the project actually wrote, which `.m1cfg` export needs.
    pub declared_type: Option<String>,
    pub unit: Option<String>,
    /// Security / access level for this symbol, from the `.m1prj`
    /// `<Props Security="…">` (e.g. `Tune`, `Calibration`, `Master Calibration`,
    /// `Resource`). `None` when the project declares none. Surfaced in hover.
    pub security: Option<String>,
    pub filename: Option<String>,
    /// Enum type this symbol's value belongs to, if known (set during .m1cfg
    /// back-resolution). When set, the typer reports `ValueType::Enum(_)`.
    pub enum_assoc: Option<EnumId>,
    /// For [`SymbolKind::Object`], the component's package class name
    /// (`Classname`, e.g. `"MoTeC Input.Sensor"`). `None` otherwise. This is the
    /// user-facing class shown in hover; it is the [`SymbolKind::Object`] subset
    /// of [`Symbol::classname`].
    pub class: Option<String>,
    /// Raw `Classname` attribute of the originating `<Component>`, verbatim for
    /// *every* component — `BuiltIn.Channel`, `BuiltIn.Parameter`, `BuiltIn.Table`,
    /// `BuiltIn.CAN.Signal`, `MoTeC Input.Sensor`, … — not just objects. `None`
    /// for symbols not sourced from a project/DBC `<Component>`. Lets the LSP
    /// reason about a component's class for class-aware structure checks and
    /// completion (#47). See [`crate::classname::valid_parent_classes`] for the schema
    /// of which parent classes a given child class is conventionally nested under.
    pub classname: Option<String>,
    /// 0-based line of this symbol's declaration in the `.m1prj` (the
    /// `<Component>` element), for goto-definition. `None` for symbols not
    /// sourced from the project file (e.g. DBC signals).
    pub def_line: Option<u32>,
    /// For a `BuiltIn.CAN.Signal`, its `(min, max)` *physical* value range,
    /// derived from the `.m1dbc` `Length`/`Type` (raw range) scaled by
    /// `Multiplier`/`Offset`. `None` for non-signals and float signals (whose
    /// range is unbounded for this check). Powers the T042 range check.
    pub dbc_range: Option<(f64, f64)>,
    /// CAN frame/signal layout metadata from a `.m1dbc`, for hover. Present on
    /// `BuiltIn.CAN.Message` objects (`can_id`/`dlc`) and `BuiltIn.CAN.Signal`
    /// channels (`start_bit`/`length`/`multiplier`/`offset`). `None` for symbols
    /// not sourced from a `.m1dbc`. See [`CanMeta`].
    pub can: Option<CanMeta>,
    /// Execution rate (Hz) of a script/function, derived from its `.m1prj`
    /// `<Props SelectedTrigger="…Events.On <N>Hz">` pointing at a
    /// `BuiltIn.EventKernel` clock. `None` for symbols with no trigger,
    /// startup-only functions (`On Startup`), and triggers that are parameter
    /// references (`$(…)`) the model can't resolve statically. Surfaced in hover.
    pub call_rate_hz: Option<f64>,
    /// For a `BuiltIn.Table` symbol, its shape (input axes) and output unit, read
    /// from the `.m1cfg` `<Table>` (the `.m1prj` carries none). `None` for
    /// non-tables and when no `.m1cfg` is loaded. See [`TableMeta`].
    pub table_meta: Option<TableMeta>,
}

/// Shape of a `BuiltIn.Table`, read from the `.m1cfg` `<Table>` element: its
/// input axes (X/Y/Z breakpoints) and the unit of the interpolated output. The
/// table's *output type* lands on its auto-created `.Value` channel
/// ([`Symbol::value_type`]); this struct carries the dimensionality a hover
/// wants to show (e.g. a 2-D `11×7` table). See `m1cfg::augment`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableMeta {
    /// Input axes in X, Y, Z order. `axes.len()` is the table's dimension; each
    /// [`TableAxis::size`] is that axis's breakpoint count.
    pub axes: Vec<TableAxis>,
    /// Unit of the interpolated output value (the `<Body>` cells), e.g. `N.m`.
    pub output_unit: Option<String>,
}

/// One input axis of a table: its breakpoint count and engineering unit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableAxis {
    /// Number of breakpoints along this axis.
    pub size: u32,
    /// Engineering unit of the axis breakpoints, when the cfg declares one.
    pub unit: Option<String>,
}

/// CAN layout metadata retained from a `.m1dbc` `<Props>`, attached to a
/// [`Symbol`] so the LSP can render it in hover. Message-level fields
/// (`can_id`, `dlc`) populate on `BuiltIn.CAN.Message` objects; signal-level
/// fields (`start_bit`, `length`, `multiplier`, `offset`) on
/// `BuiltIn.CAN.Signal` channels. A signal's parent message is looked up by
/// path to combine the two in one tooltip.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanMeta {
    /// CAN identifier of the message (decimal, as stored in the `.m1dbc`).
    pub can_id: Option<u32>,
    /// Data-length code — the message payload size in bytes.
    pub dlc: Option<u32>,
    /// Signal's least-significant bit position within the frame.
    pub start_bit: Option<u32>,
    /// Signal width in bits.
    pub length: Option<u32>,
    /// Scale factor applied to the raw value (`physical = raw * multiplier + offset`).
    pub multiplier: Option<f64>,
    /// Offset added after scaling.
    pub offset: Option<f64>,
}

#[derive(Debug, Default)]
pub struct SymbolTable {
    symbols: Vec<Symbol>,
    by_path: HashMap<String, usize>,
    enums: Vec<EnumType>,
    /// member name -> enum type ids that declare a member of that name.
    member_index: HashMap<String, Vec<EnumId>>,
}

impl SymbolTable {
    pub fn push(&mut self, sym: Symbol) {
        let idx = self.symbols.len();
        self.by_path.insert(sym.path.clone(), idx);
        self.symbols.push(sym);
    }
    pub fn get(&self, path: &str) -> Option<&Symbol> {
        self.by_path.get(path).map(|&i| &self.symbols[i])
    }
    pub fn get_mut(&mut self, path: &str) -> Option<&mut Symbol> {
        self.by_path.get(path).map(|&i| &mut self.symbols[i])
    }
    pub fn len(&self) -> usize {
        self.symbols.len()
    }
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.iter()
    }
    pub fn add_enum(&mut self, e: EnumType) -> EnumId {
        let id = self.enums.len();
        for (member, _order) in &e.members {
            self.member_index
                .entry(member.clone())
                .or_default()
                .push(id);
        }
        self.enums.push(e);
        id
    }
    pub fn enums(&self) -> &[EnumType] {
        &self.enums
    }
    /// The enum type whose *name* equals `name`, if any.
    pub fn enum_by_name(&self, name: &str) -> Option<EnumId> {
        self.enums.iter().position(|e| e.name == name)
    }
    /// Enum type ids that declare a member of this exact name.
    pub fn enums_with_member(&self, member: &str) -> &[EnumId] {
        self.member_index
            .get(member)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
    pub fn enum_has_member(&self, id: EnumId, member: &str) -> bool {
        self.enums
            .get(id)
            .map(|e| e.members.iter().any(|(m, _)| m == member))
            .unwrap_or(false)
    }
    pub fn enum_type(&self, id: EnumId) -> &EnumType {
        &self.enums[id]
    }
    /// Associate a symbol (by path) with an enum type id. No-op if absent.
    pub fn set_enum_assoc(&mut self, path: &str, id: EnumId) {
        if let Some(&i) = self.by_path.get(path) {
            self.symbols[i].enum_assoc = Some(id);
            self.symbols[i].value_type = ValueType::Enum(id);
        }
    }
    /// True if `path` exists as a symbol with a direct child of the given leaf
    /// name (e.g. a group `…Drive State` that contains `…Drive State.Value`).
    /// Used to recognise value-bearing enum/channel compounds, whose members
    /// expose built-in accessors that are not themselves stored symbols.
    pub fn has_child(&self, path: &str, leaf: &str) -> bool {
        self.by_path.contains_key(&format!("{path}.{leaf}"))
    }

    /// The immediate child symbols of `path` — symbols whose path is
    /// `"{path}.{leaf}"` where `leaf` is a single segment (segments are
    /// separated by `.`; a segment may contain spaces). Used to enumerate an
    /// object's members for hover/completion.
    pub fn immediate_children(&self, path: &str) -> Vec<&Symbol> {
        let prefix = format!("{path}.");
        self.symbols
            .iter()
            .filter(|s| {
                s.path
                    .strip_prefix(&prefix)
                    .is_some_and(|leaf| !leaf.contains('.'))
            })
            .collect()
    }
}
