# m1-typecheck v2 — Design Specification

**Date:** 2026-05-31
**Status:** Approved for implementation
**Scope:** v2 — a real enum domain model (enum association + member typing), the
type/semantic rules it unlocks (enum-membership, enum/int comparison, assignment
type-compatibility), single-assignment-per-channel control-flow analysis, and a
project-wide symbol-name audit ("lint the `.m1prj`")

> **Note:** Example identifiers in this document are synthetic placeholders, not
> drawn from any real project. The example project/corpus path is resolved via
> the `M1_CORPUS_PATH` / `M1_PROJECT` env vars (falling back to the sibling
> EV-M1 example project).

**Predecessor:** `docs/superpowers/specs/2026-05-30-m1-typecheck-v1-design.md`
(symbol model, resolution, the basic typer, rules T001–T011).

---

## 1. Purpose

v1 gave `m1-typecheck` a flat symbol model, name resolution, a best-effort
expression typer, and five rules (T001 unresolved-reference, T002 float-equality,
T003 int/float mixing, T010/T011 local Hungarian naming). It deliberately left
the symbol model **shallow about enums** — `ValueType::Enum(EnumId)` exists but no
symbol ever carries it, the `.m1cfg` enum cells map to `Unknown`, and the typer
never produces an `Enum`. It also stayed entirely **within-statement**: no flow
analysis, no whole-project view.

v2 makes two coherent steps forward, both faithful to the v1 *"silent under
`Unknown`, no false positives"* principle:

1. **An enum domain model.** Associate channels/parameters with their enum data
   type, type `<EnumType>.<Member>` and `is (Member)` against the project's
   declared `<Type Storage="enum">`s, and turn the dead `ValueType::Enum` arm into
   a live one. This unlocks **enum-membership** (`is (NotAMember)`) and
   **enum/int-comparison** checks, plus **assignment type-compatibility** for the
   cases where both sides are now known.
2. **Two analyses that need more than one statement:** a **single-assignment-per-
   channel** control-flow pass (the deferred `m1-lint` B-rule, here grounded in
   the real symbol model so it knows what *is* a channel), and a **project-wide
   symbol-name audit** that lints the `.m1prj`'s own channel/parameter/constant/
   group/enum names against the EV-M1 `CONTRIBUTING.md` naming conventions.

As in v1, `m1-typecheck` is both a **library** (the eventual `m1-lsp` consumer)
and a **CLI**. v2 does **not** promote the model into `m1-core` (still only one
consumer — deferred to v3, §11).

---

## 2. What v2 deliberately does NOT do (carried-forward / new v3 deferrals)

| Item | Why still deferred |
|------|--------------------|
| Function/method **return-type inference** | The `.m1prj` `FuncUser`/`MethodUser` components carry **no return type** (verified: `<Component Classname="BuiltIn.FuncUser" Filename="…" Name="…"><Props SelectedTrigger="…"/></Component>` — only a trigger). Inferring it would mean typing the function *body's* assignments to its output channel, a cross-file pass. Calls stay `Unknown` (v1 behaviour). v3. |
| **Absolute-path-reference enforcement** (CONTRIBUTING "use the absolute path when referencing objects outside the current main group") | Requires confident "outside the current main group" intent; `Parent.X` vs `GroupA.X` distinction is brittle without modelling the `Parent`/`This` traversal. High false-positive risk. v3. |
| Promoting the symbol model into `m1-core` | Still a single consumer (the CLI). Promote when `m1-lsp` integrates. v3. |
| `--format json` | Human output is the only consumer so far. v3. |
| Cross-file / project-wide *script* analysis, incremental reload | Belongs with `m1-lsp` integration. v3. |

v2 is intentionally a **focused increment**: enrich the model with enums, then
spend that richness on three semantic rules, one flow rule, and one project-audit
rule. It does not attempt return-type inference or path-style enforcement.

---

## 3. The enum domain model (new)

### 3.1 What the data looks like (grounded in the example project)

