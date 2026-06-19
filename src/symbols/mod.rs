//! Symbol model loaded from .m1prj / .m1cfg.
pub mod enums;
pub mod m1cfg;
pub mod m1dbc;
pub mod m1prj;

use crate::types::ValueType;
pub use enums::{EnumId, EnumType};
use std::collections::HashMap;

/// A failure to parse one of the MoTeC XML files (`.m1prj` / `.m1cfg` / `.m1dbc`).
///
/// The three loaders previously each had their own single-variant error enum
/// (`ParseError`, `ConfigError`, `DbcError`) wrapping a `roxmltree::Error`; this
/// is the one shared type they now return. `context` is the human-readable file
/// kind for the message (e.g. `".m1prj"`), so each loader's `Display` text is
/// preserved verbatim. `Project`'s `LoadError::Parse(String)` conversion still
/// works unchanged because it goes through this type's `Display`.
#[derive(Debug)]
pub struct XmlParseError {
    /// The file kind, for the error message (e.g. `".m1prj"`, `".m1cfg"`, `".m1dbc"`).
    pub context: &'static str,
    /// The underlying roxmltree parse error.
    pub source: roxmltree::Error,
}

impl XmlParseError {
    /// Build a closure that tags a `roxmltree::Error` with `context` — for use as
    /// `roxmltree::Document::parse(xml).map_err(XmlParseError::in_context(".m1prj"))`.
    pub fn in_context(context: &'static str) -> impl Fn(roxmltree::Error) -> XmlParseError {
        move |source| XmlParseError { context, source }
    }
}

impl std::fmt::Display for XmlParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid {} XML: {}", self.context, self.source)
    }
}

impl std::error::Error for XmlParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    /// The catch-all kind; also the inert [`Default`] used by `Symbol::default()`.
    #[default]
    Other,
}

