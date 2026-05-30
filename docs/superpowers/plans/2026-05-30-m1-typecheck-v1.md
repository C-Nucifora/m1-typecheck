# m1-typecheck v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `.m1prj`/`.m1cfg` symbol model plus name resolution, a basic expression typer, and five type/naming rules (T001–T011) for `.m1scr` scripts, as a library + CLI.

**Architecture:** `roxmltree` parses the project XML into a `SymbolTable` (symbols by dotted path + enum types) and a `file→group` map. A per-script `Scope` (locals + enclosing group + project) drives a single CST walk; rules query a shared `type_of` typer. Everything is silent under `Unknown` to avoid false positives.

**Tech Stack:** Rust 2021, `m1-core` (path dep), `roxmltree` 0.20.

**Spec:** `docs/superpowers/specs/2026-05-30-m1-typecheck-v1-design.md`

**Prerequisites:** Rust ≥ 1.78; `m1-core` at `../m1-core`. Example project at
`../EV-M1/UQR-EV/01.00/Project.m1prj` (or `$M1_PROJECT`); corpus via
`$M1_CORPUS_PATH`. The corpus gate (Task 14) is skipped if absent.

---

## File structure

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | lib + bin; deps `m1-core`, `roxmltree` |
| `src/lib.rs` | public surface re-exports + `check_script*` |
| `src/types.rs` | `ValueType`, literal typing, numeric join |
| `src/symbols/mod.rs` | `SymbolTable`, `Symbol`, `SymbolKind` |
| `src/symbols/enums.rs` | `EnumType`, `EnumId` |
| `src/symbols/m1prj.rs` | parse `Project.m1prj` → symbols + enums + file→group |
| `src/symbols/m1cfg.rs` | parse `parameters.m1cfg` → augment param types |
| `src/project.rs` | `Project` (table + opaque roots + file→group) + `load` |
| `src/resolve.rs` | `Scope`, resolution algorithm, opaque roots |
| `src/typer.rs` | `type_of` over the CST |
| `src/diagnostics.rs` | `TypeCode`, `TypeDiagnostic`, `CheckResult` |
| `src/rules/mod.rs` | `Rule` trait, `Registry`, `Runner` |
| `src/rules/t0*.rs` | the five rules |
| `src/main.rs` | CLI |
| `tests/…`, `tests/fixtures/…` | unit/fixture/corpus tests |

---

## Task 0: Crate bootstrap

**Files:** Create `Cargo.toml`, `src/lib.rs`, `src/main.rs`, empty module files.

- [ ] **Step 1: `Cargo.toml`**

```toml
[package]
name = "m1-typecheck"
version = "0.1.0"
edition = "2021"
description = "Symbol model and type checker for the MoTeC M1 script language"
publish = false

[[bin]]
name = "m1-typecheck"
path = "src/main.rs"

[lib]
name = "m1_typecheck"
path = "src/lib.rs"

[dependencies]
m1-core = { path = "../m1-core" }
roxmltree = "0.20"
```

- [ ] **Step 2: `src/lib.rs`**

```rust
//! m1-typecheck: symbol model + name resolution + basic type checking for .m1scr.
pub mod types;
pub mod symbols;
pub mod project;
pub mod resolve;
pub mod typer;
pub mod diagnostics;
pub mod rules;

pub use diagnostics::{CheckResult, TypeCode, TypeDiagnostic};
pub use project::Project;
pub use types::ValueType;
```

- [ ] **Step 3: `src/main.rs` stub**

```rust
fn main() {
    eprintln!("m1-typecheck: not yet implemented");
    std::process::exit(1);
}
```

- [ ] **Step 4: module stubs**

Create `src/types.rs`, `src/project.rs`, `src/resolve.rs`, `src/typer.rs`,
`src/diagnostics.rs` with a single `//!` doc line each. Create `src/symbols/mod.rs`
with:
```rust
//! Symbol model loaded from .m1prj / .m1cfg.
pub mod enums;
pub mod m1prj;
pub mod m1cfg;
```
Create `src/symbols/enums.rs`, `src/symbols/m1prj.rs`, `src/symbols/m1cfg.rs` and
`src/rules/mod.rs` with a `//!` line each.

- [ ] **Step 5: Build**

Run: `cargo build`
Expected: compiles (warnings about unused/empty modules are fine).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/
git commit -m "Task 0: m1-typecheck crate skeleton (lib + bin, roxmltree)"
```

---

## Task 1: types.rs — ValueType, literal typing, numeric join

**Files:** Modify `src/types.rs`.

- [ ] **Step 1: Write the failing tests**

`src/types.rs`:
```rust
//! The type lattice and literal/arithmetic typing rules.

pub type EnumId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Boolean,
    Integer,
    Unsigned,
    Float,
    Enum(EnumId),
    String,
    Unknown,
}