The `.m1prj` `<DataTypes>` block declares enum types by **name**, with ordered
members:

```xml
<Type Name="Universal Basic State Enumeration" Storage="enum" Default="OK">
  <Enum Name="Fault"   ContainerOrder="-1"><Help/></Enum>
  <Enum Name="OK"      ContainerOrder="0"><Help/></Enum>
  <Enum Name="Warning" ContainerOrder="1"><Help/></Enum>
</Type>
<Type Name="Drive State" Storage="enum" Default="Idle">
  <Enum Name="Idle" ContainerOrder="0"/>
  <Enum Name="Latching Fault" ContainerOrder="-1"/>
  <Enum Name="Ready To Drive" ContainerOrder="3"/>
</Type>
```

v1 already parses these into `EnumType { name, members: Vec<(String, i64)>,
default }` and stores them in `SymbolTable::enums`. **No code consumes them yet.**

In `.m1scr` source, enum values appear **fully named by their type** — verified in
the corpus:

```
State = Universal Basic State Enumeration.OK;
Value = Drive State.Latching Fault;
if (Driveline.Accumulator.Precharge State eq Precharge State.Error) { … }
when (Mode is (Idle)) { … }
```

So there are exactly two on-disk idioms that reveal an enum's identity:

- **Typed member path** `<EnumTypeName>.<Member>` — the leading segment(s) name a
  declared enum `<Type>`, the trailing segment is a member. This is *self-typing*:
  no association needed.
- **`is (Member)` clause** — `LHS is (Member)`. The member name (alone) identifies
  the enum **iff** it belongs to exactly one declared enum type (members are
  globally fairly unique; ambiguous names are left `Unknown`, see §3.3).

### 3.2 New `SymbolTable` enum index

Add a member-name → `EnumId` reverse index, built once at load:

```rust
// symbols/mod.rs, inside SymbolTable
member_index: HashMap<String, SmallVec<EnumId>>, // member name -> enum types that contain it
```

with queries:

```rust
/// EnumId of the enum type whose *name* equals `path` (the typed-member-path head).
pub fn enum_by_name(&self, name: &str) -> Option<EnumId>;
/// The enum types that declare a member of this exact name.
pub fn enums_with_member(&self, member: &str) -> &[EnumId];
/// Membership test.
pub fn enum_has_member(&self, id: EnumId, member: &str) -> bool;
pub fn enum_type(&self, id: EnumId) -> &EnumType;
```

`enum_by_name` matches against the longest-prefix enum-type name: for the path
`Universal Basic State Enumeration.OK` it finds the enum named `Universal Basic
State Enumeration` and treats `OK` as the member. (Implementation: strip trailing
`.<segment>` and test `enum_by_name` on the head.)

### 3.3 Enum association (channel/parameter → enum type)

A channel/parameter is **enum-associated** when we can name its enum type. Two
derivations, both conservative:

