# m1-typecheck v1 — Design Specification

**Date:** 2026-05-30
**Status:** Approved for implementation
**Scope:** v1 — the `.m1prj`/`.m1cfg` symbol model, name resolution, a basic expression type system, and the type/naming rules those unlock

> **Note:** Example identifiers in this document are synthetic placeholders, not
> drawn from any real project. The example project/corpus path is resolved via
> the `M1_CORPUS_PATH` / `M1_PROJECT` env vars (falling back to the sibling
> EV-M1 example project).

---

## 1. Purpose

`m1-typecheck` is the semantic-analysis layer of the M1 toolchain. Where
`m1-core` (syntax) and `m1-lint` (CST-only style rules) know nothing about what a
name *is*, `m1-typecheck` loads the project's **symbol model** — the channels,
parameters, constants, functions, and enum types defined in the MoTeC
`Project.m1prj` (and the parameter types/units in `.m1cfg`) — and uses it to:

- **resolve** every identifier path in a `.m1scr` script to a symbol (or report
  that it can't),
- **type** expressions (combining the symbol table, literal forms, and the
  Hungarian-notation convention for locals), and
- emit **type/naming diagnostics** that are impossible without this knowledge:
  precise floating-point-equality detection, integer/float mixing, and
  local-variable naming-convention checks.

It is both a **library** (consumed by `m1-lsp` in v2 for hover/types and as a
third diagnostic source) and a **CLI** (`m1-typecheck path/*.m1scr`).

Toolchain position:

```
tree-sitter-m1 → m1-core → m1-typecheck   (this crate; symbol model + types)
                            m1-lint        (CST-only rules)
                            m1-fmt
                            m1-lsp         (consumes typecheck in v2)
```

`m1-typecheck` depends on `m1-core` and must NOT import `tree-sitter` directly.

---

## 2. Where the symbol model lives

`m1-core`'s spec deferred the symbol model "until `m1-typecheck` gives it a
concrete shape to satisfy." This crate defines that shape.

**Decision (v1):** implement the symbol model **inside `m1-typecheck`** (module
`symbols`). Once it is stable and a second consumer needs it (e.g. `m1-lsp`
features), it can be promoted into `m1-core` behind the same public types with no
API change for downstream callers. v1 does not pre-emptively put it in `m1-core`
(YAGNI; avoids adding an XML dependency to the foundational crate before its
shape is proven).

---

## 3. Inputs — the `.m1prj` and `.m1cfg` formats

### 3.1 `Project.m1prj` (XML, ~0.5 MB for a real project)

The symbol table is the flat list of `<Component>` elements:

```xml
<Component Classname="BuiltIn.Channel"  Name="Root.Foo.Bar.Diagnostic"><Props Security="Tune"/></Component>
<Component Classname="BuiltIn.Constant" Name="Root.Foo.Bar.Bus"><Props Value="CAN Bus 2"/></Component>
<Component Classname="BuiltIn.Parameter" Name="Root.Foo.Bar.Mode.Value"><Props Security="Master Calibration"/></Component>
<Component Classname="BuiltIn.FuncUser" Filename="Foo Bar Update.m1scr" Name="Root.Foo.Bar.Update"/>
<Component Classname="BuiltIn.MethodUser" Name="Root.Foo.Bar.Startup"/>
```

Relevant facts (verified against the example project):
- **`Name`** is the fully-qualified dotted path, rooted at `Root.`. Path
  *segments* may contain spaces (`Root.Foo.Vund Klee.Status Word`).
- **`Classname`** gives the symbol kind. The kinds we map (others → `Other`):
  | Classname (prefix) | SymbolKind |
  |---|---|
  | `BuiltIn.Channel`, `BuiltIn.CalibrationChannel` | `Channel` |
  | `BuiltIn.Parameter`, `BuiltIn.CalibrationParameter`, `BuiltIn.IOResourceParameter` | `Parameter` |
  | `BuiltIn.Constant`, `BuiltIn.IOResourceConstant` | `Constant` |
  | `BuiltIn.FuncUser`, `BuiltIn.CalFuncUser`, `BuiltIn.FuncGenerated.*` | `Function` |
  | `BuiltIn.MethodUser`, `BuiltIn.EventKernel` | `Method` |
  | `BuiltIn.Table`, `BuiltIn.CalibrationTable` | `Table` |
  | `BuiltIn.GroupCompound` | `Group` |
  | `BuiltIn.Reference` | `Reference` |
  | anything else (`MoTeC Input.Sensor`, `_IOMethod.*`, …) | `Other` |
- **`Filename`** (when present, on `FuncUser`/`MethodUser`) links a symbol to the
  `.m1scr` file that is its body. This is how a script maps to its **enclosing
  group**: a script whose function symbol is `Root.Foo.Bar.Update` lives in group
  `Root.Foo.Bar`, which is the base for relative name resolution (§5).
- **`<DataTypes>`** defines enum types:
  ```xml
  <Type Name="Switch State" Storage="enum" Default="Off">
    <Enum Name="Off" ContainerOrder="0"/><Enum Name="On" ContainerOrder="1"/>
  </Type>
  ```
  → an `EnumType { name, members: [(name, order)], default }`.

### 3.2 `parameters.m1cfg` (XML, small)

Gives concrete **types and units** for parameters:

```xml
<Parameter Name="Inputs.APPS.APPS2.Offset"><Cell Type="f32" Unit="V"><![CDATA[2.08]]></Cell></Parameter>
<Parameter Name="Outputs.Logging.LVLogging"><Cell Type="enum"><![CDATA[On]]></Cell></Parameter>
```

`Cell/@Type` ∈ {`f32`, `enum`, `u8/u16/u32`, `s8/s16/s32`, `bool`, …}. We map
these to `ValueType` (§4) and attach the unit string. The `.m1cfg` augments the
`.m1prj` symbol entries (matched by path) with value types; it is optional.

### 3.3 Parsing

Read-only XML traversal with **`roxmltree`** (fast, borrow-based DOM; no schema
validation needed — the project's own `validate_m1prj.py` already does that). A
0.5 MB document parses in a few ms. We never write these files.

---

## 4. The type model

```rust
pub enum ValueType {
    Boolean,
    Integer,    // signed
    Unsigned,
    Float,
    Enum(EnumId),   // index into the table's enum list
    String,
    Unknown,        // unresolved / not inferable — never produces a diagnostic
}
```

Mapping rules:
- **`.m1cfg` Cell types:** `f32`/`f64` → `Float`; `u8/u16/u32/u64` → `Unsigned`;
  `s8/s16/s32/s64` → `Integer`; `bool` → `Boolean`; `enum` → `Enum(..)` (resolved
  by the parameter's declared data type when available, else a nameless enum).
- **Literals:** integer literal `1` → `Integer`; `0u`/hex `0x..` → `Unsigned`;
  `1.0`/`1e3` → `Float`; `true`/`false` → `Boolean`; `"..."` → `String`.
- **Locals (Hungarian):** prefix letter of the declared name → `b`→`Boolean`,
  `u`→`Unsigned`, `i`→`Integer`, `f`→`Float`; explicit `local <Type> name`
  annotation overrides the prefix (resolve `<Type>` against project data types /
  the primitive names).
- **Symbols:** `Parameter`/`Channel` value type from `.m1cfg` or `.m1prj` props
  when present, else `Unknown`. `Constant` → type of its `Props/@Value` literal.
  `Function`/`Method` **call results → `Unknown`** in v1 (return types are not
  reliably available; conservative to avoid false positives).

**`Unknown` is absorbing and silent:** any rule that would fire only does so when
all operands it needs are *known*. This guarantees no false positives from gaps
in the symbol/return-type data.

### 4.1 Expression typer

`type_of(node, ctx) -> ValueType` over the `m1-core` CST:
- literals → as above;
- identifier / member-expression path → resolve (§5) → symbol/local type;
- `parenthesized` → inner;
- `unary` `-x` → type of `x`; `not x`/`!x` → `Boolean`;
- `binary` arithmetic (`+ - * / %`) → numeric join (`Float` if either is
  `Float`, else `Unsigned` if both `Unsigned`, else `Integer`; `Unknown` if
  either operand is `Unknown`);
- `binary` comparison/logical (`== != eq neq < > <= >= and or && ||`) → `Boolean`;
- `binary` bitwise/shift (`& | ^ << >>`) → left operand's integer type;
- `ternary` → join of the two branches;
- `call` → `Unknown` (v1).

---

## 5. Name resolution

`resolve(path_segments, scope) -> Option<&Symbol>`:

A `.m1scr` reference is a member-expression chain of identifier segments (each
possibly space-containing), e.g. `Foo.Vund Klee.Status Word`. Resolution order:

1. **Local** — if the path is a single segment matching a `local` declared
   earlier in the script, it resolves to that local (kind `Local`).
2. **Absolute** — try the path as-is and `Root.`-prefixed against the table.
3. **Group-relative** — prepend the script's enclosing-group path (from the
   `.m1prj` `Filename` mapping, §3.1), then walk up ancestor groups to `Root`,
   trying each prefix. First match wins.
4. **Built-in / opaque root** — if the leading segment is a known built-in
   library namespace (see below), the reference is treated as **resolved-opaque**
   (type `Unknown`), never flagged.
5. Otherwise → **unresolved**.

**Built-in opaque roots (v1 allowlist):** the leading segments of the selected
module sets (e.g. `Calculate`, `Limit`, `Convert`, `Math`, `CanComms`, `Time`,
`Table`, `Channel`, `Constant`, `Parameter`, plus the module-set names found in
`<SelectedModuleSets>`), and `This` (the M1 self-reference). This list is derived
at load time from the project's module sets, augmented by a small static set of
always-present M1 standard-library roots. Treating unknown roots as opaque is the
key false-positive guard: **`m1-typecheck` only reports an unresolved reference
when the path's root IS a known project group but the remainder doesn't exist.**

The enclosing-group context is established per file: the driver maps each input
`.m1scr` path to its `FuncUser`/`MethodUser` symbol via the `Filename` attribute;
if a script has no matching component (e.g. analysed standalone), group-relative
resolution is skipped and only absolute/local/opaque resolution applies.

---

## 6. Diagnostics (v1 rules)

Codes are `T0xx`. Reuses `m1_core::Diagnostic` as the payload, wrapped in
`TypeDiagnostic { code: TypeCode, inner: Diagnostic }` (mirrors `m1-lint`'s
`LintDiagnostic` pattern, keeping `m1-core` decoupled from semantic codes).

| Code | Severity | Rule |
|------|----------|------|
| **T001** | Warning | **unresolved-reference**: a member path whose root is a known project group does not resolve to any symbol. (Opaque/library roots and `Unknown`-rooted paths are never flagged — §5.) |
| **T002** | Error | **float-equality**: an `==`/`!=`/`eq`/`neq` whose either operand types as `Float`. Precise successor to `m1-lint`'s literal-only heuristic L006. Message points at the operator and suggests a tolerance check. |
| **T003** | Warning | **int-float-mixing**: an arithmetic binary op (`+ - * / %`) with one operand `Float` and the other `Integer`/`Unsigned`, without an explicit `Convert.ToFloat`/`Convert.ToInteger` wrapping the non-float operand. Mirrors CONTRIBUTING "avoid mixing Integer and Floating Point". |
| **T010** | Warning | **local-hungarian-prefix**: a `local` declaration whose name lacks a recognised type prefix (`b`/`u`/`i`/`f` followed by an upper-case letter), e.g. `local count = …`. |
| **T011** | Warning | **hungarian-prefix-mismatch**: a `local` whose prefix letter disagrees with the type of its initialiser (e.g. `local fCount = 3;` where `3` is `Integer`), when that type is known. |

Rules fire only on operands/initialisers whose type is **known** (§4) — `Unknown`
never triggers T002/T003/T011, eliminating false positives from symbol-data gaps.

### 6.1 Relationship to `m1-lint`

`m1-lint` keeps its CST-only rules including the L006 float-eq *heuristic* (works
with no project loaded). `m1-typecheck`'s T002 is the precise version used when a
project is available. They are complementary, not duplicative: a CI run can use
`m1-lint` alone (no project) or both (project loaded). `m1-lsp` v2 will prefer
T002 and suppress L006 when a project is present (documented there, not here).

---

## 7. Architecture

### 7.1 Crate layout

```
m1-typecheck/
  Cargo.toml          (lib + bin; deps: m1-core, roxmltree)
  src/
    main.rs           (CLI: load project, check files, print diagnostics, exit code)
    lib.rs            (public surface: Project, SymbolTable, check_script, TypeDiagnostic)
    symbols/
      mod.rs          (SymbolTable, Symbol, SymbolKind, lookup/iteration)
      m1prj.rs        (parse Project.m1prj -> components, datatypes -> SymbolTable)
      m1cfg.rs        (parse parameters.m1cfg -> param value types/units; augment table)
      enums.rs        (EnumType, EnumId, datatype parsing)
    types.rs          (ValueType, literal typing, numeric join)
    resolve.rs        (Scope, resolution algorithm, built-in opaque roots)
    typer.rs          (type_of over the CST)
    rules/
      mod.rs          (Rule trait, Registry, Runner -> Vec<TypeDiagnostic>)
      t001_unresolved.rs
      t002_float_eq.rs
      t003_int_float_mix.rs
      t010_local_prefix.rs
      t011_prefix_mismatch.rs
    diagnostics.rs    (TypeCode, TypeDiagnostic, CheckResult)
  tests/
    symbols.rs        (m1prj/m1cfg parsing -> expected symbols & enums)
    resolve.rs        (absolute/relative/opaque resolution cases)
    typer.rs          (expression typing cases)
    rules.rs          (per-rule fixtures, no false positives on conforming code)
    corpus.rs         (load example project + check all scripts: no panic, baseline counts)
  tests/fixtures/
    mini.m1prj        (hand-written tiny project: a few channels/params/constants/enums/funcs)
    mini.m1cfg
    *.m1scr / *.tdiag (input + expected diagnostics, like m1-lint's fixtures)
```

### 7.2 Public API

```rust
pub struct Project { /* SymbolTable + enums + module-set roots + file->group map */ }

impl Project {
    /// Load a Project.m1prj (and optional sibling parameters.m1cfg).
    pub fn load(m1prj: &Path) -> Result<Project, LoadError>;
    pub fn with_config(self, m1cfg: &Path) -> Result<Project, LoadError>;
    /// The symbol table for queries (used by m1-lsp later).
    pub fn symbols(&self) -> &SymbolTable;
}

pub struct CheckResult { pub diagnostics: Vec<TypeDiagnostic>, pub syntax_errors: Vec<m1_core::Diagnostic> }

/// Type-check one script against a project. `script_path` is used to find the
/// enclosing group for relative resolution; pass the on-disk path.
pub fn check_script(project: &Project, script_path: &Path, source: &str) -> CheckResult;

/// Project-less mode: resolution falls back to local/opaque only; T001 is not
/// emitted (no project groups to anchor it). Useful for editor use before a
/// project is selected.
pub fn check_script_no_project(source: &str) -> CheckResult;
```

### 7.3 Rule invocation model

Same visitor shape as `m1-lint`: the `Runner` parses once via `m1-core`, builds a
per-script `Scope` (locals + enclosing group + project), then walks the CST
once calling each rule's `check_node`. Rules that need types call the shared
`typer::type_of`. `CheckResult::syntax_errors` come from `m1-core` (rules are not
run when the parse has errors — a clean parse is a precondition, matching how
type checkers behave on unparseable input).

### 7.4 Data flow

```
Project.m1prj ─roxmltree→ components+datatypes ┐
parameters.m1cfg ─roxmltree→ param types ──────┤→ Project { SymbolTable, enums, roots, file→group }
                                                          │
.m1scr source ─m1_core::parse→ Cst ─┐                     │
                                    ▼                     ▼
                          Runner: Scope(locals, group, project)
                                    │  single CST walk
                                    ▼
                          rules T001/T002/T003/T010/T011  ──→ Vec<TypeDiagnostic>
```

---

## 8. CLI

```
m1-typecheck [--project <Project.m1prj>] [--config <parameters.m1cfg>] <file.m1scr>...
```

- `--project` defaults to the nearest `Project.m1prj` found upward from the first
  input file, or `$M1_PROJECT` if set. If none is found, runs in project-less
  mode (T001 disabled) and notes this on stderr.
- Output: human-readable, identical shape to `m1-lint`:
  `path:line:col: severity[T0xx]: message`. Syntax errors (from `m1-core`)
  printed first as `error[syntax]:`.
- Exit codes: `0` no error-severity diagnostics; `1` ≥1 error; `2` invocation
  error (bad project file, file not found).

---

## 9. Testing strategy

- **Symbol parsing** (`tests/symbols.rs`): a hand-written `mini.m1prj` with a
  known set of channels/params/constants/funcs/enums → assert exact
  `SymbolTable` contents, kinds, the `Filename→group` map, and enum members.
  Augment with `mini.m1cfg` → assert parameter value types/units attached.
- **Resolution** (`tests/resolve.rs`): absolute hit, `Root.`-prefixed hit,
  group-relative hit, ancestor-walk hit, opaque-root pass-through (`Calculate.Min`
  resolves-opaque), and true miss (project-rooted but absent → unresolved).
- **Typer** (`tests/typer.rs`): each literal form, Hungarian locals, `<Type>`
  annotation override, numeric join (`int + float → float`), comparison →
  `Boolean`, call → `Unknown`.
- **Per-rule fixtures** (`tests/rules.rs` + `tests/fixtures/*.tdiag`): for each
  `T0xx`, a positive case at an exact `line:col` and a conforming negative case
  with **no** diagnostic. Explicit no-false-positive cases: `eq` on two integers
  (no T002), `float + Convert.ToFloat(int)` (no T003), library call paths (no
  T001).
- **Corpus gate** (`tests/corpus.rs`, skipped unless the example project is
  present): load the example `Project.m1prj`, `check_script` every `*.m1scr`
  under `$M1_CORPUS_PATH`; assert no panic and that diagnostic counts match a
  committed `corpus_baseline.json` (regenerated via an `--include-ignored`
  helper, as in `m1-lint`). This catches resolution false-positive regressions
  against real scripts.

---

## 10. Out of scope (v1) — YAGNI / deferred

| Item | Why deferred |
|------|--------------|
| Assignment type-compatibility (`channel = wrongType`) | Needs reliable channel value types across the whole table; high false-positive risk in v1 |
| Enum-domain checks (`is (NotAMember)`, enum/int compares) | Needs enum association per channel; v2 |
| Function/method return-type inference | Return types not reliably in `.m1prj`; calls stay `Unknown` |
| Single-assignment-per-channel | Needs control-flow analysis (the deferred `m1-lint` B-rule) |
| Absolute-path-reference enforcement | Needs confident relative-vs-absolute intent; v2 |
| Project-wide symbol-name auditing (param/constant/channel naming) | A separate "lint the `.m1prj`" feature; v2 |
| Cross-file / project-wide analysis, incremental reload | v2 (also when `m1-lsp` integrates this) |
| Promoting the symbol model into `m1-core` | Only once a second consumer needs it |
| `--format json` | After human output proves out; v2 |

---

## 11. Non-goals

- `m1-typecheck` does not parse `.m1scr` itself (delegates to `m1-core`) or
  validate the `.m1prj`/`.m1cfg` against their schemas (the project's Python
  `validate_*` scripts and M1 Build own that).
- `m1-typecheck` does not modify any file.
- `m1-typecheck` does not re-implement `m1-lint`'s CST-only rules; it adds only
  the rules that require the symbol model or a type.
- v1 does not aim for soundness/completeness of the M1 type system — it is a
  best-effort checker that is silent under uncertainty (`Unknown`).
