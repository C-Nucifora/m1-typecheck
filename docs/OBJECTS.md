# M1 object support

M1 Build models a project as a tree of **objects** (the `<Component>` elements in
`Project.m1prj`), each with a `Classname` (its type) and a dotted `Name` (its
path). Two distinct kinds matter to the tooling.

## 1. Project objects — IMPLEMENTED

Instances of a **package class** that live in the project file: sensors, outputs,
IO methods, tables, etc. — e.g.

```xml
<Component Classname="MoTeC Input.Sensor" Name="Root.Driver.Throttle.Main"/>
<Component Classname="BuiltIn.Channel"    Name="Root.Driver.Throttle.Main.Value"/>
<Component Classname="BuiltIn.MethodUser" Name="Root.Driver.Throttle.Main.Calculation"/>
```

The object instance is any component whose `Classname` is **not** `BuiltIn.*`.
Its members are separate components whose path is prefixed by the object's path.

What the model now does (`src/symbols/`):

- `SymbolKind::Object` — the instance is classified as an object (was `Other`).
- `Symbol::class` — holds the package class name (`"MoTeC Input.Sensor"`).
- `SymbolTable::immediate_children(path)` — enumerates an object's direct members
  for hover/completion.

Members already resolve via their full path (a member channel is a
`BuiltIn.Channel` component), so `resolve()` returns them with the right kind and
type. No external data is needed — it's all in `Project.m1prj`.

> Surfacing in editors: m1-lsp vendors m1-typecheck, so it must **re-vendor** this
> crate and add `SymbolKind::Object` arms in `features/hover.rs` (`kind_str`) and
> `features/semantic_tokens.rs` to show objects + complete their members.

## 2. Built-in global objects — IMPLEMENTED (intrinsics catalogue)

The most-called things in real scripts are **built-in/firmware objects** that
are **not in `Project.m1prj` at all**:

```
Calculate.Min(a, b)      CanComms.GetUnsignedInteger(h, 48, 16)
Convert.ToInteger(x)     Delay.Rising(cond)      Output.SetState(s)
```

These are modelled by the vendored intrinsics catalogue,
[`assets/m1-intrinsics.json`], loaded once per process by
[`src/intrinsics.rs`](../src/intrinsics.rs) (`intrinsics::get()`):

- **Resolution** ([`src/resolve.rs`](../src/resolve.rs)): library objects
  resolve before project symbols, per the M1 scope order
  (local → library → project). `Calculate` → `Resolution::BuiltinObject`;
  `Calculate.Min` → `Resolution::BuiltinFn(overloads)`; an **unknown member of
  a known object stays `Opaque`** — that boundary is what keeps
  intrinsic-backed rules safe (see below). A project object that shadows a
  library name is reached via the `This.`/`Library.` anchors.
- **Rules** that consume resolved built-ins:
  [T064](../src/rules/t064_arg_count.rs) wrong-argument-count (union-aware
  across overload arities), [T061/T062/T063](../src/rules/) stateful /
  deprecated / calibration-only usage.
- **Typing**: `BuiltinFn` return types feed the typer, so downstream checks
  (T030 and friends) see through library calls.
- **Editors**: hover, completion, signature help and the diagnostics all go
  through `resolve()`, so m1-lsp surfaces the catalogue with no extra feature
  code.

### Coverage and the T064 graduation question

`tests/intrinsic_coverage.rs` is the tracked coverage report
(`cargo test --test intrinsic_coverage -- --nocapture` with a corpus
available): it walks every corpus script, collects `Object.Method(` call
heads, and classifies them against the catalogue. As of 2026-06-12, **every
library call in both real corpora resolves to a modelled method** (the
remaining unmatched heads are project symbols — timers, outputs, tables — not
intrinsics). T064 nevertheless stays **opt-in** for now: the catalogue is
help-capture-derived rather than firmware-export-derived, so corpora beyond
ours can call methods it misses; the safety boundary (unknown member →
`Opaque`, never flagged) plus the coverage report make the future
graduation decision data-driven rather than hopeful.

### Refreshing the catalogue

The catalogue merges curated entries with M1 Build help-pane captures
(`M1_LIBRARIES_ENUMS_TYPES`): after a new capture set, run
`python3 assets/merge-help-captures.py <captures-dir>`. Curated entries win on
conflict (they carry `stateful`/`deprecated`/`calibrationOnly` flags and
overload unions). Known limits: no firmware-version keying (the catalogue is a
union across captured versions), and no variadic markers beyond what the
captures expose.

> Historical note: this section previously specified a `builtins.json`-based
> design (`Resolution::Builtin`, a `Builtins` registry) as future work. The
> implemented model differs mainly in naming (`BuiltinObject`/`BuiltinFn`, the
> `Intrinsics` loader) and in sourcing the data from help captures instead of
> an M1 Build firmware export — the export remains the ideal upstream if it
> ever becomes available.

## 2026-06-11 — help-pane capture integration

`assets/m1-intrinsics.json` now also carries, merged from the M1 Build
help-pane captures (`M1_LIBRARIES_ENUMS_TYPES`, reconstructed 2026-06-11) via
`assets/merge-help-captures.py`:

- **8 additional libraries / 129 additional functions** (J1939, LTC, MDD,
  MPSE, Switch, TC, UnixTime, VCS + gaps in the existing 15) with full
  signatures and per-argument docs. Existing curated entries win on conflict
  (they carry stateful/deprecated/calibrationOnly flags and overload unions).
- **`enums`** — the builtin enumeration catalogue: 130 MoTeC firmware/module
  enumerated data types with authoritative members (value, name, M1 Tune
  severity, doc). Registered closed into every project's symbol table
  (`add_builtin_enum`, member-index-bypassing) so script literals resolve and
  T020/T021/T030/T070 enforce membership exactly as M1 Build does
  (Errors 1306/1329/1352). Project-local `<Type>` declarations always win.
- **`classes`** — 110 package class help summaries for editor hover.

To refresh after a new capture set:
`python3 assets/merge-help-captures.py <captures-dir>`.