1. **`.m1cfg` enum cell back-resolution.** A parameter whose `.m1cfg` cell is
   `<Cell Type="enum"><![CDATA[OK]]></Cell>` has a *value* (`OK`) but no type
   name. Resolve it: `enums_with_member("OK")`. If **exactly one** enum type
   declares `OK`, associate the parameter with that `EnumId`; if zero or many,
   leave it `Unknown` (v1 silence). This replaces v1's `m1cfg::cell_type("enum")
   => Unknown`.
2. **Source-driven, per-statement (no persistence).** When the typer sees a typed
   member path `Drive State.Latching Fault`, it types it `Enum(<Drive State id>)`
   directly (§3.1, idiom 1) — no association table needed.

Channel enum association from the `.m1prj` alone is **not** available (the `Props`
carry only `Security`/`Value`, no `DataType` — verified). So in v2 a *bare* channel
reference (e.g. `State`) still types `Unknown` unless the `.m1cfg` associated it.
This is the conservative floor; it never produces a false positive, only fewer
true positives. (v3 could mine the function body's `= <EnumType>.<Member>`
assignments to associate the output channel — cross-file, deferred.)

### 3.4 Typer changes

`typer::type_of` gains:

- **Typed member path** `<EnumTypeName>.<Member>` → `Enum(id)` when
  `enum_by_name(head)` succeeds **and** that enum has the member; else falls
  through to existing resolution (so a real `Drive State` *group* path is
  unaffected unless it's also an enum-type name — names are distinct namespaces in
  practice, and the member check disambiguates).
- **`Symbol` with an associated enum** → `Enum(id)` (set by §3.3.1).
- Everything else unchanged. `Enum` participates in `numeric_join` as a non-numeric
  type → `Unknown` (it must not silently arithmetically combine).

---

## 4. New rules (v2)

Codes continue the `T0xx` scheme; the registry (`rules::Registry`) grows from
`default_v1()` to `default_v2()` (v1 codes retained). All new rules obey the
**known-operands-only** gate: they fire only when every type/enum they need is
known, never under `Unknown`.

| Code | Severity | Rule |
|------|----------|------|
| **T020** | Warning | **enum-non-member**: an `is (Member)` clause, or a typed member path `<EnumTypeName>.<Member>`, whose `Member` is **not** declared by the associated/named enum type. Only fires when the enum identity is known (LHS associated for `is`, head-name resolved for the path). |
| **T021** | Warning | **enum-numeric-comparison**: a comparison (`eq/neq/==/!=/<` …) with one operand of type `Enum(id)` and the other `Integer`/`Unsigned`/`Float`. Compare enums to enum members, not to raw numbers. Fires only when one side is *known* `Enum` and the other *known* numeric. |
| **T030** | Warning | **assignment-type-mismatch**: an `AssignmentStatement` `target = value` where the target resolves to a `Channel`/`Parameter` symbol with a **known** value type and the value's type is **known** and incompatible. Compatibility: identical type; `Integer`↔`Unsigned` mutually OK; `Enum(a)` only with `Enum(a)` or a member of `a`; numeric→numeric OK only same-family-or-int-widening (a `Float` literal into an integer channel is flagged, mirroring T003's spirit at the assignment boundary). Both sides `Unknown` ⇒ silent. |
| **T040** | Warning | **channel-multiple-assignment**: a `Channel` symbol assigned more than once **on the same code path** within a single script. Control-flow-aware (§5): assignments in mutually-exclusive `if`/`else` branches are NOT a conflict; two unconditional assignments, or one unconditional plus one conditional, ARE. Mirrors CONTRIBUTING "A channel can only be assigned once per code path." Locals are exempt (CONTRIBUTING: locals may be reassigned). |
| **T050** | Warning | **symbol-name-convention** (project audit): a `.m1prj` symbol whose **leaf name** violates the CONTRIBUTING convention for its kind — Channels `lowerCamelCase`, Parameters/Groups `UpperCamelCase`, Constants `CAPITALISATION` (spaces allowed), Functions/Methods `lowerCamelCase`, Enum types `UpperCamelCase`. Emitted by `audit_project`, not the per-script walk (§6). |

(Numbering leaves T012–T019 and T022–T029 free for future intra-statement type
rules; T04x for flow rules; T05x for project-audit rules.)

### 4.1 The single-assignment analysis (T040) in detail — see §5.
### 4.2 The project audit (T050) in detail — see §6.

---

## 5. Control-flow analysis for single-assignment (T040)

### 5.1 The problem

CONTRIBUTING: *"A channel can only be assigned once per code path."* A channel
assigned in both an `if` and its `else` is fine (exactly one runs); assigned twice
unconditionally is a bug; assigned once at top level and again inside an `if` is a
bug (the conditional path assigns twice).

This needs to walk the statement tree and, for each channel, decide whether two
assignments can both execute. The M1 control-flow statements (from `m1-core`'s
`Kind`): `IfStatement` (with optional `ElseClause`), `WhenStatement`,
`ExpandStatement`, and `Block`.

### 5.2 Algorithm — assignment-set merge over the statement tree

Define, for a statement subtree, a set of channel paths **definitely or possibly
assigned** within it, tracking whether each is *guaranteed* on every path or only
on *some* path. Represent as:

```rust
struct AssignFacts {
    // channel path -> how many times it is assigned on the *worst-case* single path
    // through this subtree (the maximum over paths). 2+ on one path == conflict.
    max_per_path: HashMap<String, u32>,
}
```

Walk rules (compose child facts into a parent's):

- **Leaf `AssignmentStatement` to channel `C`** → `{ C: 1 }`.
- **Sequence** (statements in a `Block` / top-level, executed one after another) →
  **add** counts: `merge_seq(a, b)[k] = a[k] + b[k]`. Two unconditional
  assignments to `C` give `C: 2` ⇒ conflict.
- **`if (cond) { THEN } else { ELSE }`** → the two arms are **mutually exclusive**,
  so take the **per-key max**: `merge_alt(then, els)[k] = max(then[k], els[k])`.
  Assigning `C` once in each arm yields `C: 1` (no conflict). An `if` with **no
  `else`** is modelled as `merge_alt(then, {})` — i.e. the "else" path assigns
  nothing — which preserves `then`'s counts (so `C` assigned in a lone `if` counts
  as `1` on the taken path; combined in sequence with a top-level assignment it
  becomes `2` ⇒ conflict, correct).
- **`when (cond) { BODY }`** — `when` blocks re-run on each scheduler tick but
  within one evaluation behave like a conditional body. Treated like a no-`else`
  `if`: `merge_alt(body, {})`.
- **`expand (…) { BODY }`** — macro expansion repeats `BODY`; an assignment inside
  an `expand` to a *fixed* channel path is treated as a single conditional
  assignment (`merge_alt(body, {})`); expansions over channel *families* generate
  distinct targets and are conservatively **excluded** from T040 (the expanded
  target text is one syntactic node; we do not unroll). This avoids false
  positives on the legitimate "assign each member of a group" idiom.

A conflict is reported (one T040 per offending channel, at the **second**
assignment node) when the **top-level** `max_per_path` for a channel is `≥ 2`.

### 5.3 Scope and false-positive guards

- Only **`Channel`** symbols (resolved via the existing resolver) participate;
  locals, parameters, and `Unresolved`/`Opaque` targets are ignored.
- An assignment whose target does not resolve to a known channel is **skipped**
  entirely (no `Unknown`-driven false positive).
- The analysis is **per script file** (single-function bodies); cross-file is out
  of scope (v3).

This is implemented as a separate pass (`flow::single_assignment(root, scope)`)
invoked by the runner, not as a node-at-a-time `Rule` (it needs the whole tree),
and appends its `TypeDiagnostic`s to the same `CheckResult`.

---

## 6. Project-wide symbol-name audit (T050)

A new entry point, distinct from `check_script` (which is per-`.m1scr`):

```rust
/// Audit the project's own symbol names against the naming conventions.
pub fn audit_project(project: &Project) -> Vec<TypeDiagnostic>;
```

It iterates `project.symbols().iter()` and, for each symbol whose kind is one of
{`Channel`, `Parameter`, `Constant`, `Function`, `Method`, `Group`}, checks the
**leaf segment** (the text after the last `.`) against a per-kind convention
predicate. Enum **type** names are audited from `project.symbols().enums()`.

Convention predicates (from EV-M1 `CONTRIBUTING.md` §"Naming Conventions"):

| Kind | Convention | Predicate sketch |
|------|-----------|------------------|
| Channel | `lowerCamelCase`, no spaces | first char lowercase, no space, no `_` |
| Parameter | `UpperCamelCase`, no spaces | first char uppercase, no space, no `_` |
| Group | `UpperCamelCase`, no spaces | first char uppercase, no space |
| Constant | `CAPITALISATION` (spaces allowed) | all-caps letters (spaces/digits OK) |
| Function / Method | `lowerCamelCase` | first char lowercase, no space |
| Enum type | `UpperCamelCase`, no spaces | first char uppercase, no space |

Each violation produces a T050 `TypeDiagnostic` whose message names the symbol,
its kind, and the expected convention. Because `.m1prj` symbols have no source
`Range` (they come from XML, not a `.m1scr`), the diagnostic's `inner.range` is a
zero range (`Range::default()`) and the symbol path goes in the message — the CLI
prints these under the project file name (§7), not a script line.

> **Important false-positive caveat.** The example project massively violates these
> conventions (channels with spaces, `UpperCamelCase` channels, enum types with
> spaces). T050 is therefore **opt-in** via a CLI flag (`--audit-names`, §7) and is
> **off by default** so the per-script CI flow is unaffected. The corpus test
> records a baseline count rather than asserting zero. This keeps the v1 promise
> (no surprising noise) while still delivering the "lint the `.m1prj`" capability.

---

## 7. CLI changes

```
m1-typecheck [--project <Project.m1prj>] [--config <parameters.m1cfg>]
             [--audit-names] <file.m1scr>...