#[derive(Debug, Clone, Default)]
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
    /// Raw physical quantity from `<Props Qty="…">`, verbatim (an SI unit string
    /// like `rad/s`, `Pa`, `K`, `ratio`). Unlike [`Symbol::unit`] (the base unit,
    /// with the SI→base exceptions applied) this is the *quantity key* — it matches
    /// a `<Quantity Symbol>` in MoTeC's units database, so the display-unit check
    /// (T095 / Error 1017) can look up which display units are valid. `None` when
    /// the project declares no quantity (or it is a `$(…)` template reference).
    pub qty: Option<String>,
    /// Raw display unit from `<Props><Locale><Default Unit="…">`, verbatim (e.g.
    /// `rpm`, `kPa`, `%`, `C`). This is the *display* unit, distinct from the stored
    /// base [`Symbol::unit`]; it must be one of the units valid for [`Symbol::qty`]
    /// (else M1 Build Error 1017). `None` when the object sets no explicit display
    /// unit (it then displays in the quantity's base unit, always valid).
    pub display_unit: Option<String>,
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
    /// True when this function/method is bound to a schedule event — i.e. its
    /// `.m1prj` `<Props>` declares a non-empty `SelectedTrigger` (a clock like
    /// `On 100Hz`, `On Startup`, or a `$(…)` parameter-reference trigger). Unlike
    /// [`Symbol::call_rate_hz`] — which is `None` for `On Startup`/`$(…)` triggers
    /// — this stays `true` for *every* triggered function, so it is the entry-point
    /// (root) predicate for the call-graph reachability check (T104 / M1 Build
    /// Error 1642). `false` for untriggered helper functions and non-functions.
    pub scheduled: bool,
    /// Default logging rate (Hz) of a channel, derived from its `.m1prj`
    /// `<Props DefaultLogRate="…">` — a period string like `"5MS"` (5 ms → 200 Hz)
    /// or `"10MS"` (100 Hz). `None` for symbols with no `DefaultLogRate` (the
    /// channel inherits the system default, which the `.m1prj` does not record).
    /// Surfaced in hover (#171).
    pub log_rate_hz: Option<f64>,
    /// Tags assigned to this object, from the `.m1prj` `<Props SelectedTags="…">`
    /// (space-separated), unioned with the tags inherited from every ancestor
    /// group (the manual: group tags are inherited by the group's children). Own
    /// tags come first, in declared order, then inherited tags not already present.
    /// Empty when neither the symbol nor any ancestor declares tags. Surfaced in
    /// hover and queryable via [`SymbolTable::symbols_with_tag`] (#170).
    pub tags: Vec<String>,
    /// Inferred return type of a user `BuiltIn.FuncUser`/`MethodUser`, from the
    /// concrete type its body assigns to `Out` (the manual's return-value object).
    /// `Some` only when every `Out = <expr>` in the body agrees on one known type;
    /// `None` when there is no `Out` assignment, the types disagree, or the symbol
    /// is not a user function — never a guessed type. Populated by
    /// (For a declared `<Signature ReturnType="…">` it is set at parse time;
    /// otherwise inferred by)
    /// [`crate::project::Project::infer_return_types`] and read by `type_of_call`
    /// so user-function call sites participate in T030/T003/T004/T021 (#110).
    pub return_type: Option<ValueType>,
    /// The function's declared input parameters from the `.m1prj`
    /// `<Signature><Params><Param Name Type>` (#110): name → value type, in
    /// declaration order. `None` when the component declares no signature —
    /// `In.*` references and call arguments are then unchecked (Opaque).
    pub in_params: Option<Vec<(String, ValueType)>>,
    /// For a `BuiltIn.Table` symbol, its shape (input axes) and output unit, read
    /// from the `.m1cfg` `<Table>` (the `.m1prj` carries none). `None` for
    /// non-tables and when no `.m1cfg` is loaded. See [`TableMeta`].
    pub table_meta: Option<TableMeta>,
    /// The raw `Target` attribute of a `BuiltIn.Reference` component's
    /// `<Props Target="…">`, verbatim (e.g. `"This.Value"`, or a cross-symbol
    /// path the reference aliases). Captured but **not** resolved or validated
    /// here — downstream tools document what the reference points at. `None` for
    /// non-references and for references that declare no `Target`.
    pub reference_target: Option<String>,
    /// For a [`SymbolKind::Group`], the raw `DefValue` of its
    /// `<Props UseDefValue="true" DefValue="…">` — the group's Default Value
    /// (e.g. `"This.Value"`). A group is a *value provider* (usable where a
    /// value is expected) only when it declares one; passing a group without a
    /// default value as a value is M1 Build Error 1331 (T106). `None` for
    /// non-groups and for groups that declare no usable default value.
    pub default_value: Option<String>,
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
    /// tag name -> indices of symbols that carry that tag (own or inherited).
    tag_index: HashMap<String, Vec<usize>>,
    /// script file name -> index of the Function/Method symbol it backs, for
    /// `function_symbol_for_script`'s explicit-`Filename=` lookup. First writer
    /// wins, preserving the previous declaration-order `iter().find()` result.
    by_filename: HashMap<String, usize>,
    /// parent path (everything before a symbol's final `.` segment) -> indices
    /// of that parent's immediate children, in declaration order. Backs
    /// `immediate_children` (LSP hover/completion member enumeration).
    by_parent: HashMap<String, Vec<usize>>,
}