impl ValueType {
    pub fn is_float(self) -> bool { matches!(self, ValueType::Float) }
    pub fn is_integral(self) -> bool { matches!(self, ValueType::Integer | ValueType::Unsigned) }
    pub fn is_known(self) -> bool { !matches!(self, ValueType::Unknown) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_literals() {
        assert_eq!(type_of_number_literal("1"), ValueType::Integer);
        assert_eq!(type_of_number_literal("42"), ValueType::Integer);
        assert_eq!(type_of_number_literal("0u"), ValueType::Unsigned);
        assert_eq!(type_of_number_literal("0xFF"), ValueType::Unsigned);
        assert_eq!(type_of_number_literal("0x1Fu"), ValueType::Unsigned);
        assert_eq!(type_of_number_literal("1.0"), ValueType::Float);
        assert_eq!(type_of_number_literal("1e3"), ValueType::Float);
        assert_eq!(type_of_number_literal("2.5e-3"), ValueType::Float);
    }

    #[test]
    fn joins() {
        use ValueType::*;
        assert_eq!(numeric_join(Integer, Float), Float);
        assert_eq!(numeric_join(Float, Unsigned), Float);
        assert_eq!(numeric_join(Unsigned, Unsigned), Unsigned);
        assert_eq!(numeric_join(Integer, Unsigned), Integer);
        assert_eq!(numeric_join(Unknown, Float), Unknown);
        assert_eq!(numeric_join(Boolean, Integer), Unknown);
    }

    #[test]
    fn prefix_types() {
        assert_eq!(type_from_hungarian("bReady"), Some(ValueType::Boolean));
        assert_eq!(type_from_hungarian("uCount"), Some(ValueType::Unsigned));
        assert_eq!(type_from_hungarian("iDelta"), Some(ValueType::Integer));
        assert_eq!(type_from_hungarian("fGain"), Some(ValueType::Float));
        assert_eq!(type_from_hungarian("count"), None);   // no prefix
        assert_eq!(type_from_hungarian("freq"), None);    // 'f' not followed by upper
        assert_eq!(type_from_hungarian("x"), None);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib types`
Expected: FAIL — `type_of_number_literal`, `numeric_join`, `type_from_hungarian` undefined.

- [ ] **Step 3: Implement**

Add above the tests:
```rust
/// Type of a numeric literal from its source text.
pub fn type_of_number_literal(text: &str) -> ValueType {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("0x") {
        return ValueType::Unsigned; // hex (optionally with trailing u)
    }
    if lower.ends_with('u') {
        return ValueType::Unsigned;
    }
    // Float if it has a decimal point or a (non-hex) exponent.
    if t.contains('.') || lower.contains('e') {
        return ValueType::Float;
    }
    ValueType::Integer
}

/// Result type of an arithmetic binary op over two operand types.
pub fn numeric_join(a: ValueType, b: ValueType) -> ValueType {
    use ValueType::*;
    match (a, b) {
        (Unknown, _) | (_, Unknown) => Unknown,
        (Float, x) | (x, Float) if x.is_float() || x.is_integral() => Float,
        (Unsigned, Unsigned) => Unsigned,
        (x, y) if x.is_integral() && y.is_integral() => Integer,
        _ => Unknown, // non-numeric operands: not our concern here
    }
}

/// Infer a local's type from its Hungarian prefix (prefix letter + UpperCase).
pub fn type_from_hungarian(name: &str) -> Option<ValueType> {
    let mut chars = name.chars();
    let first = chars.next()?;
    let second = chars.next()?;
    if !second.is_ascii_uppercase() {
        return None;
    }
    match first {
        'b' => Some(ValueType::Boolean),
        'u' => Some(ValueType::Unsigned),
        'i' => Some(ValueType::Integer),
        'f' => Some(ValueType::Float),
        _ => None,
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --lib types`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/types.rs
git commit -m "Task 1: ValueType, literal typing, numeric join, Hungarian prefix"
```

---

## Task 2: Symbol model + Project.m1prj parser

**Files:** Modify `src/symbols/enums.rs`, `src/symbols/mod.rs`, `src/symbols/m1prj.rs`. Create `tests/fixtures/mini.m1prj`, `tests/symbols.rs`.

- [ ] **Step 1: Write the fixture `tests/fixtures/mini.m1prj`**

```xml
<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="Mini" TargetHardware="ecu120">
  <SelectedModuleSets>
   <File Name="MoTeC Comms"/>
   <File Name="MoTeC Control"/>
  </SelectedModuleSets>
  <DataTypes>
   <Type Name="Switch State" Storage="enum" Default="Off">
    <Enum Name="Off" ContainerOrder="0"/>
    <Enum Name="On" ContainerOrder="1"/>
   </Type>
  </DataTypes>
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Foo"/>
    <Component Classname="BuiltIn.Channel" Name="Root.Foo.Speed"><Props Security="Tune"/></Component>
    <Component Classname="BuiltIn.Parameter" Name="Root.Foo.Gain.Value"><Props/></Component>
    <Component Classname="BuiltIn.Constant" Name="Root.Foo.Limit"><Props Value="100"/></Component>
    <Component Classname="BuiltIn.FuncUser" Filename="Foo Update.m1scr" Name="Root.Foo.Update"/>
    <Component Classname="BuiltIn.MethodUser" Name="Root.Foo.Startup"/>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
```

- [ ] **Step 2: Write `src/symbols/enums.rs`**

```rust
//! Enum data types declared in <DataTypes>.
pub type EnumId = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumType {
    pub name: String,
    pub members: Vec<(String, i64)>, // (name, ContainerOrder)
    pub default: Option<String>,
}
```

- [ ] **Step 3: Write `src/symbols/mod.rs`**

```rust
//! Symbol model loaded from .m1prj / .m1cfg.
pub mod enums;
pub mod m1prj;
pub mod m1cfg;

use crate::types::ValueType;
use std::collections::HashMap;
pub use enums::{EnumType, EnumId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Channel, Parameter, Constant, Function, Method, Table, Group, Reference, Other,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub path: String,
    pub kind: SymbolKind,
    pub value_type: ValueType,
    pub unit: Option<String>,
    pub filename: Option<String>,
}

#[derive(Debug, Default)]
pub struct SymbolTable {
    symbols: Vec<Symbol>,
    by_path: HashMap<String, usize>,
    enums: Vec<EnumType>,
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
    pub fn len(&self) -> usize { self.symbols.len() }
    pub fn is_empty(&self) -> bool { self.symbols.is_empty() }
    pub fn iter(&self) -> impl Iterator<Item = &Symbol> { self.symbols.iter() }
    pub fn add_enum(&mut self, e: EnumType) -> EnumId { self.enums.push(e); self.enums.len() - 1 }
    pub fn enums(&self) -> &[EnumType] { &self.enums }
}
```

- [ ] **Step 4: Write the failing test `tests/symbols.rs`**

```rust
use std::path::Path;
use m1_typecheck::symbols::{m1prj, SymbolKind};

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mini.m1prj")
}

#[test]
fn parses_symbols_and_kinds() {
    let parsed = m1prj::parse(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    let table = &parsed.table;
    assert_eq!(table.get("Root.Foo.Speed").unwrap().kind, SymbolKind::Channel);
    assert_eq!(table.get("Root.Foo.Gain.Value").unwrap().kind, SymbolKind::Parameter);
    assert_eq!(table.get("Root.Foo.Limit").unwrap().kind, SymbolKind::Constant);
    assert_eq!(table.get("Root.Foo.Update").unwrap().kind, SymbolKind::Function);
    assert_eq!(table.get("Root.Foo.Startup").unwrap().kind, SymbolKind::Method);
    assert_eq!(table.get("Root.Foo").unwrap().kind, SymbolKind::Group);
}

#[test]
fn parses_enum_types() {
    let parsed = m1prj::parse(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    let e = parsed.table.enums().iter().find(|e| e.name == "Switch State").unwrap();
    assert_eq!(e.default.as_deref(), Some("Off"));
    assert_eq!(e.members, vec![("Off".to_string(), 0), ("On".to_string(), 1)]);
}

#[test]
fn maps_filename_to_enclosing_group() {
    let parsed = m1prj::parse(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    assert_eq!(parsed.file_to_group.get("Foo Update.m1scr").map(String::as_str), Some("Root.Foo"));
}

#[test]
fn collects_module_set_roots() {
    let parsed = m1prj::parse(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    assert!(parsed.module_roots.contains("MoTeC Comms"));
}
```

- [ ] **Step 5: Run to verify it fails**

Run: `cargo test --test symbols`
Expected: FAIL — `m1prj::parse` and `Parsed` undefined.

- [ ] **Step 6: Implement `src/symbols/m1prj.rs`**

```rust
//! Parse Project.m1prj into a SymbolTable, enum types, and a file->group map.
use std::collections::{HashMap, HashSet};
use crate::types::ValueType;
use super::{EnumType, Symbol, SymbolKind, SymbolTable};

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
        match self { ParseError::Xml(e) => write!(f, "invalid .m1prj XML: {e}") }
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
    } else if classname.starts_with("BuiltIn.Constant") || classname == "BuiltIn.IOResourceConstant" {
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

pub fn parse(xml: &str) -> Result<Parsed, ParseError> {
    let doc = roxmltree::Document::parse(xml).map_err(ParseError::Xml)?;
    let mut table = SymbolTable::default();
    let mut file_to_group = HashMap::new();
    let mut module_roots = HashSet::new();

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
                            c.attribute("ContainerOrder").and_then(|s| s.parse().ok()).unwrap_or(0),
                        )
                    })
                    .collect();
                table.add_enum(EnumType { name, members, default });
            }
            "Component" => {
                let (Some(classname), Some(name)) =
                    (node.attribute("Classname"), node.attribute("Name"))
                else { continue };
                let kind = classify(classname);
                let filename = node.attribute("Filename").map(str::to_string);
                if matches!(kind, SymbolKind::Function | SymbolKind::Method) {
                    if let (Some(f), Some(g)) = (filename.clone(), parent_group(name)) {
                        file_to_group.insert(f, g);
                    }
                }
                // Constant value type from Props/@Value literal, if any.
                let value_type = if kind == SymbolKind::Constant {
                    node.children()
                        .find(|c| c.has_tag_name("Props"))
                        .and_then(|p| p.attribute("Value"))
                        .map(constant_value_type)
                        .unwrap_or(ValueType::Unknown)
                } else {
                    ValueType::Unknown
                };
                table.push(Symbol { path: name.to_string(), kind, value_type, unit: None, filename });
            }
            _ => {}
        }
    }
    Ok(Parsed { table, file_to_group, module_roots })
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
```

- [ ] **Step 7: Run to verify it passes**

Run: `cargo test --test symbols`
Expected: PASS (4 tests).

- [ ] **Step 8: Commit**

```bash
git add src/symbols/ tests/symbols.rs tests/fixtures/mini.m1prj
git commit -m "Task 2: SymbolTable + Project.m1prj parser (kinds, enums, file->group)"
```

---

## Task 3: parameters.m1cfg parser (augment param types)

**Files:** Modify `src/symbols/m1cfg.rs`. Create `tests/fixtures/mini.m1cfg`. Extend `tests/symbols.rs`.

- [ ] **Step 1: Write `tests/fixtures/mini.m1cfg`**

```xml
<?xml version="1.0"?>
<Configuration>
 <Group Name="">
  <Parameter Name="Root.Foo.Gain.Value"><Cell Type="f32" Unit="ratio"><![CDATA[1.5]]></Cell></Parameter>
 </Group>
</Configuration>
```

- [ ] **Step 2: Write the failing test (append to `tests/symbols.rs`)**

```rust
#[test]
fn m1cfg_augments_parameter_types() {
    use m1_typecheck::symbols::{m1prj, m1cfg};
    use m1_typecheck::types::ValueType;
    let mut parsed = m1prj::parse(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    let cfg = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mini.m1cfg"),
    ).unwrap();
    m1cfg::augment(&mut parsed.table, &cfg).unwrap();
    let gain = parsed.table.get("Root.Foo.Gain.Value").unwrap();
    assert_eq!(gain.value_type, ValueType::Float);
    assert_eq!(gain.unit.as_deref(), Some("ratio"));
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test --test symbols m1cfg_augments`
Expected: FAIL — `m1cfg::augment` undefined.

- [ ] **Step 4: Implement `src/symbols/m1cfg.rs`**

```rust
//! Parse parameters.m1cfg and augment matching symbols with value type + unit.
use crate::types::ValueType;
use super::SymbolTable;

#[derive(Debug)]
pub enum ConfigError { Xml(roxmltree::Error) }
impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { ConfigError::Xml(e) => write!(f, "invalid .m1cfg XML: {e}") }
    }
}
impl std::error::Error for ConfigError {}

pub fn cell_type(t: &str) -> ValueType {
    match t {
        "f32" | "f64" => ValueType::Float,
        "u8" | "u16" | "u32" | "u64" => ValueType::Unsigned,
        "s8" | "s16" | "s32" | "s64" => ValueType::Integer,
        "bool" => ValueType::Boolean,
        "enum" => ValueType::Unknown, // enum association deferred to v2
        _ => ValueType::Unknown,
    }
}

pub fn augment(table: &mut SymbolTable, xml: &str) -> Result<(), ConfigError> {
    let doc = roxmltree::Document::parse(xml).map_err(ConfigError::Xml)?;
    for param in doc.descendants().filter(|n| n.has_tag_name("Parameter")) {
        let Some(name) = param.attribute("Name") else { continue };
        let Some(cell) = param.children().find(|c| c.has_tag_name("Cell")) else { continue };
        let vt = cell.attribute("Type").map(cell_type).unwrap_or(ValueType::Unknown);
        let unit = cell.attribute("Unit").map(str::to_string);
        if let Some(sym) = table.get_mut(name) {
            sym.value_type = vt;
            sym.unit = unit;
        }
    }
    Ok(())
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test --test symbols`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add src/symbols/m1cfg.rs tests/symbols.rs tests/fixtures/mini.m1cfg
git commit -m "Task 3: parameters.m1cfg parser augments parameter value types/units"
```

---

## Task 4: Project — load + opaque roots

**Files:** Modify `src/project.rs`. Create `tests/project.rs`.

- [ ] **Step 1: Write the failing test `tests/project.rs`**

```rust
use std::path::Path;
use m1_typecheck::project::Project;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("mini.m1prj")).unwrap()
        .with_config(&dir.join("mini.m1cfg")).unwrap()
}

#[test]
fn loads_table_and_config() {
    let p = proj();
    assert!(p.symbols().get("Root.Foo.Speed").is_some());
    assert_eq!(p.symbols().get("Root.Foo.Gain.Value").unwrap().unit.as_deref(), Some("ratio"));
}

#[test]
fn opaque_roots_include_modules_and_builtins() {
    let p = proj();
    assert!(p.is_opaque_root("MoTeC Comms")); // from module sets
    assert!(p.is_opaque_root("Calculate"));   // static builtin
    assert!(p.is_opaque_root("This"));
    assert!(!p.is_opaque_root("Root"));        // Root is a project group, not opaque
}

#[test]
fn group_for_script() {
    let p = proj();
    assert_eq!(p.group_for_script("Foo Update.m1scr").as_deref(), Some("Root.Foo"));
    assert_eq!(p.group_for_script("Unknown.m1scr"), None);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test project`
Expected: FAIL — `Project` API undefined.

- [ ] **Step 3: Implement `src/project.rs`**

```rust
//! The loaded project: symbol table + enum types + opaque roots + file->group.
use std::collections::{HashMap, HashSet};
use std::path::Path;
use crate::symbols::{m1cfg, m1prj, SymbolTable};

/// M1 standard-library roots always treated as opaque (never flagged).
const STATIC_OPAQUE_ROOTS: &[&str] = &[
    "This", "Calculate", "Limit", "Convert", "Math", "Time", "Filter",
    "CanComms", "Table", "Channel", "Constant", "Parameter", "Diagnostic",
];

pub struct Project {
    table: SymbolTable,
    opaque_roots: HashSet<String>,
    file_to_group: HashMap<String, String>,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(String),
}
impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "{e}"),
            LoadError::Parse(s) => write!(f, "{s}"),
        }
    }
}
impl std::error::Error for LoadError {}