```

- New `--audit-names`: after checking the given scripts, run `audit_project` and
  print its T050 diagnostics as
  `<project-path>: warning[T050]: <message>` (no line/col — project-level).
  Requires a loaded project; if none, prints a stderr note and skips.
- Per-script output is **unchanged** from v1
  (`path:line:col: severity[T0xx]: message`). T020/T021/T030/T040 print exactly
  like the existing T002/T003 (they are ordinary per-script diagnostics).
- Exit codes unchanged: `0` no error-severity diagnostic; `1` ≥1 error; `2`
  invocation error. All new rules are `Warning` severity, so they never by
  themselves cause a non-zero exit (matching v1's warning-only rules).

`--format json` remains deferred (§2).

---

## 8. Architecture deltas

v2 adds files and extends existing ones; no v1 file is rewritten.

```
m1-typecheck/
  src/
    symbols/
      mod.rs          (+ member_index, enum_by_name/enums_with_member/enum_has_member/enum_type)
      m1cfg.rs        (enum cell back-resolution -> set Symbol enum association)
    types.rs          (Enum already present; numeric_join leaves Enum -> Unknown — unchanged)
    typer.rs          (+ typed-member-path -> Enum; Symbol-with-enum -> Enum)
    flow.rs           (NEW: single_assignment control-flow pass; AssignFacts merge)
    audit.rs          (NEW: audit_project + per-kind naming-convention predicates)
    project.rs        (+ pub fn audit(&self) -> Vec<TypeDiagnostic> delegating to audit.rs)
    diagnostics.rs    (+ TypeCode::T020/T021/T030/T040/T050; as_str arms)
    rules/
      mod.rs          (default_v2(): adds t020/t021/t030; runner also calls flow::single_assignment)
      t020_enum_member.rs       (NEW)
      t021_enum_numeric_cmp.rs  (NEW)
      t030_assign_mismatch.rs   (NEW)
    main.rs           (+ --audit-names; calls project.audit())
  tests/
    enums.rs          (NEW: member_index, enum_by_name, association from .m1cfg)
    typer_enum.rs     (NEW: typed-member-path -> Enum)
    rules_v2.rs       (NEW: T020/T021/T030 fixtures, no-false-positive cases)
    flow.rs           (NEW: T040 single-assignment, if/else mutual-exclusion cases)
    audit.rs          (NEW: T050 per-kind convention cases on a hand fixture)
  tests/fixtures/
    enums.m1prj       (NEW: enum types + an enum-associated param via .m1cfg + a channel)
    enums.m1cfg       (NEW)
    naming.m1prj      (NEW: deliberately convention-violating + conforming symbols)
