# m1-typecheck v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Note:** Example identifiers below are synthetic placeholders, not from any
> real project. The example project/corpus is resolved via `M1_PROJECT` /
> `M1_CORPUS_PATH` (falling back to the sibling EV-M1 example project).

**Goal:** Give `m1-typecheck` a live enum domain model (member index + enum
association + enum typing), the three semantic rules it unlocks (T020 enum-non-
member, T021 enum/numeric comparison, T030 assignment-type-mismatch), a control-
flow single-assignment pass (T040), and an opt-in project-wide name audit (T050) —
building on the existing v1 `src/` with no rewrites.

**Architecture:** Enum data already parses into `SymbolTable::enums` (v1, dead).
v2 adds a `member_index` and `enum_*` queries, back-resolves `.m1cfg` enum cells
to associate parameters, and teaches `typer::type_of` to produce `ValueType::Enum`.
Three new node-at-a-time `Rule`s join the registry (`default_v2`); a whole-tree
`flow::single_assignment` pass merges per-path assignment facts; `audit::audit_project`
lints the `.m1prj`'s own names. Every rule stays silent under `Unknown`.

**Tech Stack:** Rust 2021, `m1-core` (path dep), `roxmltree` 0.20, `clap` 4.
(No new dependencies.)

**Spec:** `docs/superpowers/specs/2026-05-31-m1-typecheck-v2-design.md`

**Prerequisites:** v1 complete (commit `d51f3d5` "Task 13: corpus gate…"). Rust
≥ 1.78; `m1-core` at `../m1-core`. Example project at
`../EV-M1/UQR-EV/01.00/Project.m1prj` (or `$M1_PROJECT`); corpus via
`$M1_CORPUS_PATH`. The corpus gate is skipped if absent.

---

## File structure (additions / edits)

| File | Responsibility | New? |
|------|----------------|------|
| `src/symbols/mod.rs` | `member_index` + `enum_by_name`/`enums_with_member`/`enum_has_member`/`enum_type`; `Symbol.enum_assoc` field | edit |
| `src/symbols/m1prj.rs` | build `member_index` after parsing enums | edit |
| `src/symbols/m1cfg.rs` | enum-cell back-resolution → set `Symbol.enum_assoc` | edit |
| `src/types.rs` | (Enum present already) — no change beyond a `is_enum()` helper | edit |
| `src/typer.rs` | typed-member-path → `Enum`; enum-associated symbol → `Enum` | edit |
| `src/diagnostics.rs` | `TypeCode::T020/T021/T030/T040/T050` + `as_str` | edit |
| `src/rules/mod.rs` | `default_v2()`; runner calls `flow::single_assignment` | edit |
| `src/rules/t020_enum_member.rs` | T020 enum-non-member | NEW |
| `src/rules/t021_enum_numeric_cmp.rs` | T021 enum/numeric comparison | NEW |
| `src/rules/t030_assign_mismatch.rs` | T030 assignment-type-mismatch | NEW |
| `src/flow.rs` | single-assignment control-flow pass (T040) | NEW |
| `src/audit.rs` | `audit_project` + per-kind name predicates (T050) | NEW |
| `src/project.rs` | `Project::audit()` | edit |
| `src/lib.rs` | `pub mod flow; pub mod audit;` | edit |
| `src/main.rs` | `--audit-names` | edit |
| `tests/enums.rs`, `tests/typer_enum.rs`, `tests/rules_v2.rs`, `tests/flow.rs`, `tests/audit.rs` | v2 tests | NEW |
| `tests/fixtures/enums.m1prj`, `enums.m1cfg`, `naming.m1prj` | v2 fixtures | NEW |

---

## Task 1: Enum member index + queries on SymbolTable

**Files:** Modify `src/symbols/mod.rs`, `src/symbols/m1prj.rs`. Create `tests/fixtures/enums.m1prj`, `tests/enums.rs`.

- [ ] **Step 1: Write the fixture `tests/fixtures/enums.m1prj`**

```xml
<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="Enums" TargetHardware="ecu120">
  <SelectedModuleSets>
   <File Name="MoTeC Comms"/>
  </SelectedModuleSets>
  <DataTypes>
   <Type Name="Drive State" Storage="enum" Default="Idle">
    <Enum Name="Idle" ContainerOrder="0"/>
    <Enum Name="Latching Fault" ContainerOrder="-1"/>
    <Enum Name="Ready To Drive" ContainerOrder="3"/>
   </Type>
   <Type Name="Switch State" Storage="enum" Default="Off">
    <Enum Name="Off" ContainerOrder="0"/>
    <Enum Name="On" ContainerOrder="1"/>
   </Type>
   <Type Name="Other State" Storage="enum" Default="Off">
    <Enum Name="Off" ContainerOrder="0"/>
    <Enum Name="Active" ContainerOrder="1"/>
   </Type>
  </DataTypes>
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Foo"/>
    <Component Classname="BuiltIn.Channel" Name="Root.Foo.driveMode"><Props Security="Tune"/></Component>
    <Component Classname="BuiltIn.Parameter" Name="Root.Foo.SwitchMode.Value"><Props/></Component>
    <Component Classname="BuiltIn.FuncUser" Filename="Foo Update.m1scr" Name="Root.Foo.Update"/>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
```

(`Off` is intentionally a member of both `Switch State` and `Other State` —
the ambiguous-member case.)

- [ ] **Step 2: Add the `enum_assoc` field + index to `src/symbols/mod.rs`**

Edit `Symbol` to add an association slot, and `SymbolTable` to add the index:

```rust
#[derive(Debug, Clone)]
pub struct Symbol {
    pub path: String,
    pub kind: SymbolKind,
    pub value_type: ValueType,
    pub unit: Option<String>,
    pub filename: Option<String>,
    /// Enum type this symbol's value belongs to, if known (set during .m1cfg
    /// back-resolution). When set, the typer reports `ValueType::Enum(_)`.
    pub enum_assoc: Option<EnumId>,
}
```

```rust
#[derive(Debug, Default)]
pub struct SymbolTable {
    symbols: Vec<Symbol>,
    by_path: HashMap<String, usize>,
    enums: Vec<EnumType>,
    /// member name -> enum type ids that declare a member of that name.
    member_index: HashMap<String, Vec<EnumId>>,
}
```

Add queries (after the existing `enums()` method) and rebuild the index when an
enum is added:

```rust
    pub fn add_enum(&mut self, e: EnumType) -> EnumId {
        let id = self.enums.len();
        for (member, _order) in &e.members {
            self.member_index.entry(member.clone()).or_default().push(id);
        }
        self.enums.push(e);
        id
    }

    /// The enum type whose *name* equals `name`, if any.
    pub fn enum_by_name(&self, name: &str) -> Option<EnumId> {
        self.enums.iter().position(|e| e.name == name)
    }
    /// Enum type ids that declare a member of this exact name.
    pub fn enums_with_member(&self, member: &str) -> &[EnumId] {
        self.member_index.get(member).map(|v| v.as_slice()).unwrap_or(&[])
    }
    pub fn enum_has_member(&self, id: EnumId, member: &str) -> bool {
        self.enums.get(id).map(|e| e.members.iter().any(|(m, _)| m == member)).unwrap_or(false)
    }
    pub fn enum_type(&self, id: EnumId) -> &EnumType {
        &self.enums[id]
    }
```

Update **every** `Symbol { … }` construction to set `enum_assoc: None`
(grep: `Symbol {` appears in `src/symbols/m1prj.rs` only). Do it in this task's
m1prj edit (Step 4).

- [ ] **Step 3: Write the failing test `tests/enums.rs`**

```rust
use std::path::Path;
use m1_typecheck::symbols::m1prj;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name),
    ).unwrap()
}

#[test]
fn enum_by_name_and_members() {
    let parsed = m1prj::parse(&fixture("enums.m1prj")).unwrap();
    let t = &parsed.table;
    let id = t.enum_by_name("Drive State").expect("Drive State enum");
    assert!(t.enum_has_member(id, "Latching Fault"));
    assert!(!t.enum_has_member(id, "Nope"));
    assert!(t.enum_by_name("No Such Type").is_none());
}

#[test]
fn member_index_unique_and_ambiguous() {
    let parsed = m1prj::parse(&fixture("enums.m1prj")).unwrap();
    let t = &parsed.table;
    // "On" is only in Switch State -> unique.
    assert_eq!(t.enums_with_member("On").len(), 1);
    // "Off" is in both Switch State and Other State -> ambiguous.
    assert_eq!(t.enums_with_member("Off").len(), 2);
    // unknown member -> empty.
    assert!(t.enums_with_member("Zzz").is_empty());
}
```

- [ ] **Step 4: Run to verify it fails, then implement**

Run: `cargo test --test enums`
Expected: FAIL — `enum_by_name`/`enums_with_member`/`enum_has_member` undefined.

Implement per Step 2. In `src/symbols/m1prj.rs`, set `enum_assoc: None` on the
`Symbol { … }` literal:

```rust
                table.push(Symbol {
                    path: name.to_string(),
                    kind,
                    value_type,
                    unit: None,
                    filename,
                    enum_assoc: None,
                });
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test --test enums`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add src/symbols/mod.rs src/symbols/m1prj.rs tests/enums.rs tests/fixtures/enums.m1prj
git commit -m "Task 1: enum member index + enum_by_name/has_member queries; Symbol.enum_assoc"
```

---

## Task 2: .m1cfg enum-cell back-resolution → enum association

**Files:** Modify `src/symbols/m1cfg.rs`, `src/symbols/mod.rs` (a setter). Create `tests/fixtures/enums.m1cfg`. Extend `tests/enums.rs`.

- [ ] **Step 1: Write `tests/fixtures/enums.m1cfg`**

```xml
<?xml version="1.0"?>
<Configuration>
 <Group Name="">
  <Parameter Name="Root.Foo.SwitchMode.Value"><Cell Type="enum"><![CDATA[On]]></Cell></Parameter>
 </Group>
</Configuration>
```

(`On` is unique to `Switch State`, so this must associate the parameter with it.)

- [ ] **Step 2: Add a setter to `src/symbols/mod.rs`**

```rust
    /// Associate a symbol (by path) with an enum type id. No-op if absent.
    pub fn set_enum_assoc(&mut self, path: &str, id: EnumId) {
        if let Some(&i) = self.by_path.get(path) {
            self.symbols[i].enum_assoc = Some(id);
            self.symbols[i].value_type = ValueType::Enum(id);
        }
    }
```

- [ ] **Step 3: Write the failing test (append to `tests/enums.rs`)**

```rust
#[test]
fn m1cfg_enum_cell_associates_unique_member() {
    use m1_typecheck::symbols::m1cfg;
    use m1_typecheck::types::ValueType;
    let mut parsed = m1prj::parse(&fixture("enums.m1prj")).unwrap();
    m1cfg::augment(&mut parsed.table, &fixture("enums.m1cfg")).unwrap();
    let sym = parsed.table.get("Root.Foo.SwitchMode.Value").unwrap();
    let switch_id = parsed.table.enum_by_name("Switch State").unwrap();
    assert_eq!(sym.enum_assoc, Some(switch_id));
    assert_eq!(sym.value_type, ValueType::Enum(switch_id));
}
```

- [ ] **Step 4: Run to verify it fails**

Run: `cargo test --test enums m1cfg_enum`
Expected: FAIL — cell still maps to `Unknown` (v1 behaviour).

- [ ] **Step 5: Implement in `src/symbols/m1cfg.rs`**

Replace the v1 `cell_type("enum") => Unknown` path with back-resolution. The
parameter's enum *value* lives in the `Cell`'s CDATA text; resolve it to a unique
enum type. Rework `augment` so it can read the cell text and consult the table's
member index (do the enum lookup **before** taking the `&mut` borrow to avoid a
borrow conflict):

```rust
pub fn augment(table: &mut SymbolTable, xml: &str) -> Result<(), ConfigError> {
    let doc = roxmltree::Document::parse(xml).map_err(ConfigError::Xml)?;
    for param in doc.descendants().filter(|n| n.has_tag_name("Parameter")) {
        let Some(name) = param.attribute("Name") else { continue };
        let Some(cell) = param.children().find(|c| c.has_tag_name("Cell")) else { continue };
        let type_attr = cell.attribute("Type").unwrap_or("");
        let unit = cell.attribute("Unit").map(str::to_string);

        if type_attr == "enum" {
            // Back-resolve the cell's member value to a *unique* enum type.
            let member = cell.text().map(str::trim).unwrap_or("");
            let candidates = table.enums_with_member(member);
            if candidates.len() == 1 {
                let id = candidates[0];
                table.set_enum_assoc(name, id);
            }
            // zero or many -> leave Unknown (silent), but still record the unit.
            if let Some(sym) = table.get_mut(name) {
                if sym.unit.is_none() { sym.unit = unit; }
            }
            continue;
        }

        let vt = cell_type(type_attr);
        if let Some(sym) = table.get_mut(name) {
            sym.value_type = vt;
            sym.unit = unit;
        }
    }
    Ok(())
}
```

Leave `cell_type` as-is for the non-enum types (its `"enum" => Unknown` arm is now
unreachable from `augment` but is kept for callers/tests; that is fine).

- [ ] **Step 6: Run to verify it passes**

Run: `cargo test --test enums && cargo test --test symbols`
Expected: PASS (enum association works; v1 `m1cfg_augments_parameter_types`
still passes — `f32` path unchanged).

- [ ] **Step 7: Commit**

```bash
git add src/symbols/m1cfg.rs src/symbols/mod.rs tests/enums.rs tests/fixtures/enums.m1cfg
git commit -m "Task 2: back-resolve .m1cfg enum cells to a unique enum type association"
```

---

## Task 3: Typer — produce ValueType::Enum

**Files:** Modify `src/typer.rs`, `src/types.rs`. Create `tests/typer_enum.rs`.

- [ ] **Step 1: Add `is_enum` to `src/types.rs`**

```rust
    pub fn is_enum(self) -> bool { matches!(self, ValueType::Enum(_)) }