impl Project {
    pub fn load(m1prj_path: &Path) -> Result<Project, LoadError> {
        let xml = std::fs::read_to_string(m1prj_path).map_err(LoadError::Io)?;
        let parsed = m1prj::parse(&xml).map_err(|e| LoadError::Parse(e.to_string()))?;
        let mut opaque_roots: HashSet<String> =
            STATIC_OPAQUE_ROOTS.iter().map(|s| s.to_string()).collect();
        opaque_roots.extend(parsed.module_roots);
        Ok(Project {
            table: parsed.table,
            opaque_roots,
            file_to_group: parsed.file_to_group,
        })
    }

    pub fn with_config(mut self, m1cfg_path: &Path) -> Result<Project, LoadError> {
        let xml = std::fs::read_to_string(m1cfg_path).map_err(LoadError::Io)?;
        m1cfg::augment(&mut self.table, &xml).map_err(|e| LoadError::Parse(e.to_string()))?;
        Ok(self)
    }

    pub fn symbols(&self) -> &SymbolTable { &self.table }

    pub fn is_opaque_root(&self, root: &str) -> bool { self.opaque_roots.contains(root) }

    /// Enclosing group for a script file name (the bare `Filename`, e.g.
    /// "Foo Update.m1scr"); None if the project has no such function/method.
    pub fn group_for_script(&self, file_name: &str) -> Option<String> {
        self.file_to_group.get(file_name).cloned()
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --test project`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/project.rs tests/project.rs
git commit -m "Task 4: Project::load (+config), opaque roots, file->group lookup"
```

---

## Task 5: resolve.rs — Scope and name resolution

**Files:** Modify `src/resolve.rs`. Create `tests/resolve.rs`.

- [ ] **Step 1: Write the failing tests `tests/resolve.rs`**

```rust
use std::collections::HashMap;
use std::path::Path;
use m1_typecheck::project::Project;
use m1_typecheck::resolve::{resolve, Resolution, Scope};
use m1_typecheck::types::ValueType;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("mini.m1prj")).unwrap()
}

#[test]
fn resolves_absolute() {
    let p = proj();
    let scope = Scope { locals: HashMap::new(), group: None, project: Some(&p) };
    assert!(matches!(resolve("Root.Foo.Speed", &scope), Resolution::Symbol(_)));
}

#[test]
fn resolves_group_relative() {
    let p = proj();
    let scope = Scope { locals: HashMap::new(), group: Some("Root.Foo".into()), project: Some(&p) };
    // "Speed" relative to Root.Foo -> Root.Foo.Speed
    assert!(matches!(resolve("Speed", &scope), Resolution::Symbol(_)));
}

#[test]
fn resolves_local() {
    let p = proj();
    let mut locals = HashMap::new();
    locals.insert("fGain".to_string(), ValueType::Float);
    let scope = Scope { locals, group: None, project: Some(&p) };
    assert!(matches!(resolve("fGain", &scope), Resolution::Local(ValueType::Float)));
}

#[test]
fn opaque_root_passes_through() {
    let p = proj();
    let scope = Scope { locals: HashMap::new(), group: Some("Root.Foo".into()), project: Some(&p) };
    assert!(matches!(resolve("Calculate.Min", &scope), Resolution::Opaque));
}

#[test]
fn unresolved_when_project_rooted_but_absent() {
    let p = proj();
    let scope = Scope { locals: HashMap::new(), group: Some("Root.Foo".into()), project: Some(&p) };
    assert!(matches!(resolve("Root.Foo.Missing", &scope), Resolution::Unresolved));
}

#[test]
fn unknown_root_is_opaque_not_unresolved() {
    let p = proj();
    let scope = Scope { locals: HashMap::new(), group: Some("Root.Foo".into()), project: Some(&p) };
    // "Bar" is neither a project group root nor opaque-listed; treat as opaque
    // (conservative: only project-rooted misses are flagged).
    assert!(matches!(resolve("Bar.Whatever", &scope), Resolution::Opaque));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test resolve`
Expected: FAIL — `resolve`, `Resolution`, `Scope` undefined.

- [ ] **Step 3: Implement `src/resolve.rs`**

```rust
//! Name resolution: locals, absolute, group-relative, opaque-root passthrough.
use std::collections::HashMap;
use crate::project::Project;
use crate::types::ValueType;

pub struct Scope<'p> {
    pub locals: HashMap<String, ValueType>,
    pub group: Option<String>,
    pub project: Option<&'p Project>,
}

#[derive(Debug)]
pub enum Resolution<'p> {
    Local(ValueType),
    Symbol(&'p crate::symbols::Symbol),
    Opaque,       // resolves to a built-in/unknown root; type Unknown, never flagged
    Unresolved,   // project-rooted path with no matching symbol
}

fn root_segment(path: &str) -> &str {
    match path.find('.') { Some(i) => &path[..i], None => path }
}

pub fn resolve<'p>(path: &str, scope: &Scope<'p>) -> Resolution<'p> {
    // 1. Local (single segment).
    if !path.contains('.') {
        if let Some(&t) = scope.locals.get(path) {
            return Resolution::Local(t);
        }
    }
    let Some(project) = scope.project else {
        // project-less mode: locals only, everything else opaque
        return Resolution::Opaque;
    };
    let table = project.symbols();

    // 2. Absolute (as-is, then Root.-prefixed).
    if let Some(sym) = table.get(path) {
        return Resolution::Symbol(sym);
    }
    let rooted = format!("Root.{path}");
    if let Some(sym) = table.get(&rooted) {
        return Resolution::Symbol(sym);
    }

    // 3. Group-relative: walk the enclosing group up to Root.
    if let Some(group) = &scope.group {
        let mut prefix = Some(group.clone());
        while let Some(g) = prefix {
            let candidate = format!("{g}.{path}");
            if let Some(sym) = table.get(&candidate) {
                return Resolution::Symbol(sym);
            }
            prefix = g.rfind('.').map(|i| g[..i].to_string());
        }
    }

    // 4. Opaque vs unresolved: only flag when the root is a known PROJECT group.
    let root = root_segment(path);
    let root_is_project_group = table
        .get(&format!("Root.{root}"))
        .map(|s| matches!(s.kind, crate::symbols::SymbolKind::Group))
        .unwrap_or(false)
        || root == "Root";
    if root_is_project_group {
        Resolution::Unresolved
    } else {
        Resolution::Opaque
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --test resolve`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add src/resolve.rs tests/resolve.rs
git commit -m "Task 5: name resolution (local/absolute/relative/opaque) with project-rooted miss detection"
```

---

## Task 6: typer.rs — type_of over the CST

**Files:** Modify `src/typer.rs`. Create `tests/typer.rs`.

- [ ] **Step 1: Write the failing tests `tests/typer.rs`**

```rust
use std::collections::HashMap;
use m1_typecheck::resolve::Scope;
use m1_typecheck::typer::type_of_source_expr;
use m1_typecheck::types::ValueType;

// Helper: type the single expression in `<expr>;` with the given locals.
fn ty(src_expr: &str, locals: &[(&str, ValueType)]) -> ValueType {
    let mut m = HashMap::new();
    for (k, v) in locals { m.insert(k.to_string(), *v); }
    let scope = Scope { locals: m, group: None, project: None };
    type_of_source_expr(&format!("{src_expr};\n"), &scope)
}

#[test]
fn literals() {
    assert_eq!(ty("1", &[]), ValueType::Integer);
    assert_eq!(ty("1.0", &[]), ValueType::Float);
    assert_eq!(ty("0u", &[]), ValueType::Unsigned);
    assert_eq!(ty("true", &[]), ValueType::Boolean);
    assert_eq!(ty("\"hi\"", &[]), ValueType::String);
}

#[test]
fn local_and_arithmetic() {
    assert_eq!(ty("fGain", &[("fGain", ValueType::Float)]), ValueType::Float);
    assert_eq!(ty("1 + 2", &[]), ValueType::Integer);
    assert_eq!(ty("fGain * 2", &[("fGain", ValueType::Float)]), ValueType::Float);
}

#[test]
fn comparisons_are_boolean() {
    assert_eq!(ty("1 < 2", &[]), ValueType::Boolean);
    assert_eq!(ty("a eq b", &[]), ValueType::Boolean); // a,b opaque -> still Boolean
}

#[test]
fn calls_are_unknown() {
    assert_eq!(ty("Calculate.Min(1, 2)", &[]), ValueType::Unknown);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test typer`
Expected: FAIL — `type_of_source_expr` undefined.

- [ ] **Step 3: Implement `src/typer.rs`**

```rust
//! Best-effort expression typing over the m1-core CST.
use m1_core::{Kind, Node};
use crate::resolve::{resolve, Resolution, Scope};
use crate::types::{numeric_join, type_of_number_literal, ValueType};

/// Type the first expression found in a parsed source string. Test/utility entry.
pub fn type_of_source_expr(src: &str, scope: &Scope) -> ValueType {
    let cst = m1_core::parse(src);
    // Find the first expression-bearing statement and type its expression child.
    fn first_expr<'a>(n: Node<'a>) -> Option<Node<'a>> {
        if is_expr(n.kind()) { return Some(n); }
        for c in n.children() {
            if let Some(e) = first_expr(c) { return Some(e); }
        }
        None
    }
    match first_expr(cst.root()) {
        Some(e) => type_of(e, scope),
        None => ValueType::Unknown,
    }
}

fn is_expr(k: Kind) -> bool {
    matches!(
        k,
        Kind::Identifier
            | Kind::MemberExpression
            | Kind::CallExpression
            | Kind::UnaryExpression
            | Kind::BinaryExpression
            | Kind::TernaryExpression
            | Kind::ParenthesizedExpression
            | Kind::Number
            | Kind::Boolean
            | Kind::String
    )
}

pub fn type_of(node: Node, scope: &Scope) -> ValueType {
    match node.kind() {
        Kind::Number => type_of_number_literal(node.text()),
        Kind::Boolean => ValueType::Boolean,
        Kind::String => ValueType::String,
        Kind::ParenthesizedExpression => {
            child_expr(node).map(|c| type_of(c, scope)).unwrap_or(ValueType::Unknown)
        }
        Kind::Identifier | Kind::MemberExpression => match resolve(&path_text(node), scope) {
            Resolution::Local(t) => t,
            Resolution::Symbol(s) => s.value_type,
            Resolution::Opaque | Resolution::Unresolved => ValueType::Unknown,
        },
        Kind::CallExpression => ValueType::Unknown, // v1: return types unknown
        Kind::UnaryExpression => {
            // `not`/`!` -> Boolean; `-x` -> type of x.
            let op_is_not = node.children().iter().any(|c| matches!(c.kind(), Kind::Not | Kind::Bang));
            if op_is_not {
                ValueType::Boolean
            } else {
                child_expr(node).map(|c| type_of(c, scope)).unwrap_or(ValueType::Unknown)
            }
        }
        Kind::TernaryExpression => {
            let exprs: Vec<_> = node.named_children().into_iter().filter(|c| is_expr(c.kind())).collect();
            // condition, consequence, alternative -> join the two branches
            match (exprs.get(1), exprs.get(2)) {
                (Some(a), Some(b)) => {
                    let (ta, tb) = (type_of(*a, scope), type_of(*b, scope));
                    if ta == tb { ta } else { numeric_join(ta, tb) }
                }
                _ => ValueType::Unknown,
            }
        }
        Kind::BinaryExpression => type_of_binary(node, scope),
        _ => ValueType::Unknown,
    }
}

fn type_of_binary(node: Node, scope: &Scope) -> ValueType {
    let operands: Vec<_> = node.named_children().into_iter().filter(|c| is_expr(c.kind())).collect();
    let (l, r) = match (operands.first(), operands.get(1)) {
        (Some(l), Some(r)) => (type_of(*l, scope), type_of(*r, scope)),
        _ => return ValueType::Unknown,
    };
    match binary_op_kind(node) {
        OpClass::Comparison | OpClass::Logical => ValueType::Boolean,
        OpClass::Arithmetic => numeric_join(l, r),
        OpClass::Bitwise => l, // result follows the (integer) left operand
        OpClass::Unknown => ValueType::Unknown,
    }
}

enum OpClass { Arithmetic, Comparison, Logical, Bitwise, Unknown }

fn binary_op_kind(node: Node) -> OpClass {
    for c in node.children() {
        match c.kind() {
            Kind::Plus | Kind::Minus | Kind::Star | Kind::Slash | Kind::Percent => return OpClass::Arithmetic,
            Kind::EqEq | Kind::BangEq | Kind::Eq | Kind::Neq
            | Kind::Lt | Kind::Gt | Kind::LtEq | Kind::GtEq => return OpClass::Comparison,
            Kind::And | Kind::Or | Kind::AmpAmp | Kind::PipePipe => return OpClass::Logical,
            Kind::Amp | Kind::Pipe | Kind::Caret | Kind::LtLt | Kind::GtGt => return OpClass::Bitwise,
            _ => {}
        }
    }
    OpClass::Unknown
}

fn child_expr<'a>(node: Node<'a>) -> Option<Node<'a>> {
    node.named_children().into_iter().find(|c| is_expr(c.kind()))
}