```

### 8.1 Public API additions

```rust
// project.rs
impl Project {
    /// Audit the project's own symbol names (T050). Empty unless violations.
    pub fn audit(&self) -> Vec<crate::diagnostics::TypeDiagnostic>;
}

// rules/mod.rs — check_script signature unchanged; it now also runs the flow pass.
```

`check_script` / `check_script_no_project` signatures are **unchanged**;
T020/T021/T030 flow through the existing registry walk, T040 through an added
flow pass inside `run`.

### 8.2 Data flow (additions in **bold**)

```
Project.m1prj ─→ components + datatypes + **member_index** ┐
parameters.m1cfg ─→ param types + **enum back-resolution → Symbol.enum assoc** ┤→ Project
.m1scr ─m1_core::parse→ Cst ─┐
                             ▼
              Runner: Scope(locals, group, project)
                             │
                ┌────────────┼─────────────────────────┐
                ▼            ▼                           ▼
    rules T001/2/3/10/11   **rules T020/T021/T030**   **flow::single_assignment → T040**
                             (single CST walk)           (whole-tree pass)
                             └──────────┬────────────────┘
                                        ▼
                                Vec<TypeDiagnostic>

audit_project(Project) ──(CLI --audit-names)──→ **T050** Vec<TypeDiagnostic>
```

---

## 9. Testing strategy

- **Enum model** (`tests/enums.rs`): a hand-written `enums.m1prj` with two enum
  types sharing no member names plus one shared-member case → assert
  `enum_by_name`, `enums_with_member` (unique vs ambiguous), and that
  `enums.m1cfg`'s `<Cell Type="enum">OK</Cell>` associates the parameter with the
  unique enum (and an ambiguous member leaves it `Unknown`).
- **Enum typing** (`tests/typer_enum.rs`): `Drive State.Latching Fault` →
  `Enum(id)`; an enum-associated parameter reference → `Enum(id)`; a non-member
  typed path (`Drive State.Nope`) does NOT type as that enum (falls through).
- **T020/T021/T030** (`tests/rules_v2.rs`, project-backed via `enums.m1prj`):
  positive at exact `line:col` + conforming negative with **no** diagnostic.
  No-false-positive cases: `is (RealMember)` (no T020); `enumParam eq
  EnumType.Member` (no T021); `floatChannel = 1.0` (no T030); both-`Unknown`
  assignment (no T030).
- **T040 flow** (`tests/flow.rs`): unconditional double-assign → T040;
  `if/else` each assigning once → **no** T040; lone-`if` plus top-level assign →
  T040; local reassigned twice → **no** T040; assignment to unresolved target →
  **no** T040.
- **T050 audit** (`tests/audit.rs`): `naming.m1prj` with one conforming and one
  violating symbol per kind → assert exactly the violators are flagged with the
  right kind/convention in the message.
- **Corpus gate** (extend `tests/corpus.rs`): still must not panic; record the
  v2 diagnostic counts (now including T020/T021/T030/T040) and, separately, the
  T050 audit count, in the committed baseline. The audit count will be large
  (the example project violates the conventions) — that is expected and is why
  T050 is opt-in (§6).

---

## 10. Relationship to v1 and `m1-lint`

- v1 rules and their semantics are **unchanged**. The registry's `default_v1()`
  is kept (so callers/tests can pin v1) and `default_v2()` is the new default for
  `check_script`.
- `m1-lint` is untouched. T040 is the precise, symbol-aware realisation of the
  deferred `m1-lint` "single-assignment" B-rule; `m1-lint` never gains it (it has
  no symbol model to know what a channel is).
- The `Unknown`-is-silent invariant is preserved end-to-end: every v2 rule short-
  circuits when its required types/enum identities are not known.

---

## 11. Non-goals (v2)

- No return-type inference; calls remain `Unknown` (§2).
- No absolute-vs-relative path-style enforcement (§2).
- No modification of any `.m1prj`/`.m1cfg`/`.m1scr` file (read-only, as v1).
- No soundness/completeness claim for the M1 type or flow system — v2 remains a
  best-effort checker, silent under uncertainty.
- No promotion of the symbol model into `m1-core` and no `--format json` (v3).