```

- [ ] **Step 2: Write the failing test `tests/typer_enum.rs`**

```rust
use std::collections::HashMap;
use std::path::Path;
use m1_typecheck::project::Project;
use m1_typecheck::resolve::Scope;
use m1_typecheck::types::ValueType;
use m1_typecheck::typer::type_of_source_expr;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("enums.m1prj")).unwrap()
        .with_config(&dir.join("enums.m1cfg")).unwrap()
}

fn ty(p: &Project, expr: &str) -> ValueType {
    let scope = Scope { locals: HashMap::new(), group: Some("Root.Foo".into()), project: Some(p) };
    type_of_source_expr(&format!("{expr};\n"), &scope)
}

#[test]
fn typed_member_path_is_enum() {
    let p = proj();
    let drive_id = p.symbols().enum_by_name("Drive State").unwrap();
    assert_eq!(ty(&p, "Drive State.Latching Fault"), ValueType::Enum(drive_id));
}

#[test]
fn non_member_typed_path_is_not_that_enum() {
    let p = proj();
    // "Drive State.Nope" has the enum-type head but no such member -> not Enum.
    assert!(!ty(&p, "Drive State.Nope").is_enum());
}

#[test]
fn associated_parameter_is_enum() {
    let p = proj();
    let switch_id = p.symbols().enum_by_name("Switch State").unwrap();
    // Group-relative "SwitchMode.Value" resolves to the associated parameter.
    assert_eq!(ty(&p, "SwitchMode.Value"), ValueType::Enum(switch_id));
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test --test typer_enum`
Expected: FAIL — typed member path types as `Unknown`.

- [ ] **Step 4: Implement in `src/typer.rs`**

Add an enum check in the `Identifier | MemberExpression` arm, **before** falling
through to `resolve`. The associated-symbol case is already covered because
`set_enum_assoc` sets `value_type = Enum(id)`, so `Resolution::Symbol(s) =>
s.value_type` returns it for free — only the typed-member-path needs new code:

```rust
        Kind::Identifier | Kind::MemberExpression => {
            let path = path_text(node);
            // Typed enum-member path: `<EnumTypeName>.<Member>` -> Enum(id).
            if let Some(p) = scope.project {
                if let Some((head, member)) = path.rsplit_once('.') {
                    if let Some(id) = p.symbols().enum_by_name(head) {
                        if p.symbols().enum_has_member(id, member) {
                            return ValueType::Enum(id);
                        }
                    }
                }
            }
            match resolve(&path, scope) {
                Resolution::Local(t) => t,
                Resolution::Symbol(s) => s.value_type,
                Resolution::Opaque | Resolution::Unresolved => ValueType::Unknown,
            }
        }
```

(Note: `rsplit_once('.')` splits at the *last* dot, so `head = "Drive State"`,
`member = "Latching Fault"` — correct for the single-`.` type-name-plus-member
idiom seen in the corpus.)

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test --test typer_enum && cargo test --test typer`
Expected: PASS (3 new; v1 typer tests unaffected — none use a project with enum
type names colliding with member paths).

- [ ] **Step 6: Commit**

```bash
git add src/typer.rs src/types.rs tests/typer_enum.rs
git commit -m "Task 3: typer produces ValueType::Enum for typed member paths and associated symbols"
```

---

## Task 4: diagnostics — new TypeCodes

**Files:** Modify `src/diagnostics.rs`.

- [ ] **Step 1: Extend `TypeCode` and `as_str`**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCode {
    T001, T002, T003, T010, T011, // v1
    T020, // enum-non-member
    T021, // enum-numeric-comparison
    T030, // assignment-type-mismatch
    T040, // channel-multiple-assignment
    T050, // symbol-name-convention (project audit)
}

impl TypeCode {
    pub fn as_str(self) -> &'static str {
        match self {
            TypeCode::T001 => "T001",
            TypeCode::T002 => "T002",
            TypeCode::T003 => "T003",
            TypeCode::T010 => "T010",
            TypeCode::T011 => "T011",
            TypeCode::T020 => "T020",
            TypeCode::T021 => "T021",
            TypeCode::T030 => "T030",
            TypeCode::T040 => "T040",
            TypeCode::T050 => "T050",
        }
    }
}
```

Keep the existing `make(code, node, severity, message)` helper — the new per-script
rules reuse it. For T050 (no `.m1scr` node), add a node-less constructor:

```rust
use m1_core::{Position, Range};

/// Build a project-level `TypeDiagnostic` (no source node; zero range).
pub fn make_project(code: TypeCode, severity: Severity, message: String) -> TypeDiagnostic {
    let zero = Position { line: 0, column: 0 };
    TypeDiagnostic {
        code,
        inner: Diagnostic {
            range: Range { start: zero, end: zero },
            byte_range: 0..0,
            severity,
            code: Code::SyntaxError,
            message,
        },
    }
}
```

(`m1_core::Range` is `{ start: Position, end: Position }` and `Position` is
`{ line: u32, column: u32 }`; neither derives `Default` — verified in
`m1-core/src/diagnostic.rs` — so the zero range is constructed explicitly above.
Confirm `Position`/`Range` are re-exported from `m1_core`'s crate root; if they
are only `m1_core::diagnostic::{Position, Range}`, import from there.)

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: compiles (the `match` in `main.rs` over `TypeCode` is via `as_str`, so
no exhaustiveness break).

- [ ] **Step 3: Commit**

```bash
git add src/diagnostics.rs
git commit -m "Task 4: add TypeCode T020/T021/T030/T040/T050 + make_project helper"
```

---

## Task 5: T020 enum-non-member

**Files:** Create `src/rules/t020_enum_member.rs`. Modify `src/rules/mod.rs`. Create `tests/rules_v2.rs`.

- [ ] **Step 1: Write the failing test `tests/rules_v2.rs`**

```rust
use std::path::Path;
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("enums.m1prj")).unwrap()
        .with_config(&dir.join("enums.m1cfg")).unwrap()
}