/// The dotted source text of an identifier / member-expression path.
pub fn path_text(node: Node) -> String {
    node.text().to_string()
}
```

> Note on `Kind` variants: `Eq`/`Neq`/`And`/`Or`/`Not` are the **word** operators
> (`eq`/`neq`/`and`/`or`/`not`); `EqEq`/`BangEq`/`AmpAmp`/`PipePipe`/`Bang` are the
> symbolic forms. Both are real variants in `m1-core`'s generated `Kind` enum
> (confirmed in `tree-sitter-m1`/`m1-core`). `path_text` uses `node.text()`, which
> for a member-expression yields the full dotted path including space-containing
> segments, exactly what `resolve` expects.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --test typer`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/typer.rs tests/typer.rs
git commit -m "Task 6: best-effort expression typer over the CST"
```

---

## Task 7: diagnostics + Rule/Registry/Runner

**Files:** Modify `src/diagnostics.rs`, `src/rules/mod.rs`.

- [ ] **Step 1: Write `src/diagnostics.rs`**

```rust
//! Type-check diagnostic codes and result types.
use m1_core::Diagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCode {
    T001, // unresolved-reference
    T002, // float-equality
    T003, // int-float-mixing
    T010, // local-hungarian-prefix
    T011, // hungarian-prefix-mismatch
}