impl SymbolTable {
    pub fn push(&mut self, sym: Symbol) {
        let idx = self.symbols.len();
        self.by_path.insert(sym.path.clone(), idx);
        for tag in &sym.tags {
            self.tag_index.entry(tag.clone()).or_default().push(idx);
        }
        if matches!(sym.kind, SymbolKind::Function | SymbolKind::Method)
            && let Some(file) = &sym.filename
        {
            // First writer wins: matches the previous declaration-order
            // `iter().find()` in `function_symbol_for_script`.
            self.by_filename.entry(file.clone()).or_insert(idx);
        }
        if let Some((parent, _leaf)) = sym.path.rsplit_once('.') {
            self.by_parent
                .entry(parent.to_string())
                .or_default()
                .push(idx);
        }
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
    /// Register a builtin (firmware/module) enum WITHOUT indexing its members
    /// in `member_index`. The index drives the `.m1cfg` enum-cell association,
    /// which must keep resolving against project-declared enums only: builtin
    /// member names (`On`, `OK`, `Normal`, …) recur across dozens of builtin
    /// enums and would otherwise make every association ambiguous.
    pub fn add_builtin_enum(&mut self, e: EnumType) -> EnumId {
        let id = self.enums.len();
        self.enums.push(e);
        id
    }
    pub fn enums(&self) -> &[EnumType] {
        &self.enums
    }
    /// The enum type `name` resolves to, if any. Exact name match wins; failing
    /// that, a *unique* ASCII-case-insensitive match resolves, because M1 Build
    /// resolves names case-insensitively (manual pp.64-65 — the behaviour
    /// T020's member carve-out and T091 already encode). An ambiguous
    /// case-variant (several enums differing only by case) resolves to nothing.
    /// (#183)
    pub fn enum_by_name(&self, name: &str) -> Option<EnumId> {
        if let Some(id) = self.enums.iter().position(|e| e.name == name) {
            return Some(id);
        }
        let mut variants = self
            .enums
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name.eq_ignore_ascii_case(name));
        match (variants.next(), variants.next()) {
            (Some((id, _)), None) => Some(id),
            _ => None,
        }
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
    /// `true` when the enum's member list is not exhaustively known (a
    /// firmware-supplied enum). Membership checks (T020) must skip open enums.
    pub fn enum_is_open(&self, id: EnumId) -> bool {
        self.enums.get(id).map(|e| e.open).unwrap_or(false)
    }
    pub fn enum_type(&self, id: EnumId) -> &EnumType {
        &self.enums[id]
    }
    /// Set a symbol's inferred return type (by path). No-op if absent (#110).
    pub fn set_return_type(&mut self, path: &str, ty: ValueType) {
        if let Some(&i) = self.by_path.get(path) {
            self.symbols[i].return_type = Some(ty);
        }
    }
    /// Associate a symbol (by path) with an enum type id. No-op if absent.
    pub fn set_enum_assoc(&mut self, path: &str, id: EnumId) {
        if let Some(&i) = self.by_path.get(path) {
            self.symbols[i].enum_assoc = Some(id);
            self.symbols[i].value_type = ValueType::Enum(id);
        }
    }
    /// Symbols carrying `tag` (own or inherited), in declaration order. Empty
    /// when no symbol carries it. Powers the `tag:` workspace-symbol filter (#170).
    pub fn symbols_with_tag(&self, tag: &str) -> Vec<&Symbol> {
        self.tag_index
            .get(tag)
            .map(|idxs| idxs.iter().map(|&i| &self.symbols[i]).collect())
            .unwrap_or_default()
    }

    /// The path of the Function/Method symbol whose explicit `Filename=` is
    /// `file_name`, if any (declaration-order first match). O(1) via the
    /// `by_filename` index; backs `Project::function_symbol_for_script`.
    pub fn function_path_for_filename(&self, file_name: &str) -> Option<&str> {
        self.by_filename
            .get(file_name)
            .map(|&i| self.symbols[i].path.as_str())
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
        self.by_parent
            .get(path)
            .map(|idxs| idxs.iter().map(|&i| &self.symbols[i]).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(path: &str, kind: SymbolKind, filename: Option<&str>) -> Symbol {
        Symbol {
            path: path.to_string(),
            kind,
            filename: filename.map(str::to_string),
            ..Symbol::default()
        }
    }

    // `Symbol::default()` is the inert symbol the builders fill in from: every
    // value-less field is its empty form, and the value type is the absorbing
    // `Unknown`. The DBC loader and this test helper rely on `..default()` to
    // avoid re-spelling the trivial fields, so a new field must not silently
    // pick up a non-inert default.
    #[test]
    fn default_symbol_is_inert() {
        let s = Symbol::default();
        assert!(s.path.is_empty());
        assert_eq!(s.kind, SymbolKind::Other);
        assert_eq!(s.value_type, ValueType::Unknown);
        assert_eq!(s.declared_type, None);
        assert_eq!(s.unit, None);
        assert_eq!(s.qty, None);
        assert_eq!(s.display_unit, None);
        assert_eq!(s.security, None);
        assert_eq!(s.filename, None);
        assert_eq!(s.enum_assoc, None);
        assert_eq!(s.class, None);
        assert_eq!(s.classname, None);
        assert_eq!(s.def_line, None);
        assert_eq!(s.dbc_range, None);
        assert_eq!(s.can, None);
        assert_eq!(s.call_rate_hz, None);
        assert!(!s.scheduled);
        assert_eq!(s.log_rate_hz, None);
        assert!(s.tags.is_empty());
        assert_eq!(s.return_type, None);
        assert_eq!(s.in_params, None);
        assert_eq!(s.table_meta, None);
        assert_eq!(s.reference_target, None);
        assert_eq!(s.default_value, None);
    }

    // `by_filename` resolves a Function/Method symbol by its `Filename=` in O(1),
    // first declared writer wins (matching the previous declaration-order
    // `iter().find()`), and non-function symbols are not indexed by filename.
    #[test]
    fn by_filename_index_is_first_function_for_that_file() {
        let mut t = SymbolTable::default();
        t.push(sym(
            "Root.Engine.Update",
            SymbolKind::Function,
            Some("Engine.Update.m1scr"),
        ));
        t.push(sym(
            "Root.Engine.Update2",
            SymbolKind::Function,
            Some("Engine.Update.m1scr"),
        ));
        // A channel sharing a filename must not shadow or be returned.
        t.push(sym(
            "Root.Engine.Chan",
            SymbolKind::Channel,
            Some("Engine.Update.m1scr"),
        ));
        assert_eq!(
            t.function_path_for_filename("Engine.Update.m1scr"),
            Some("Root.Engine.Update")
        );
        assert_eq!(t.function_path_for_filename("Missing.m1scr"), None);
    }

    // `immediate_children` (via `by_parent`) returns only direct children, keeps
    // declaration order, and tolerates segments that contain spaces.
    #[test]
    fn immediate_children_are_direct_in_declaration_order() {
        let mut t = SymbolTable::default();
        t.push(sym("Root", SymbolKind::Group, None));
        t.push(sym("Root.Ctrl", SymbolKind::Group, None));
        t.push(sym("Root.Ctrl.Seg 1", SymbolKind::Group, None));
        t.push(sym("Root.Ctrl.Seg 1.Volt", SymbolKind::Channel, None));
        t.push(sym("Root.Ctrl.Beta", SymbolKind::Channel, None));
        t.push(sym("Root.Ctrl.Alpha", SymbolKind::Channel, None));
        let children: Vec<&str> = t
            .immediate_children("Root.Ctrl")
            .iter()
            .map(|s| s.path.as_str())
            .collect();
        // Direct children only (grandchild `Seg 1.Volt` excluded), declaration order.
        assert_eq!(
            children,
            ["Root.Ctrl.Seg 1", "Root.Ctrl.Beta", "Root.Ctrl.Alpha"]
        );
        // The space-containing segment is itself a parent of its child.
        let grand: Vec<&str> = t
            .immediate_children("Root.Ctrl.Seg 1")
            .iter()
            .map(|s| s.path.as_str())
            .collect();
        assert_eq!(grand, ["Root.Ctrl.Seg 1.Volt"]);
        assert!(t.immediate_children("Root.Nope").is_empty());
    }

    fn enum_named(name: &str) -> EnumType {
        EnumType {
            name: name.to_string(),
            members: vec![("Off".to_string(), 0), ("On".to_string(), 1)],
            default: None,
            open: false,
        }
    }

    // M1 Build resolves names case-insensitively (manual pp.64-65): a unique
    // case-variant of an enum type name must resolve to that enum. (#183)
    #[test]
    fn enum_by_name_falls_back_to_unique_case_insensitive_match() {
        let mut t = SymbolTable::default();
        let id = t.add_enum(enum_named("Universal Switch State"));
        assert_eq!(t.enum_by_name("universal Switch State"), Some(id));
        assert_eq!(t.enum_by_name("UNIVERSAL SWITCH STATE"), Some(id));
    }

    #[test]
    fn enum_by_name_exact_match_wins_over_case_variants() {
        let mut t = SymbolTable::default();
        let _upper = t.add_enum(enum_named("State"));
        let lower = t.add_enum(enum_named("state"));
        assert_eq!(t.enum_by_name("state"), Some(lower));
    }

    #[test]
    fn enum_by_name_ambiguous_case_variants_resolve_to_none() {
        let mut t = SymbolTable::default();
        let _a = t.add_enum(enum_named("State"));
        let _b = t.add_enum(enum_named("state"));
        assert_eq!(t.enum_by_name("STATE"), None);
    }
}