fn codes(p: &Project, src: &str) -> Vec<TypeCode> {
    check_script(p, Path::new("Foo Update.m1scr"), src)
        .diagnostics.iter().map(|d| d.code).collect()
}

#[test]
fn t020_flags_non_member_typed_path() {
    let p = proj();
    // "Drive State.Nope" has the enum-type head but no such member.
    assert!(codes(&p, "driveMode = Drive State.Nope;\n").contains(&TypeCode::T020));
}

#[test]
fn t020_no_flag_for_real_member() {
    let p = proj();
    assert!(!codes(&p, "driveMode = Drive State.Idle;\n").contains(&TypeCode::T020));
}

#[test]
fn t020_flags_is_clause_non_member() {
    let p = proj();
    // SwitchMode.Value is associated with Switch State {Off, On}; "Active" is not a member.
    assert!(codes(&p, "if (SwitchMode.Value is (Active)) {\n}\n").contains(&TypeCode::T020));
}

#[test]
fn t020_no_flag_is_clause_real_member() {
    let p = proj();
    assert!(!codes(&p, "if (SwitchMode.Value is (On)) {\n}\n").contains(&TypeCode::T020));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test rules_v2 t020`
Expected: FAIL (rule not registered).

- [ ] **Step 3: Implement `src/rules/t020_enum_member.rs`**

Two triggers: a typed member path whose head is an enum type but member is absent;
and an `IsClause` whose LHS is a known enum and whose member is not in it.

```rust
use m1_core::{Kind, Node, Severity};
use crate::diagnostics::{make, TypeCode, TypeDiagnostic};
use crate::resolve::Scope;
use crate::typer::{path_text, type_of};
use crate::types::ValueType;

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let Some(project) = scope.project else { return };
        let table = project.symbols();
        match node.kind() {
            // Trigger 1: `<EnumTypeName>.<Member>` with a head that names an enum
            // type but a member that the enum does not declare.
            Kind::MemberExpression => {
                if matches!(node.parent().map(|p| p.kind()), Some(Kind::MemberExpression)) {
                    return; // only the outermost path
                }
                let path = path_text(*node);
                if let Some((head, member)) = path.rsplit_once('.') {
                    if let Some(id) = table.enum_by_name(head) {
                        if !table.enum_has_member(id, member) {
                            out.push(make(
                                TypeCode::T020, node, Severity::Warning,
                                format!("`{member}` is not a member of enum `{head}`"),
                            ));
                        }
                    }
                }
            }
            // Trigger 2: `LHS is (Member)` where LHS is a known enum.
            Kind::IsClause => {
                let kids = node.named_children();
                let Some(lhs) = kids.iter().find(|c| is_expr(c.kind())) else { return };
                // The member name is the identifier inside the parentheses.
                let Some(member_node) = kids.iter().rev().find(|c| c.kind() == Kind::Identifier)
                else { return };
                let member = member_node.text();
                if let ValueType::Enum(id) = type_of(*lhs, scope) {
                    if !table.enum_has_member(id, member) {
                        out.push(make(
                            TypeCode::T020, node, Severity::Warning,
                            format!(
                                "`{member}` is not a member of enum `{}`",
                                table.enum_type(id).name
                            ),
                        ));
                    }
                }
            }
            _ => {}
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

> **Verify the `IsClause` shape against `m1-core`.** `Kind::IsClause` and `Kind::Is`
> exist (confirmed in `m1-core/src/kind.rs`). Parse `X is (Active)` and inspect the
> `IsClause` node's `named_children()` to confirm the LHS expression and the
> member `Identifier` are both named children (the `is`, `(`, `)` are anonymous).
> If the LHS is *outside* the `IsClause` (i.e. the clause wraps only `(Active)` and
> the binary `X is (…)` is a `BinaryExpression`), switch Trigger 2 to fire on the
> enclosing node and read the LHS from its first operand. Adjust to the real tree;
> the test fixtures above pin the intended behaviour.

- [ ] **Step 4: Register in `src/rules/mod.rs`**

Add `pub mod t020_enum_member;` and a `default_v2()` that includes the v1 rules
plus the new ones (keep `default_v1()` intact):

```rust
pub mod t020_enum_member;

impl Registry {
    pub fn default_v2() -> Self {
        Registry {
            rules: vec![
                Box::new(t001_unresolved::Rule),
                Box::new(t002_float_eq::Rule),
                Box::new(t003_int_float_mix::Rule),
                Box::new(t010_local_prefix::Rule),
                Box::new(t011_prefix_mismatch::Rule),
                Box::new(t020_enum_member::Rule),
            ],
        }
    }
}
```

And switch the runner's `Registry::default_v1()` call to `Registry::default_v2()`
in `run`. (Tasks 6 and 7 append `t021`/`t030` to `default_v2`.)

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test --test rules_v2 t020`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add src/rules/t020_enum_member.rs src/rules/mod.rs tests/rules_v2.rs
git commit -m "Task 5: T020 enum-non-member (typed member path + is-clause), default_v2 registry"
```

---

## Task 6: T021 enum/numeric comparison

**Files:** Create `src/rules/t021_enum_numeric_cmp.rs`. Modify `src/rules/mod.rs`. Extend `tests/rules_v2.rs`.

- [ ] **Step 1: Add failing tests (append to `tests/rules_v2.rs`)**

```rust
#[test]
fn t021_flags_enum_vs_integer() {
    let p = proj();
    // SwitchMode.Value is enum; comparing to integer literal 1.
    assert!(codes(&p, "if (SwitchMode.Value eq 1) {\n}\n").contains(&TypeCode::T021));
}

#[test]
fn t021_no_flag_enum_vs_member() {
    let p = proj();
    assert!(!codes(&p, "if (SwitchMode.Value eq Switch State.On) {\n}\n").contains(&TypeCode::T021));
}

#[test]
fn t021_no_flag_int_vs_int() {
    let p = proj();
    assert!(!codes(&p, "local iX = 1;\nif (iX eq 2) {\n}\n").contains(&TypeCode::T021));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test rules_v2 t021`
Expected: FAIL.

- [ ] **Step 3: Implement `src/rules/t021_enum_numeric_cmp.rs`**

```rust
use m1_core::{Kind, Node, Severity};
use crate::diagnostics::{make, TypeCode, TypeDiagnostic};
use crate::resolve::Scope;
use crate::typer::type_of;
use crate::types::ValueType;

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::BinaryExpression {
            return;
        }
        let is_cmp = node.children().iter().any(|c| {
            matches!(
                c.kind(),
                Kind::EqEq | Kind::BangEq | Kind::Eq | Kind::Neq
                    | Kind::Lt | Kind::Gt | Kind::LtEq | Kind::GtEq
            )
        });
        if !is_cmp {
            return;
        }
        let ops: Vec<_> = node.named_children().into_iter().filter(|c| is_expr(c.kind())).collect();
        let (Some(l), Some(r)) = (ops.first(), ops.get(1)) else { return };
        let (lt, rt) = (type_of(*l, scope), type_of(*r, scope));
        let enum_vs_num = (lt.is_enum() && is_numeric(rt)) || (rt.is_enum() && is_numeric(lt));
        if enum_vs_num {
            out.push(make(
                TypeCode::T021, node, Severity::Warning,
                "comparing an enum to a number; compare against an enum member instead".into(),
            ));
        }
    }
}

fn is_numeric(t: ValueType) -> bool {
    matches!(t, ValueType::Integer | ValueType::Unsigned | ValueType::Float)
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

- [ ] **Step 4: Register** — add `pub mod t021_enum_numeric_cmp;` and
`Box::new(t021_enum_numeric_cmp::Rule)` to `default_v2`'s vec.

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test --test rules_v2 t021`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add src/rules/t021_enum_numeric_cmp.rs src/rules/mod.rs tests/rules_v2.rs
git commit -m "Task 6: T021 enum/numeric comparison rule"
```

---

## Task 7: T030 assignment-type-mismatch

**Files:** Create `src/rules/t030_assign_mismatch.rs`. Modify `src/rules/mod.rs`. Extend `tests/rules_v2.rs`.

- [ ] **Step 1: Add failing tests (append to `tests/rules_v2.rs`)**

The `enums.m1prj` channel `driveMode` has no value type (Unknown). Add a typed
channel to the fixture for this rule. Extend `tests/fixtures/enums.m1prj`'s `<List>`
with a float channel and give it a `.m1cfg` type:

In `tests/fixtures/enums.m1prj` add inside `<List>`:
```xml
    <Component Classname="BuiltIn.Channel" Name="Root.Foo.gain"><Props Security="Tune"/></Component>
```
In `tests/fixtures/enums.m1cfg` add inside `<Group Name="">`:
```xml
  <Parameter Name="Root.Foo.gain"><Cell Type="f32" Unit="ratio"><![CDATA[1.0]]></Cell></Parameter>
```

(`.m1cfg` augments any matching symbol regardless of kind, so `gain` gets
`Float`.)

```rust
#[test]
fn t030_flags_enum_member_into_wrong_enum() {
    let p = proj();
    // SwitchMode.Value is Switch State; assigning a Drive State member is a mismatch.
    assert!(codes(&p, "SwitchMode.Value = Drive State.Idle;\n").contains(&TypeCode::T030));
}

#[test]
fn t030_no_flag_same_enum() {
    let p = proj();
    assert!(!codes(&p, "SwitchMode.Value = Switch State.On;\n").contains(&TypeCode::T030));
}

#[test]
fn t030_no_flag_unknown_target() {
    let p = proj();
    // driveMode has no known type -> silent.
    assert!(!codes(&p, "driveMode = 1.0;\n").contains(&TypeCode::T030));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test rules_v2 t030`
Expected: FAIL.

- [ ] **Step 3: Implement `src/rules/t030_assign_mismatch.rs`**

```rust
use m1_core::{Kind, Node, Severity};
use crate::diagnostics::{make, TypeCode, TypeDiagnostic};
use crate::resolve::{resolve, Resolution, Scope};
use crate::symbols::SymbolKind;
use crate::typer::{path_text, type_of};
use crate::types::ValueType;

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::AssignmentStatement {
            return;
        }
        // Only plain `=` (not `+=` etc., which imply the existing type).
        if !node.children().iter().any(|c| c.kind() == Kind::Assign) {
            return;
        }
        let kids = node.named_children();
        let (Some(target), Some(value)) =
            (kids.iter().find(|c| is_expr(c.kind())), kids.iter().rev().find(|c| is_expr(c.kind())))
        else { return };
        if std::ptr::eq(target, value) {
            return; // single child; nothing to compare
        }
        // Target must resolve to a Channel/Parameter with a known type.
        let target_ty = match resolve(&path_text(**target), scope) {
            Resolution::Symbol(s)
                if matches!(s.kind, SymbolKind::Channel | SymbolKind::Parameter) =>
            {
                s.value_type
            }
            _ => return,
        };
        let value_ty = type_of(**value, scope);
        if !target_ty.is_known() || !value_ty.is_known() {
            return; // silent under Unknown
        }
        if !compatible(target_ty, value_ty) {
            out.push(make(
                TypeCode::T030, node, Severity::Warning,
                format!("assigning {value_ty:?} to a target of type {target_ty:?}"),
            ));
        }
    }
}

/// Assignment compatibility (conservative).
fn compatible(target: ValueType, value: ValueType) -> bool {
    use ValueType::*;
    match (target, value) {
        (a, b) if a == b => true,
        // Integer/Unsigned interchangeable.
        (Integer, Unsigned) | (Unsigned, Integer) => true,
        // Integer channel accepts integral values; float channel accepts numerics.
        (Float, Integer) | (Float, Unsigned) => true,
        // Enum target only accepts the *same* enum.
        (Enum(a), Enum(b)) => a == b,
        _ => false, // e.g. Float -> Integer channel, Enum vs numeric, Boolean vs numeric
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

- [ ] **Step 4: Register** — `pub mod t030_assign_mismatch;` +
`Box::new(t030_assign_mismatch::Rule)` in `default_v2`.

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test --test rules_v2 && cargo test --test typer_enum && cargo test --test enums`
Expected: PASS (the fixture additions keep the enum/typer tests green —
`enum_by_name`/`enums_with_member` counts are unaffected by the new channel).

- [ ] **Step 6: Commit**

```bash
git add src/rules/t030_assign_mismatch.rs src/rules/mod.rs tests/rules_v2.rs tests/fixtures/enums.m1prj tests/fixtures/enums.m1cfg
git commit -m "Task 7: T030 assignment-type-mismatch (channel/parameter targets, known both sides)"
```

---

## Task 8: flow.rs — single-assignment control-flow pass (T040)

**Files:** Create `src/flow.rs`. Modify `src/lib.rs`, `src/rules/mod.rs`. Create `tests/flow.rs`.

- [ ] **Step 1: Write the failing tests `tests/flow.rs`**

```rust
use std::path::Path;
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;

fn proj() -> Project {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Project::load(&dir.join("enums.m1prj")).unwrap()
}

fn codes(p: &Project, src: &str) -> Vec<TypeCode> {
    check_script(p, Path::new("Foo Update.m1scr"), src)
        .diagnostics.iter().map(|d| d.code).collect()
}

#[test]
fn t040_flags_unconditional_double_assignment() {
    let p = proj();
    let src = "driveMode = Drive State.Idle;\ndriveMode = Drive State.Ready To Drive;\n";
    assert!(codes(&p, src).contains(&TypeCode::T040));
}

#[test]
fn t040_no_flag_mutually_exclusive_branches() {
    let p = proj();
    let src = "if (gain > 1) {\ndriveMode = Drive State.Idle;\n} else {\ndriveMode = Drive State.Ready To Drive;\n}\n";
    assert!(!codes(&p, src).contains(&TypeCode::T040));
}

#[test]
fn t040_flags_unconditional_plus_conditional() {
    let p = proj();
    let src = "driveMode = Drive State.Idle;\nif (gain > 1) {\ndriveMode = Drive State.Ready To Drive;\n}\n";
    assert!(codes(&p, src).contains(&TypeCode::T040));
}

#[test]
fn t040_no_flag_for_local() {
    let p = proj();
    let src = "local iX = 1;\niX = 2;\n";
    assert!(!codes(&p, src).contains(&TypeCode::T040));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test flow`
Expected: FAIL (no flow pass).

- [ ] **Step 3: Implement `src/flow.rs`**

```rust
//! Control-flow analysis: single-assignment-per-channel (T040).
//!
//! For each statement subtree we compute, per channel path, the maximum number
//! of assignments on any single execution path (`max_per_path`). Sequential
//! statements add; mutually-exclusive `if`/`else` arms take the per-key max. A
//! channel whose top-level `max_per_path` reaches 2 is assigned twice on one path.
use std::collections::HashMap;
use m1_core::{Kind, Node};
use crate::diagnostics::{make, TypeCode, TypeDiagnostic};
use crate::resolve::{resolve, Resolution, Scope};
use crate::symbols::SymbolKind;
use crate::typer::path_text;

#[derive(Default)]
struct Facts {
    /// channel path -> max assignments on a single path through this subtree.
    max_per_path: HashMap<String, u32>,
    /// channel path -> the node of the assignment that pushed it to its 2nd hit,
    /// used to anchor the diagnostic.
    second_at: HashMap<String, (u32, u32)>, // (line, column) of the 2nd assignment
}

/// Entry point: run over the whole script, append T040 diagnostics.
pub fn single_assignment(root: Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    let facts = seq_children(root, scope);
    for (path, n) in &facts.max_per_path {
        if *n >= 2 {
            // Re-walk to find the 2nd assignment node for a precise span.
            if let Some(node) = nth_assignment(root, scope, path, 2) {
                out.push(make(
                    TypeCode::T040, &node,
                    m1_core::Severity::Warning,
                    format!("channel `{path}` is assigned more than once on a single code path"),
                ));
            }
        }
    }
}

/// Compose the facts of a node's *statement* children in sequence.
fn seq_children(node: Node, scope: &Scope) -> Facts {
    let mut acc = Facts::default();
    for child in node.children() {
        let f = facts_of(child, scope);
        acc = merge_seq(acc, f);
    }
    acc
}

/// Facts for a single statement node.
fn facts_of(node: Node, scope: &Scope) -> Facts {
    match node.kind() {
        Kind::AssignmentStatement => {
            let mut f = Facts::default();
            if let Some(path) = assigned_channel(node, scope) {
                f.max_per_path.insert(path, 1);
            }
            f
        }
        Kind::IfStatement => {
            // then-block facts vs else-block facts -> per-key max (mutual exclusion).
            let blocks: Vec<_> = node.children().into_iter()
                .filter(|c| c.kind() == Kind::Block || c.kind() == Kind::ElseClause)
                .collect();
            let then_f = blocks.first().map(|b| seq_children(*b, scope)).unwrap_or_default();
            let else_f = blocks.iter().find(|b| b.kind() == Kind::ElseClause)
                .map(|b| seq_children(*b, scope)).unwrap_or_default();
            merge_alt(then_f, else_f)
        }
        Kind::WhenStatement | Kind::ExpandStatement => {
            // Treated as a no-else conditional body (see spec §5.2).
            let body = node.children().into_iter().find(|c| c.kind() == Kind::Block);
            let body_f = body.map(|b| seq_children(b, scope)).unwrap_or_default();
            merge_alt(body_f, Facts::default())
        }
        Kind::Block => seq_children(node, scope),
        _ => Facts::default(),
    }
}

fn merge_seq(mut a: Facts, b: Facts) -> Facts {
    for (k, v) in b.max_per_path {
        let entry = a.max_per_path.entry(k).or_insert(0);
        *entry += v;
    }
    a
}

fn merge_alt(a: Facts, b: Facts) -> Facts {
    let mut out = Facts::default();
    for (k, v) in a.max_per_path.iter().chain(b.max_per_path.iter()) {
        let e = out.max_per_path.entry(k.clone()).or_insert(0);
        *e = (*e).max(*v);
    }
    out
}

/// If `node` is `target = …` to a Channel symbol, return its canonical path.
fn assigned_channel(node: Node, scope: &Scope) -> Option<String> {
    if !node.children().iter().any(|c| c.kind() == Kind::Assign) {
        return None; // skip += etc. (still a single assignment per path, but the
                     // "set once" rule targets plain assignment; conservative)
    }
    let target = node.named_children().into_iter().find(|c| is_expr(c.kind()))?;
    match resolve(&path_text(target), scope) {
        Resolution::Symbol(s) if s.kind == SymbolKind::Channel => Some(s.path.clone()),
        _ => None,
    }
}

/// Find the n-th (1-based) assignment to `path` in document order, for the span.
fn nth_assignment<'a>(root: Node<'a>, scope: &Scope, path: &str, n: u32) -> Option<Node<'a>> {
    let mut count = 0;
    fn walk<'a>(node: Node<'a>, scope: &Scope, path: &str, n: u32, count: &mut u32) -> Option<Node<'a>> {
        if node.kind() == Kind::AssignmentStatement {
            if assigned_channel(node, scope).as_deref() == Some(path) {
                *count += 1;
                if *count == n { return Some(node); }
            }
        }
        for c in node.children() {
            if let Some(found) = walk(c, scope, path, n, count) { return Some(found); }
        }
        None
    }
    walk(root, scope, path, n, &mut count)
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

> The unused `second_at` field is a placeholder for a future precise-span path;
> `nth_assignment` already gives the 2nd-assignment node, so **remove `second_at`**
> if clippy flags it (it is not read). Keep the struct minimal: just
> `max_per_path`.

- [ ] **Step 4: Wire into the runner and lib**

In `src/lib.rs` add `pub mod flow;`. In `src/rules/mod.rs`'s `run`, after the
registry walk, call the flow pass:

```rust
    walk(cst.root(), &registry, &scope, &mut diagnostics);
    crate::flow::single_assignment(cst.root(), &scope, &mut diagnostics);
    CheckResult { diagnostics, syntax_errors }
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test --test flow`
Expected: PASS (4 tests). If the `IfStatement` child shape differs (e.g. the
then-body is the direct `Block` child and the `else` is an `ElseClause` wrapping
another `Block`), confirm with a quick `dbg!` of `node.children().map(|c|
c.kind())` on the if-fixture and adjust the `blocks` selection. The tests pin
behaviour.

- [ ] **Step 6: Commit**

```bash
git add src/flow.rs src/lib.rs src/rules/mod.rs tests/flow.rs
git commit -m "Task 8: T040 single-assignment-per-channel control-flow pass"
```

---

## Task 9: audit.rs — project-wide name audit (T050)

**Files:** Create `src/audit.rs`. Modify `src/lib.rs`, `src/project.rs`. Create `tests/fixtures/naming.m1prj`, `tests/audit.rs`.

- [ ] **Step 1: Write the fixture `tests/fixtures/naming.m1prj`**

```xml
<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="Naming" TargetHardware="ecu120">
  <SelectedModuleSets><File Name="MoTeC Comms"/></SelectedModuleSets>
  <DataTypes>
   <Type Name="GoodState" Storage="enum" Default="Off">
    <Enum Name="Off" ContainerOrder="0"/>
   </Type>
   <Type Name="bad state" Storage="enum" Default="Off">
    <Enum Name="Off" ContainerOrder="0"/>
   </Type>
  </DataTypes>
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Inputs"/>
    <Component Classname="BuiltIn.Channel" Name="Root.Inputs.brakePressure"><Props/></Component>
    <Component Classname="BuiltIn.Channel" Name="Root.Inputs.BrakePressure"><Props/></Component>
    <Component Classname="BuiltIn.Parameter" Name="Root.Inputs.BrakeBias"><Props/></Component>
    <Component Classname="BuiltIn.Parameter" Name="Root.Inputs.brakeBias"><Props/></Component>
    <Component Classname="BuiltIn.Constant" Name="Root.Inputs.TRACKWIDTH"><Props Value="1"/></Component>
    <Component Classname="BuiltIn.Constant" Name="Root.Inputs.trackWidth"><Props Value="1"/></Component>
    <Component Classname="BuiltIn.FuncUser" Filename="x.m1scr" Name="Root.Inputs.checkLimit"/>
    <Component Classname="BuiltIn.FuncUser" Filename="y.m1scr" Name="Root.Inputs.CheckLimit"/>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
```

For each kind one conforming (`brakePressure`, `BrakeBias`, `TRACKWIDTH`,
`checkLimit`, enum `GoodState`) and one violating (`BrakePressure`, `brakeBias`,
`trackWidth`, `CheckLimit`, enum `bad state`).

- [ ] **Step 2: Write the failing test `tests/audit.rs`**

```rust
use std::path::Path;
use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::project::Project;

fn proj() -> Project {
    Project::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/naming.m1prj"),
    ).unwrap()
}

#[test]
fn t050_flags_only_violators() {
    let p = proj();
    let msgs: Vec<String> = p.audit().iter()
        .filter(|d| d.code == TypeCode::T050)
        .map(|d| d.inner.message.clone())
        .collect();
    let all = msgs.join("\n");
    // Violators present.
    assert!(all.contains("BrakePressure"), "channel UpperCamel should flag: {all}");
    assert!(all.contains("brakeBias"), "parameter lowerCamel should flag");
    assert!(all.contains("trackWidth"), "constant non-caps should flag");
    assert!(all.contains("CheckLimit"), "function UpperCamel should flag");
    assert!(all.contains("bad state"), "enum with space should flag");
    // Conformers absent.
    assert!(!all.contains("brakePressure"), "conforming channel must not flag");
    assert!(!all.contains("\"BrakeBias\""), "conforming parameter must not flag");
    assert!(!all.contains("TRACKWIDTH"));
    assert!(!all.contains("checkLimit"));
    assert!(!all.contains("GoodState"));
}
```

> The `brakePressure`/`BrakePressure` and `BrakeBias`/`brakeBias` pairs are
> substrings of each other; the assertions above use the exact violating spellings
> which are not substrings of their conforming counterparts (`BrakePressure` is not
> a substring of `brakePressure`, and vice-versa). Keep messages quoting the leaf
> in backticks so `contains` matches the intended token; if a collision arises,
> assert on the full ``` `leaf` ``` form (e.g. `` all.contains("`BrakePressure`") ``).

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test --test audit`
Expected: FAIL — `Project::audit` undefined.

- [ ] **Step 4: Implement `src/audit.rs`**

```rust
//! Project-wide symbol-name audit (T050) against the EV-M1 naming conventions.
use m1_core::Severity;
use crate::diagnostics::{make_project, TypeCode, TypeDiagnostic};
use crate::project::Project;
use crate::symbols::SymbolKind;

/// Audit the project's own symbol + enum-type names. Empty unless violations.
pub fn audit_project(project: &Project) -> Vec<TypeDiagnostic> {
    let table = project.symbols();
    let mut out = Vec::new();

    for sym in table.iter() {
        let leaf = leaf_of(&sym.path);
        let conv = match sym.kind {
            SymbolKind::Channel => Some(("lowerCamelCase", lower_camel(leaf))),
            SymbolKind::Parameter | SymbolKind::Group => Some(("UpperCamelCase", upper_camel(leaf))),
            SymbolKind::Constant => Some(("CAPITALISATION", capitalised(leaf))),
            SymbolKind::Function | SymbolKind::Method => Some(("lowerCamelCase", lower_camel(leaf))),
            _ => None,
        };
        if let Some((name, ok)) = conv {
            if !ok {
                out.push(make_project(
                    TypeCode::T050, Severity::Warning,
                    format!("{:?} `{leaf}` does not follow {name} (in `{}`)", sym.kind, sym.path),
                ));
            }
        }
    }
    for e in table.enums() {
        if !upper_camel(&e.name) {
            out.push(make_project(
                TypeCode::T050, Severity::Warning,
                format!("enum type `{}` does not follow UpperCamelCase", e.name),
            ));
        }
    }
    out
}

fn leaf_of(path: &str) -> &str {
    path.rsplit_once('.').map(|(_, l)| l).unwrap_or(path)
}

fn lower_camel(s: &str) -> bool {
    let Some(c) = s.chars().next() else { return false };
    c.is_ascii_lowercase() && !s.contains(' ') && !s.contains('_')
}
fn upper_camel(s: &str) -> bool {
    let Some(c) = s.chars().next() else { return false };
    c.is_ascii_uppercase() && !s.contains(' ') && !s.contains('_')
}
/// CAPITALISATION: letters are upper-case; spaces and digits allowed (per
/// CONTRIBUTING "Constant names may use spaces").
fn capitalised(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphabetic())
        && s.chars().filter(|c| c.is_ascii_alphabetic()).all(|c| c.is_ascii_uppercase())
}
```

In `src/project.rs` add:

```rust
    /// Audit the project's own symbol names (T050).
    pub fn audit(&self) -> Vec<crate::diagnostics::TypeDiagnostic> {
        crate::audit::audit_project(self)
    }