impl TypeCode {
    pub fn as_str(self) -> &'static str {
        match self {
            TypeCode::T001 => "T001",
            TypeCode::T002 => "T002",
            TypeCode::T003 => "T003",
            TypeCode::T010 => "T010",
            TypeCode::T011 => "T011",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TypeDiagnostic {
    pub code: TypeCode,
    pub inner: Diagnostic,
}

#[derive(Debug, Default)]
pub struct CheckResult {
    pub diagnostics: Vec<TypeDiagnostic>,
    pub syntax_errors: Vec<Diagnostic>,
}
```

- [ ] **Step 2: Write `src/rules/mod.rs` (trait + runner) with a unit test**

```rust
//! Rule trait, registry, and the single-walk runner.
use std::path::Path;
use m1_core::Node;
use crate::diagnostics::{CheckResult, TypeDiagnostic};
use crate::project::Project;
use crate::resolve::Scope;

pub mod t001_unresolved;
pub mod t002_float_eq;
pub mod t003_int_float_mix;
pub mod t010_local_prefix;
pub mod t011_prefix_mismatch;

pub trait Rule: Send + Sync {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>);
}

pub struct Registry { rules: Vec<Box<dyn Rule>> }

impl Registry {
    pub fn default_v1() -> Self {
        Registry {
            rules: vec![
                Box::new(t001_unresolved::Rule),
                Box::new(t002_float_eq::Rule),
                Box::new(t003_int_float_mix::Rule),
                Box::new(t010_local_prefix::Rule),
                Box::new(t011_prefix_mismatch::Rule),
            ],
        }
    }
    pub fn rules(&self) -> &[Box<dyn Rule>] { &self.rules }
}

/// Collect locals (name -> type) declared anywhere in the script.
fn collect_locals(root: Node) -> std::collections::HashMap<String, crate::types::ValueType> {
    use crate::types::{type_from_hungarian, ValueType};
    let mut locals = std::collections::HashMap::new();
    fn walk(n: Node, locals: &mut std::collections::HashMap<String, ValueType>) {
        if n.kind() == m1_core::Kind::LocalDeclaration {
            if let Some(name_node) = n.named_children().into_iter().find(|c| c.kind() == m1_core::Kind::Identifier) {
                let name = name_node.text().to_string();
                let t = type_from_hungarian(&name).unwrap_or(ValueType::Unknown);
                locals.insert(name, t);
            }
        }
        for c in n.children() { walk(c, locals); }
    }
    walk(root, &mut locals);
    locals
}

/// Check one script. `script_path` (file name) anchors group-relative resolution.
pub fn check_script(project: &Project, script_path: &Path, source: &str) -> CheckResult {
    run(Some(project), script_path.file_name().and_then(|s| s.to_str()), source)
}

pub fn check_script_no_project(source: &str) -> CheckResult {
    run(None, None, source)
}

fn run(project: Option<&Project>, file_name: Option<&str>, source: &str) -> CheckResult {
    let cst = m1_core::parse(source);
    let syntax_errors = cst.syntax_diagnostics();
    if !syntax_errors.is_empty() {
        return CheckResult { diagnostics: vec![], syntax_errors };
    }
    let group = match (project, file_name) {
        (Some(p), Some(f)) => p.group_for_script(f),
        _ => None,
    };
    let scope = Scope { locals: collect_locals(cst.root()), group, project };
    let registry = Registry::default_v1();
    let mut diagnostics = Vec::new();
    walk(cst.root(), &registry, &scope, &mut diagnostics);
    CheckResult { diagnostics, syntax_errors }
}

fn walk(node: Node, registry: &Registry, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    for rule in registry.rules() {
        rule.check_node(&node, scope, out);
    }
    for c in node.children() {
        walk(c, registry, scope, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_script_no_project_has_no_diagnostics_for_resolution() {
        // local with proper prefix, simple int math -> no T010/T011/T002/T003
        let r = check_script_no_project("local iSum = 1 + 2;\n");
        assert!(r.syntax_errors.is_empty());
        assert!(r.diagnostics.iter().all(|d| d.code != crate::diagnostics::TypeCode::T010));
    }
}
```

- [ ] **Step 3: Add empty rule modules so it compiles**

Create each `src/rules/t0*.rs` with a stub that compiles:
```rust
use m1_core::Node;
use crate::diagnostics::TypeDiagnostic;
use crate::resolve::Scope;

pub struct Rule;
impl super::Rule for Rule {
    fn check_node(&self, _node: &Node, _scope: &Scope, _out: &mut Vec<TypeDiagnostic>) {}
}
```

- [ ] **Step 4: Build + run the runner test**

Run: `cargo test --lib rules`
Expected: PASS (the no-op rules produce no diagnostics; the test asserts that).

- [ ] **Step 5: Commit**

```bash
git add src/diagnostics.rs src/rules/
git commit -m "Task 7: diagnostics types + Rule/Registry/Runner with local collection"
```

---

## Task 8: T002 float-equality

**Files:** Modify `src/rules/t002_float_eq.rs`. Create `tests/rules.rs`.

- [ ] **Step 1: Write the failing test `tests/rules.rs`**

```rust
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::rules::check_script_no_project;

fn codes(src: &str) -> Vec<TypeCode> {
    check_script_no_project(src).diagnostics.iter().map(|d| d.code).collect()
}

#[test]
fn t002_flags_float_literal_equality() {
    // fValue is Float (Hungarian); compared with == to a float literal
    assert!(codes("local fValue = 1.0;\nlocal bX = fValue == 1.0;\n").contains(&TypeCode::T002));
}

#[test]
fn t002_no_false_positive_on_integers() {
    assert!(!codes("local iA = 1;\nlocal bX = iA == 2;\n").contains(&TypeCode::T002));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test rules t002`
Expected: FAIL — no T002 emitted (stub).

- [ ] **Step 3: Implement `src/rules/t002_float_eq.rs`**

```rust
use m1_core::{Kind, Node};
use crate::diagnostics::{TypeCode, TypeDiagnostic};
use crate::resolve::Scope;
use crate::typer::type_of;
use crate::types::ValueType;
use m1_core::{Code, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::BinaryExpression {
            return;
        }
        let is_eq = node.children().iter().any(|c| {
            matches!(c.kind(), Kind::EqEq | Kind::BangEq | Kind::Eq | Kind::Neq)
        });
        if !is_eq {
            return;
        }
        let operands: Vec<_> = node
            .named_children()
            .into_iter()
            .filter(|c| is_expr(c.kind()))
            .collect();
        let any_float = operands.iter().any(|o| type_of(*o, scope) == ValueType::Float);
        if any_float {
            out.push(TypeDiagnostic {
                code: TypeCode::T002,
                inner: m1_core::Diagnostic {
                    range: node.range(),
                    byte_range: node.byte_range(),
                    severity: Severity::Error,
                    code: Code::SyntaxError, // payload code field is unused by T-rules
                    message: "never compare floats with equality operators; use a tolerance check".into(),
                },
            });
        }
    }
}

fn is_expr(k: Kind) -> bool {
    matches!(
        k,
        Kind::Identifier | Kind::MemberExpression | Kind::CallExpression
            | Kind::UnaryExpression | Kind::BinaryExpression | Kind::TernaryExpression
            | Kind::ParenthesizedExpression | Kind::Number | Kind::Boolean | Kind::String
    )
}
```

> The `inner.code` field is `m1_core::Code` (a parse-code enum). T-rules carry
> their identity in `TypeDiagnostic::code`; set `inner.code` to any value
> (`Code::SyntaxError`) — the CLI prints `TypeDiagnostic::code`, never `inner.code`.
> If you prefer, extract this boilerplate `Diagnostic` construction into a small
> `diagnostics::make(code, node, severity, msg)` helper and reuse it in Tasks 9–12.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --test rules t002`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/rules/t002_float_eq.rs tests/rules.rs
git commit -m "Task 8: T002 precise float-equality rule"
```

---

## Task 9: T003 int/float mixing

**Files:** Modify `src/rules/t003_int_float_mix.rs`. Extend `tests/rules.rs`.

- [ ] **Step 1: Add failing tests (append to `tests/rules.rs`)**

```rust
#[test]
fn t003_flags_int_float_mix() {
    assert!(codes("local fX = 1.0;\nlocal iY = 2;\nlocal fZ = fX + iY;\n").contains(&TypeCode::T003));
}

#[test]
fn t003_no_flag_when_converted() {
    // Convert.ToFloat(iY) is a call -> Unknown -> join is Unknown -> no T003
    let src = "local fX = 1.0;\nlocal iY = 2;\nlocal fZ = fX + Convert.ToFloat(iY);\n";
    assert!(!codes(src).contains(&TypeCode::T003));
}

#[test]
fn t003_no_flag_pure_float() {
    assert!(!codes("local fX = 1.0;\nlocal fY = 2.0;\nlocal fZ = fX + fY;\n").contains(&TypeCode::T003));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test rules t003`
Expected: FAIL.

- [ ] **Step 3: Implement `src/rules/t003_int_float_mix.rs`**

```rust
use m1_core::{Code, Kind, Node, Severity};
use crate::diagnostics::{TypeCode, TypeDiagnostic};
use crate::resolve::Scope;
use crate::typer::type_of;
use crate::types::ValueType;

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::BinaryExpression {
            return;
        }
        let is_arith = node.children().iter().any(|c| {
            matches!(c.kind(), Kind::Plus | Kind::Minus | Kind::Star | Kind::Slash | Kind::Percent)
        });
        if !is_arith {
            return;
        }
        let ops: Vec<_> = node.named_children().into_iter().filter(|c| is_expr(c.kind())).collect();
        let (Some(l), Some(r)) = (ops.first(), ops.get(1)) else { return };
        let (lt, rt) = (type_of(*l, scope), type_of(*r, scope));
        let mixed = (lt == ValueType::Float && rt.is_integral())
            || (rt == ValueType::Float && lt.is_integral());
        if mixed {
            out.push(TypeDiagnostic {
                code: TypeCode::T003,
                inner: m1_core::Diagnostic {
                    range: node.range(),
                    byte_range: node.byte_range(),
                    severity: Severity::Warning,
                    code: Code::SyntaxError,
                    message: "mixing integer and float in arithmetic; use Convert.ToFloat/ToInteger".into(),
                },
            });
        }
    }
}

fn is_expr(k: Kind) -> bool {
    matches!(
        k,
        Kind::Identifier | Kind::MemberExpression | Kind::CallExpression
            | Kind::UnaryExpression | Kind::BinaryExpression | Kind::TernaryExpression
            | Kind::ParenthesizedExpression | Kind::Number | Kind::Boolean | Kind::String
    )
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --test rules t003`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/rules/t003_int_float_mix.rs tests/rules.rs
git commit -m "Task 9: T003 int/float mixing rule"
```

---

## Task 10: T010 local-hungarian-prefix + T011 prefix-mismatch

**Files:** Modify `src/rules/t010_local_prefix.rs`, `src/rules/t011_prefix_mismatch.rs`. Extend `tests/rules.rs`.

- [ ] **Step 1: Add failing tests (append to `tests/rules.rs`)**

```rust
#[test]
fn t010_flags_missing_prefix() {
    assert!(codes("local count = 1;\n").contains(&TypeCode::T010));
}
#[test]
fn t010_accepts_valid_prefix() {
    assert!(!codes("local iCount = 1;\n").contains(&TypeCode::T010));
}
#[test]
fn t011_flags_prefix_type_mismatch() {
    // fCount declared float-prefix but initialised with an Integer literal
    assert!(codes("local fCount = 3;\n").contains(&TypeCode::T011));
}
#[test]
fn t011_no_flag_when_matching() {
    assert!(!codes("local fGain = 3.0;\n").contains(&TypeCode::T011));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test rules t01`
Expected: FAIL.

- [ ] **Step 3: Implement `src/rules/t010_local_prefix.rs`**

```rust
use m1_core::{Code, Kind, Node, Severity};
use crate::diagnostics::{TypeCode, TypeDiagnostic};
use crate::resolve::Scope;
use crate::types::type_from_hungarian;

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, _scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::LocalDeclaration {
            return;
        }
        let Some(name_node) = node.named_children().into_iter().find(|c| c.kind() == Kind::Identifier)
        else { return };
        let name = name_node.text();
        if type_from_hungarian(name).is_none() {
            out.push(TypeDiagnostic {
                code: TypeCode::T010,
                inner: m1_core::Diagnostic {
                    range: name_node.range(),
                    byte_range: name_node.byte_range(),
                    severity: Severity::Warning,
                    code: Code::SyntaxError,
                    message: format!("local `{name}` lacks a Hungarian type prefix (b/u/i/f + UpperCase)"),
                },
            });
        }
    }
}
```

- [ ] **Step 4: Implement `src/rules/t011_prefix_mismatch.rs`**

```rust
use m1_core::{Code, Kind, Node, Severity};
use crate::diagnostics::{TypeCode, TypeDiagnostic};
use crate::resolve::Scope;
use crate::typer::type_of;
use crate::types::{type_from_hungarian, ValueType};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::LocalDeclaration {
            return;
        }
        let children = node.named_children();
        let Some(name_node) = children.iter().find(|c| c.kind() == Kind::Identifier) else { return };
        let Some(declared) = type_from_hungarian(name_node.text()) else { return };
        // The initialiser is the expression child after the name (if any).
        let Some(value) = children.iter().rev().find(|c| is_expr(c.kind())) else { return };
        if std::ptr::eq(*value as *const _, *name_node as *const _) {
            return; // no initialiser expression distinct from the name
        }
        let actual = type_of(**value, scope);
        // Treat Integer/Unsigned as compatible with each other (both integral).
        let mismatch = actual.is_known()
            && declared != actual
            && !(declared.is_integral() && actual.is_integral());
        if mismatch {
            out.push(TypeDiagnostic {
                code: TypeCode::T011,
                inner: m1_core::Diagnostic {
                    range: name_node.range(),
                    byte_range: name_node.byte_range(),
                    severity: Severity::Warning,
                    code: Code::SyntaxError,
                    message: format!(
                        "local `{}` prefix implies {:?} but initialiser is {:?}",
                        name_node.text(), declared, actual
                    ),
                },
            });
        }
    }
}

fn is_expr(k: Kind) -> bool {
    matches!(
        k,
        Kind::Identifier | Kind::MemberExpression | Kind::CallExpression
            | Kind::UnaryExpression | Kind::BinaryExpression | Kind::TernaryExpression
            | Kind::ParenthesizedExpression | Kind::Number | Kind::Boolean | Kind::String
    )
}
```

> The `std::ptr::eq` guard is a robustness check; if `Node` equality is awkward,
> instead collect named children, drop the first `Identifier` (the name), and type
> the *next* expression child — i.e. find the initialiser positionally. Use
> whichever is cleaner against `m1-core`'s `Node` API. Both achieve "type the
> initialiser, not the name."

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test --test rules t01`
Expected: PASS (4 tests). Fix the initialiser-selection per the note if T011 mis-targets.

- [ ] **Step 6: Commit**

```bash
git add src/rules/t010_local_prefix.rs src/rules/t011_prefix_mismatch.rs tests/rules.rs
git commit -m "Task 10: T010 missing Hungarian prefix + T011 prefix/type mismatch"
```

---

## Task 11: T001 unresolved-reference

**Files:** Modify `src/rules/t001_unresolved.rs`. Create `tests/resolve_rule.rs`.

- [ ] **Step 1: Write the failing test `tests/resolve_rule.rs`**

```rust
use std::path::Path;
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("mini.m1prj")).unwrap()
}

#[test]
fn t001_flags_project_rooted_miss() {
    let p = proj();
    // "Foo Update.m1scr" maps to group Root.Foo; "Root.Foo.Missing" is project-rooted but absent.
    let src = "Root.Foo.Missing = 1;\n";
    let codes: Vec<_> = check_script(&p, Path::new("Foo Update.m1scr"), src)
        .diagnostics.iter().map(|d| d.code).collect();
    assert!(codes.contains(&TypeCode::T001));
}

#[test]
fn t001_no_flag_for_known_symbol_or_library() {
    let p = proj();
    let src = "Speed = Calculate.Min(1, 2);\n"; // Speed resolves (group-relative); Calculate is opaque
    let codes: Vec<_> = check_script(&p, Path::new("Foo Update.m1scr"), src)
        .diagnostics.iter().map(|d| d.code).collect();
    assert!(!codes.contains(&TypeCode::T001));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test resolve_rule`
Expected: FAIL.

- [ ] **Step 3: Implement `src/rules/t001_unresolved.rs`**

```rust
use m1_core::{Code, Kind, Node, Severity};
use crate::diagnostics::{TypeCode, TypeDiagnostic};
use crate::resolve::{resolve, Resolution, Scope};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        // Only check a member-expression or bare identifier that is NOT the
        // `.property` child of a larger member-expression (avoid double-checking
        // every prefix). Strategy: check a node whose parent is not a
        // MemberExpression, so we evaluate each full path once.
        if !matches!(node.kind(), Kind::Identifier | Kind::MemberExpression) {
            return;
        }
        if let Some(parent) = node.parent() {
            if parent.kind() == Kind::MemberExpression {
                return; // part of a longer path; the outer node carries the full path
            }
            // Skip the type name inside `<Type>` annotations and the property side.
            if parent.kind() == Kind::TypeAnnotation {
                return;
            }
        }
        if matches!(resolve(&node.text().to_string(), scope), Resolution::Unresolved) {
            out.push(TypeDiagnostic {
                code: TypeCode::T001,
                inner: m1_core::Diagnostic {
                    range: node.range(),
                    byte_range: node.byte_range(),
                    severity: Severity::Warning,
                    code: Code::SyntaxError,
                    message: format!("unresolved reference `{}`", node.text()),
                },
            });
        }
    }
}
```

> If `check_script`'s walk visits a `MemberExpression` and then its inner
> `MemberExpression`/`Identifier` children, the parent-guard above ensures only
> the outermost path node is resolution-checked. Verify against a 3-segment path
> (`Root.Foo.Missing`) that exactly one T001 is produced (the test asserts
> `contains`, but add a debug check that it is not emitted three times — if it is,
> the guard logic needs the parent check tightened to "parent is MemberExpression
> AND this node is its object child").

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --test resolve_rule`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/rules/t001_unresolved.rs tests/resolve_rule.rs
git commit -m "Task 11: T001 unresolved-reference (project-rooted misses only)"
```

---

## Task 12: CLI

**Files:** Modify `Cargo.toml` (add `clap`), `src/main.rs`.

- [ ] **Step 1: Add `clap`**

In `Cargo.toml` `[dependencies]`: `clap = { version = "4", features = ["derive"] }`

- [ ] **Step 2: Implement `src/main.rs`**

```rust
use std::path::PathBuf;
use std::process;
use clap::Parser;
use m1_core::Severity;
use m1_typecheck::project::Project;
use m1_typecheck::rules::{check_script, check_script_no_project};

#[derive(Parser, Debug)]
#[command(name = "m1-typecheck", about = "Type checker for MoTeC M1 scripts")]
struct Args {
    /// Scripts to check
    files: Vec<PathBuf>,
    /// Project.m1prj (defaults to nearest upward, or $M1_PROJECT)
    #[arg(long)]
    project: Option<PathBuf>,
    /// parameters.m1cfg (optional)
    #[arg(long)]
    config: Option<PathBuf>,
}

fn find_project(args: &Args) -> Option<PathBuf> {
    if let Some(p) = &args.project { return Some(p.clone()); }
    if let Some(p) = std::env::var_os("M1_PROJECT") { return Some(PathBuf::from(p)); }
    let start = args.files.first()?;
    let mut dir = start.parent()?;
    loop {
        let cand = dir.join("Project.m1prj");
        if cand.is_file() { return Some(cand); }
        dir = dir.parent()?;
    }
}

fn main() {
    let args = Args::parse();
    let mut had_error = false;

    let project = find_project(&args).and_then(|path| {
        match Project::load(&path) {
            Ok(mut p) => {
                if let Some(cfg) = &args.config {
                    p = p.with_config(cfg).unwrap_or_else(|e| {
                        eprintln!("m1-typecheck: config {}: {e}", cfg.display());
                        process::exit(2);
                    });
                }
                Some(p)
            }
            Err(e) => { eprintln!("m1-typecheck: project {}: {e}", path.display()); process::exit(2); }
        }
    });
    if project.is_none() {
        eprintln!("m1-typecheck: no project found; running in project-less mode (T001 disabled)");
    }

    for file in &args.files {
        let src = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => { eprintln!("m1-typecheck: {}: {e}", file.display()); had_error = true; continue; }
        };
        let result = match &project {
            Some(p) => check_script(p, file, &src),
            None => check_script_no_project(&src),
        };
        for d in &result.syntax_errors {
            println!("{}:{}:{}: error[syntax]: {}",
                file.display(), d.range.start.line + 1, d.range.start.column + 1, d.message);
            had_error = true;
        }
        for d in &result.diagnostics {
            let sev = match d.inner.severity {
                Severity::Error => { had_error = true; "error" }
                Severity::Warning => "warning",
                Severity::Info => "info",
                Severity::Hint => "hint",
            };
            println!("{}:{}:{}: {}[{}]: {}",
                file.display(), d.inner.range.start.line + 1, d.inner.range.start.column + 1,
                sev, d.code.as_str(), d.inner.message);
        }
    }

    if had_error { process::exit(1); }
}
```

- [ ] **Step 3: Smoke test**

Run:
```bash
cargo build --release
printf 'local count = 1;\n' > /tmp/t.m1scr
./target/release/m1-typecheck /tmp/t.m1scr
```
Expected: a line like `/tmp/t.m1scr:1:7: warning[T010]: local \`count\` lacks a Hungarian type prefix ...` and exit 0.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml src/main.rs
git commit -m "Task 12: CLI with --project/--config, human output, exit codes"
```

---

## Task 13: Corpus gate + polish

**Files:** Create `tests/corpus.rs`. Create `README.md`.

- [ ] **Step 1: Write `tests/corpus.rs`**

```rust
//! Loads the example project and checks every script: must not panic. Skipped
//! unless both the project and corpus are present. Records a baseline count.
use std::path::{Path, PathBuf};
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;