```

In `src/lib.rs` add `pub mod audit;`.

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test --test audit`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/audit.rs src/lib.rs src/project.rs tests/audit.rs tests/fixtures/naming.m1prj
git commit -m "Task 9: T050 project-wide symbol-name audit (audit_project + per-kind conventions)"
```

---

## Task 10: CLI --audit-names + corpus baseline + polish

**Files:** Modify `src/main.rs`. Modify `tests/corpus.rs`. Modify `README.md`.

- [ ] **Step 1: Add `--audit-names` to `src/main.rs`**

Add the flag to `Args`:

```rust
    /// Audit the project's own symbol names against the naming conventions (T050).
    #[arg(long)]
    audit_names: bool,
```

After the per-file loop in `main`, before the exit:

```rust
    if args.audit_names {
        match &project {
            Some(p) => {
                let path_label = find_project(&args)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<project>".into());
                for d in p.audit() {
                    println!("{}: {}[{}]: {}",
                        path_label,
                        match d.inner.severity {
                            Severity::Error => "error",
                            Severity::Warning => "warning",
                            Severity::Info => "info",
                            Severity::Hint => "hint",
                        },
                        d.code.as_str(),
                        d.inner.message);
                }
            }
            None => eprintln!("m1-typecheck: --audit-names needs a project; skipping"),
        }
    }
```

(Note: `find_project` is called twice; acceptable, or cache it in a `let
project_path = find_project(&args);` at the top and reuse — do whichever keeps
clippy quiet.)

- [ ] **Step 2: Smoke test**

Run:
```bash
cargo build --release
M1_PROJECT=../EV-M1/UQR-EV/01.00/Project.m1prj ./target/release/m1-typecheck --audit-names 2>/dev/null | head
```
Expected: a stream of `…/Project.m1prj: warning[T050]: …` lines (the example
project violates the conventions — this is expected; T050 is opt-in for exactly
this reason).

- [ ] **Step 3: Extend `tests/corpus.rs`**

Add a second assertion that the v2 per-script pass still does not panic and record
the count (the existing test already iterates scripts via `check_script`, which now
includes T020/T021/T030/T040). Append an audit smoke test:

```rust
#[test]
fn audits_project_names_without_panicking() {
    let Some(proj_path) = project_path() else { return };
    let project = Project::load(&proj_path).expect("load project");
    let n = project.audit().len();
    eprintln!("project name-audit produced {n} T050 diagnostics");
    // No assertion on the exact count: the example project intentionally violates
    // the conventions. This guards only against a panic / API regression.
}
```

(Add `use m1_typecheck::project::Project;` if not already imported — it is.)

- [ ] **Step 4: Full suite + lints**

Run: `cargo test`
Expected: all unit/integration tests pass; corpus tests run or skip.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: zero warnings. Remove the `Facts::second_at` field if flagged (Task 8
note). Cache `find_project` if the double-call is flagged.

Run: `cargo fmt --check`
Expected: clean.

- [ ] **Step 5: Update `README.md`**

Add a "v2" section documenting: the enum domain model (member index, association
via `.m1cfg` enum cells, typed member paths), the new rules T020/T021/T030/T040,
the opt-in `--audit-names` (T050) and why it is off by default, and the
`default_v2` registry. Keep the synthetic-identifier obfuscation note.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs tests/corpus.rs README.md
git commit -m "Task 10: --audit-names CLI flag, corpus audit smoke test, README v2, clippy/fmt clean"
```