fn project_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("M1_PROJECT") { return Some(PathBuf::from(p)); }
    let cand = Path::new(env!("CARGO_MANIFEST_DIR")).join("../EV-M1/UQR-EV/01.00/Project.m1prj");
    cand.is_file().then_some(cand)
}
fn corpus_dir() -> PathBuf {
    match std::env::var_os("M1_CORPUS_PATH") {
        Some(p) => PathBuf::from(p),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../EV-M1/UQR-EV/01.00/Scripts"),
    }
}

#[test]
fn checks_corpus_without_panicking() {
    let (Some(proj_path), dir) = (project_path(), corpus_dir()) else { return };
    if !dir.is_dir() { eprintln!("corpus absent; skipping"); return; }
    let project = Project::load(&proj_path).expect("load project");
    let mut checked = 0;
    let mut total_diags = 0usize;
    for entry in std::fs::read_dir(&dir).expect("read corpus") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("m1scr") { continue; }
        let src = std::fs::read_to_string(&path).expect("read");
        let r = check_script(&project, &path, &src);
        total_diags += r.diagnostics.len();
        checked += 1;
    }
    assert!(checked > 0, "no scripts found");
    eprintln!("checked {checked} scripts, {total_diags} type diagnostics");
}
```

- [ ] **Step 2: Run the full suite + lints**

Run: `cargo test`
Expected: all unit/integration tests pass; the corpus test runs or skips.

Run: `cargo clippy -- -D warnings`
Expected: zero warnings (fix any; remove the `_path_marker`/`_` placeholders if clippy objects).

Run: `cargo fmt --check`
Expected: clean.

- [ ] **Step 3: Write `README.md`**

Summarise: purpose, the symbol model, the five rules, CLI usage (`m1-typecheck
--project Project.m1prj Scripts/*.m1scr`), the `M1_PROJECT`/`M1_CORPUS_PATH` env
vars, and the obfuscation note (example identifiers are synthetic placeholders).

- [ ] **Step 4: Commit**

```bash
git add tests/corpus.rs README.md
git commit -m "Task 13: corpus gate, clippy/fmt clean, README"
```

---

## Deferred to v2

| Item | Note |
|------|------|
| Assignment type-compatibility | Needs reliable channel value types |
| Enum-domain checks (`is`/compares) | Needs per-channel enum association |
| Function/method return types | Calls stay `Unknown` in v1 |
| Single-assignment-per-channel | Needs control-flow analysis |
| Absolute-path-reference enforcement | v2 |
| Project-wide symbol-name auditing | "Lint the .m1prj" feature; v2 |
| Promote symbol model into `m1-core` | When a second consumer needs it (m1-lsp) |
| `--format json` | After human output proves out |

---

## Summary of tasks

| # | Title |
|---|-------|
| 0 | Crate bootstrap |
| 1 | types.rs (ValueType, literals, join, Hungarian) |
| 2 | SymbolTable + .m1prj parser |
| 3 | .m1cfg augmentation |
| 4 | Project::load + opaque roots |
| 5 | resolve.rs |
| 6 | typer.rs |
| 7 | diagnostics + Rule/Registry/Runner |
| 8 | T002 float-equality |
| 9 | T003 int/float mixing |
| 10 | T010/T011 local naming |
| 11 | T001 unresolved-reference |
| 12 | CLI |
| 13 | Corpus gate + polish |