---

## Deferred to v3

| Item | Note |
|------|------|
| Function/method return-type inference | `.m1prj` carries no return type; needs cross-file body analysis. Calls stay `Unknown`. |
| Absolute-path-reference enforcement | `Parent.X` vs `GroupA.X` intent is brittle; high false-positive risk. |
| Promote symbol model into `m1-core` | When `m1-lsp` becomes the second consumer. |
| `--format json` | After human + audit output prove out. |
| Cross-file / project-wide script analysis, incremental reload | With `m1-lsp` integration. |
| Per-channel enum association from function bodies | Mine `channel = <EnumType>.<Member>` assignments; cross-file. |

---

## Summary of tasks

| # | Title | New code |
|---|-------|----------|
| 1 | Enum member index + queries; `Symbol.enum_assoc` | `symbols/mod.rs`, `symbols/m1prj.rs` |
| 2 | `.m1cfg` enum-cell back-resolution → association | `symbols/m1cfg.rs`, `symbols/mod.rs` |
| 3 | Typer produces `ValueType::Enum` | `typer.rs`, `types.rs` |
| 4 | New `TypeCode`s T020/T021/T030/T040/T050 | `diagnostics.rs` |
| 5 | T020 enum-non-member + `default_v2` registry | `rules/t020_enum_member.rs`, `rules/mod.rs` |
| 6 | T021 enum/numeric comparison | `rules/t021_enum_numeric_cmp.rs` |
| 7 | T030 assignment-type-mismatch | `rules/t030_assign_mismatch.rs` |
| 8 | T040 single-assignment control-flow pass | `flow.rs` |
| 9 | T050 project-wide name audit | `audit.rs`, `project.rs` |
| 10 | CLI `--audit-names`, corpus baseline, polish | `main.rs`, `tests/corpus.rs`, `README.md` |
