# m1-typecheck

The semantic-analysis layer of the MoTeC M1 (`.m1scr`) toolchain. Where `m1-core`
(syntax) and `m1-lint` (CST-only style rules) know nothing about what a name *is*,
`m1-typecheck` loads the project's **symbol model** and uses it to resolve
identifiers, type expressions, and emit type/naming diagnostics that are
impossible without that knowledge.

It is both a **library** (consumed by `m1-lsp` for hover/types and as a third
diagnostic source) and a **CLI**.

## Workspace layout

The M1 toolchain lives in **six separate repositories**. They are not published
to crates.io; instead each crate pins its upstreams as **versioned git-tag Cargo
dependencies** (e.g. `m1-core = { git = "…/m1-core.git", tag = "v0.3.1" }`), so
this crate **does** build from a standalone clone — Cargo fetches its upstreams
from their tagged releases. Checking the whole set out as siblings under one
parent directory is handy for cross-repo work, but is not required to build:

```
<parent>/
├── tree-sitter-m1/   # grammar (root)
├── m1-core/          # parse / CST / diagnostics
├── m1-lint/          # linter
├── m1-fmt/           # formatter
├── m1-typecheck/     # this crate
└── m1-lsp/           # language server; depends on the four above
```

**`m1-typecheck` depends on `m1-core` and `m1-workspace`** (git-tag deps;
`m1-core` transitively pulls in `tree-sitter-m1`), and is itself a git-tag
dependency of `m1-lsp`. (The `m1-example` example project, used by the corpus
gate, is an optional sibling checkout — see the env-var notes below.)

Because every dependency is pinned by tag, the coupling **is** visible on
GitHub — each `Cargo.toml` names its upstreams and their versions, and
Dependabot opens bump PRs as new upstream tags ship. Cutting a new upstream
release and bumping `tag = "vX.Y.Z"` in each consumer is what propagates a
change across the stack.

## The symbol model

`m1-typecheck` parses the project's MoTeC `Project.m1prj` (XML) into a flat
`SymbolTable` keyed by fully-qualified dotted path (`Root.Foo.Bar`). Each symbol
carries a `SymbolKind` (Channel, Parameter, Constant, Function, Method, Table,
Group, Reference, Other), derived from the component `Classname`. Enum data types
(`<DataTypes>`) become `EnumType`s.

### `parameters.m1cfg` — the parameter type source

The `.m1prj` names parameters but rarely says what *type* they hold. The sibling
**`parameters.m1cfg`** is where that lives: each `<Parameter><Cell Type="…"
Unit="…">` gives a parameter its concrete **value type** (`f32`→`Float`,
`u16`→`Unsigned`, `s16`→`Integer`, `bool`→`Boolean`, `enum`→a specific enum type)
and its **unit**. Loading it is what turns parameters from untyped names into
typed symbols, which is what powers:

- assignment type-mismatch checking (`T030`) against parameter targets,
- enum association of `enum`-typed cells (see below),
- richer hover/completion in the language server (type + unit).

It is **auto-discovered**: the first `*.m1cfg` found by searching the project
file's directory and its ancestors (nearest wins) is loaded automatically (the
CLI, the language server, and therefore m1-ci all do this), so a project that
ships a `parameters.m1cfg` gets type-aware checking with no extra configuration.
`--config` overrides the discovered file.

`FuncUser`/`MethodUser` components carry a `Filename`, mapping each script to its
**enclosing group** — the base for relative name resolution.

### Name resolution

A reference resolves as: local → absolute (and `Root.`-prefixed) → group-relative
(walking ancestor groups) → built-in/opaque root. Unknown and library roots are
treated as **opaque** (never flagged); an accessor on a resolved channel/parameter
or value-bearing compound (e.g. `SomeChannel.AsInteger`) is also opaque. Only a
path rooted in a known *namespace* project group whose leaf is absent is reported
unresolved — this is the false-positive guard.

### Type model

`ValueType` is `Boolean | Integer | Unsigned | Float | Enum | String | Unknown`.
`Unknown` is absorbing and silent: a rule fires only when all operands it needs
are *known*, eliminating false positives from gaps in the symbol data. Locals are
typed by Hungarian prefix (`b`/`u`/`i`/`f` + UpperCase).

## Rules

| Code | Severity | Rule |
|------|----------|------|
| T001 | Warning | **unresolved-reference** — member path whose root is a known project group does not resolve. |
| T002 | Error | **float-equality** — `==`/`!=`/`eq`/`neq` with a `Float` operand; use a tolerance check. |
| T003 | Warning | **float→integer narrowing** — a compound assignment (`+=` `-=` `*=` `/=`) storing a `Float` value back into an integral target, which truncates. Plain integer/float *widening* in arithmetic is implicit in M1 Build and not flagged. |

## v2: enum domain model, flow analysis, and the project audit

v2 builds a live **enum domain model** on top of the v1 symbol table:

- A **member index** (member name → enum type ids) is built once at load, with
  queries `enum_by_name`, `enums_with_member`, `enum_has_member`, `enum_type`.
- **Enum association**: a `.m1cfg` parameter cell `<Cell Type="enum">…</Cell>`
  back-resolves its member value to a *unique* enum type and associates the
  parameter with it (zero or many candidates → left `Unknown`, silent).
- The **typer** produces `ValueType::Enum(id)` for a typed member path
  `<EnumTypeName>.<Member>` (when the head names an enum that declares the member)
  and for any enum-associated symbol.

This unlocks four new rules, added to the default rule set
(`Registry::default()`) that `check_script` runs:

| Code | Severity | Rule |
|------|----------|------|
| T020 | Warning | **enum-non-member** — a typed member path `<EnumType>.<Member>` whose member the enum does not declare. |
| T021 | Warning | **enum-numeric-comparison** — a comparison with one *known* `Enum` operand and the other *known* numeric. |
| T004 | Warning | **signed-unsigned-comparison** — an ordering comparison (`<` `>` `<=` `>=`) mixing a known `Unsigned` operand with a known signed `Integer`, where the result can flip on wraparound. Narrow by design: ordering only, and literal operands are allowed. |
| T030 | Warning | **assignment-type-mismatch** — `target = value` where the target is a Channel/Parameter with a known type and the value's known type is incompatible. |
| T031 | Warning | **unresolved-assignment-target** — the target of a plain `=` assignment is a project-rooted path that does not resolve (e.g. `Root.Foo.Missing = 1`). Distinct from T001 because a write target is not a read; compound assignments (`+=`, …) read the target first and stay T001. |
| T040 | Warning | **channel-multiple-assignment** — a Channel assigned more than once on a single code path (control-flow aware: mutually-exclusive `if`/`else` arms are not a conflict). Implemented as a whole-tree pass in `flow.rs`, not a node-at-a-time rule. |
| T070 | Error | **when-is-exhaustive** — a `when (subject) { is (…) … }` over an enum-typed subject that does not cover every enumerator (`or` groups enumerators within one arm). One of the remaining §6 compile-time checks M1 Build enforces. An arm naming a non-member label is treated as a catch-all (the `when` is then exhaustive); a non-enum subject is T082's finding. |
| T080 | Error / Warning | **invalid-value-reaches-finite-sink** — NaN/Inf provenance tracing (see below). Fires only for a value marked `@m1:requires-finite` / `@m1:safety-critical`; otherwise silent. |
| T085 | Error / Warning | **user-argument-mismatch** — a call to a user function whose `.m1prj` `<Signature>` declares parameters: wrong arity is an Error, an incompatible argument type a Warning. `In.<Param>` reads inside the callee are typed from the same signature. |
| T086 | Warning | **unit-mismatch** — a direct channel-to-channel copy whose two sides carry different physical quantities (base units from `Props Qty`). Computed right-hand sides are unchecked (arithmetic legitimately changes dimension). |
| T088 | Warning (opt-in) | **circular-dependency** — same-rate write/read cycles between scripts (manual p.29). Opt-in: real projects contain deliberate same-rate feedback loops M1 Build accepts. |
| T089 | Hint (opt-in) | **rate-inversion** — a faster script reads a channel written only at a slower rate (stale between writer ticks). Opt-in: often intentional downsampling. |
| T097 | Error | **recursive-call** — a cycle in the user-function call graph: a script calling its own function, or mutual recursion through other user functions (self-calls included). Scripts are event-scheduled functions on a fixed-stack runtime; a call cycle can never complete. |
| T082 | Error | **when-subject-not-enum** — the manual (p.32) requires the `when` argument to be an enumerated data type; fires when the subject's type is *known* and non-enum. |
| T083 | Error | **static-local-initialiser** — a `static local`'s initial value must be a literal, enumerator or constant (manual p.34); runtime reads and calls flag. Constant-foldable literal arithmetic (`2 * 60`) is accepted. |
| T084 | Error | **expand-bounds** — `expand` bounds must be literals or constants, 0 or positive (manual p.33); negative, float and runtime-value bounds flag. |
| T087 | Warning | **type-restricted-to-locals** — project audit: a Channel/Parameter declared `bool`/`string`, which the manual (p.24) restricts to local variables. CAN/DBC signals are exempt. |
| T091 | Warning | **local-object-case-ambiguity** — a `local` whose name matches the leaf of a project object referenced in the same script, identically or differing only by case (manual pp.64–65: "don't distinguish local variable names from other object names only by uppercase/lowercase writing"). Package-object internals are exempt. |
| T092 | Warning (opt-in) | **untagged-component** — M1 Build tag-warning parity (manual p.67): a user Channel/Parameter whose effective tag set (own `SelectedTags` ∪ inherited group tags) has no System tag (Engine/Vehicle/Driver) and/or no Type tag (Normal/Diagnostic/Advanced/Pin/Tune/Setup); one finding per missing group. Opt-in: the real corpora use no tags at all. |

Every v2 rule keeps the v1 invariant: it fires only when every type/enum it needs
is **known**, and stays silent under `Unknown` — so the m1-example corpus stays free of
spurious diagnostics.

### T080 invalid-value (NaN/Inf) provenance tracing

M1 floats are IEEE 754 binary32 with **no "no-data" sentinel**, so an invalid
result is a genuine NaN/±Inf that propagates silently (or, scaled to an int for
CAN/logging, reads as an extreme "legitimate" value). The type lattice can't see
this. **T080** tracks a *can-be-invalid* bit and flags an unsanitised invalid
value reaching a finiteness sink.

It is **annotation-driven** (zero-noise unless opted in):

```m1
// @m1:source(volatile)         <- a sensor channel that can drop out → NaN
Gyro Z = Sbg.Read();

// @m1:safety-critical          <- this value MUST stay finite (error if not)
Front Axle Torque = Yaw Moment / Track Width;   // T080: division can be NaN/Inf
```

- **Sources** (can-be-invalid): division, domain math (`Calculate.FastSquareRoot`,
  `Math.Log`, …), explicit `Calculate.Infinity()`, and reads of an
  `@m1:source`/`@m1:external` channel.
- **Sanitiser** (clears the bit): an assignment annotated `@m1:sanitizes` /
  `@m1:clears`. Note `Math.IsNaN()` is **calibration-only** per the Development
  Manual, so it cannot guard an ECU script. **Clamps do _not_ sanitise** —
  `Limit.Range`/`Calculate.Min`/`Calculate.Max` pass NaN straight through (NaN
  defeats min/max).
- **Latching escalation:** a tainted value reaching a stateful function
  (`Filter.*`/`Integral.*`/`Derivative.*`) is escalated to an **error** — the
  filter state stays poisoned after the input recovers.

Severity: `@m1:safety-critical` → error (fails the build); `@m1:requires-finite`
→ warning, escalated to error when latched. Because m1-typecheck exits non-zero
on any error, this **is** the CI gate — no separate wiring.

#### Cross-script propagation

The analysis is **project-wide**: every run with a project solves the
writer→reader channel graph over all the scripts passed on the command line
(pass the whole project, as CI does, so the solve sees every edge). A channel
written from an invalid-value source in one script taints its reads everywhere
else, through any depth of copies — the solve iterates to a **fixpoint** over a
finite monotone lattice, so even feedback loops between scripts terminate. An
`Out =` assignment writes the script's backing function symbol (the same
`Filename=`/path convention used for return-type inference), and a call to that
function reads it, so taint also flows through user-function results.
Diagnostics carry the full backward provenance chain:

```
warning[T080]: a value required to be finite can be NaN/Inf: `Root.Demo.Heading` is
written by `Demo.Update.m1scr` from `Root.Demo.Rate`; `Root.Demo.Rate` is written by
`Demo.Read.m1scr`: division (the denominator may be 0) — guard it with Math.IsNaN()
or a finite fallback
```

`@m1:sanitizes`/`@m1:clears` is the cross-script barrier too (a sanitised
rewrite stops the chain), latching escalates across scripts the same way, and
**T081** also fires cross-script when a remotely-tainted float is stored into an
integer channel. Function argument→`In` flow is not yet modelled (only `Out` and
channel reads propagate). Query one channel's solved status with
[`--explain`](#cli-usage).

### T050 project-name audit (opt-in)

`--audit-names` runs a project-wide symbol-name audit (`Project::audit()`),
checking each symbol's leaf name against the m1-example naming conventions: Channels and
Functions/Methods `lowerCamelCase`, Parameters/Groups and enum types
`UpperCamelCase`, Constants `CAPITALISATION` (spaces allowed). It emits **T050**
diagnostics at the project level (no source line/col). It is **off by default**:
the example project massively violates these conventions, so running it as part of
the per-script CI flow would be pure noise. The corpus gate records only a baseline
count for it.

The same audit also emits **T071 name-case-collision** (Warning, the other
remaining §6 compile-time check): (a) any two project symbols whose fully-qualified
dotted paths are equal case-insensitively but differ in actual case (reported once
per colliding pair), and (b) any project symbol whose leaf name case-insensitively
matches a firmware library object name (`Calculate`, `CanComms`, …), which it would
shadow. Both flow through `Project::audit()` / `--audit-names` alongside T050.

The audit also emits **T010 invalid-component-parent** (Warning): a component
whose `Classname` is only valid under a specific parent class but whose actual
parent has a different class. The class schema (`classname.rs`) is deliberately
conservative — only classes with an unambiguous nesting invariant are constrained
(today the CAN frame hierarchy: a `BuiltIn.CAN.Signal` must sit under a
`BuiltIn.CAN.Message`, a `BuiltIn.CAN.Message` under a `BuiltIn.CAN.DBC`), so
well-formed projects never flag. Every component's raw `Classname` is stored on
`Symbol::classname`, ready for richer class-aware checks and completion.

## Suppressing a diagnostic — `// @m1:allow(...)`

A `// @m1:allow(T0xx, …)` annotation (the toolchain annotation framework,
m1-core#33) suppresses the listed checks on the construct it attaches to; a bare
`// @m1:allow` suppresses every check there.

```m1
local b = 1.5;
local c = 2.5;
// @m1:allow(T002)
if (b == c) { }            // T002 (float ==) not reported here
```

It attaches **leading** (the next statement, stacking on consecutive lines) or
**trailing** (a statement it follows on the same line); suppression is
line-scoped to the target construct. Project-level findings (T010/T041/T050/T071/T087/T092, not tied
to a source construct) are **not** suppressible this way — quiet a whole code
with `--ignore` / the `[diagnostics]` config, or suppress a single symbol with
`[diagnostics] ignore_symbols = ["T050:Root.Engine.Speed"]` in `m1-tools.toml`
(or the `--ignore-symbol` flag).

## CLI usage

```
m1-typecheck [--project <Project.m1prj>] [--config <parameters.m1cfg>]
             [--audit-names] [--select <CODES>] [--ignore <CODES>] [--ignore-symbol <CODE:PATH>]
             [--format human|json|sarif] [--rules] [--explain <CHANNEL>] <file.m1scr>...
```

```sh
m1-typecheck --project Project.m1prj --config parameters.m1cfg Scripts/*.m1scr
m1-typecheck --project Project.m1prj --audit-names
m1-typecheck --ignore T041 Scripts/*.m1scr        # silence the cfg-coverage audit
m1-typecheck --audit-names --ignore-symbol T050:Root.GPS  # allow one naming exception
m1-typecheck --format json Scripts/*.m1scr        # machine-parsable diagnostics
m1-typecheck --format sarif Scripts/*.m1scr       # SARIF 2.1.0 for code scanning
m1-typecheck --select T064 Scripts/*.m1scr        # opt into wrong-argument-count
m1-typecheck --select T088,T089 Scripts/*.m1scr   # scheduling audit (cycles + rate inversions)
m1-typecheck --select T092 Scripts/*.m1scr        # M1 Build tag-warning parity (untagged components)
m1-typecheck --explain-units "Driveline.Accumulator.Voltage" Scripts/*.m1scr
m1-typecheck --rules                              # list every T-code
m1-typecheck --explain Demo.Rate Scripts/*.m1scr  # trace a channel's NaN provenance
```

- `--project` defaults to the nearest `Project.m1prj` upward from the first input
  file, or `$M1_PROJECT`. With no project found, runs in project-less mode (T001
  disabled).
- `--config` defaults to the first `*.m1cfg` found by searching the project
  file's directory and its ancestors (nearest wins), or `$M1_CONFIG`; pass it
  explicitly to override. Without a cfg, parameters are untyped (`Unknown`) and
  type-dependent rules stay silent for them.
- When both a project **and** a `.m1cfg` are loaded, the run also emits **T041**
  (`missing-cfg-parameter`) once per `BuiltIn.Parameter` that the `.m1prj`
  declares but the `.m1cfg` omits — M1-Build falls back to its default value for
  those. Hint severity (#156): riding the default is normal, supported M1
  behaviour — most parameters in a real project intentionally stay uncalibrated
  — so at hundreds per healthy project a Warning would bury every actionable
  diagnostic. Teams that require full calibration coverage can still surface the
  set via `--select T041` or `m1-cfg-export --missing-only` (same set).
- `.m1dbc` files under the project directory are auto-loaded, so CAN signals
  resolve. **T042** (`dbc-signal-range`) then flags a **literal** value assigned
  to a signal that falls outside its physical range (derived from the signal's
  `Length`/`Type` raw range scaled by `Multiplier`/`Offset`). Computed RHS values
  are left alone (unknown at check time).
- `--audit-names` prints the project-name audit (T050 and T071) as
  `<project-path>: warning[T0xx]: <message>`; requires a loaded project.
- When a project loads but **no** `parameters.m1cfg` (or no `.m1dbc`) is
  discovered, a `note:` line on stderr announces that **T041** (calibration
  coverage) / **T042** (dbc-signal-range) are skipped — so a degraded CI run is
  visible rather than silently green, mirroring the project-less mode note.
- `--select <CODES>` / `--ignore <CODES>` (comma-separated T-codes) filter the
  reported diagnostics, applying the same `[diagnostics]` semantics as the unified
  `m1-tools.toml` the editors read (the nearest `m1-tools.toml` `[diagnostics]`
  `ignore`/`select` is honoured too; explicit flags override the file). A non-empty
  `--select` runs only the listed codes; `--ignore` drops the listed codes.
- `--ignore-symbol <CODE:PATH>` (comma-separated `CODE:Symbol.Path` entries)
  suppresses a project-level code for exactly one symbol; the
  `[diagnostics] ignore_symbols` list in `m1-tools.toml` is honoured the same
  way (flags replace the file list).
- `--rules` prints the T-code catalogue (with `--format json`, as JSON) and exits.
- `--explain <CHANNEL>` prints one channel's solved project-wide invalid-value
  (NaN/Inf) status and exits: the backward provenance chain when tainted (plus a
  latch note), or a clean note otherwise. Accepts the canonical
  `Root.`-qualified or the bare channel path; pass every project script as
  FILES so the solve sees the whole graph. With `--format json`, emits
  `{"version":1,"explain":{"channel":…,"invalid":…,"latched":…,"chain":[…]}}`.
  A query, not a check: exits `0` either way, `2` for a missing project or an
  unknown channel.
- `--format sarif` emits a SARIF 2.1.0 document (the format GitHub code
  scanning ingests, mirroring `m1-lint --format sarif`): one reportingDescriptor
  per T-code, one result per finding; project-level audits (T041/T050/T092/…)
  anchor to the `.m1prj` at line 1.
- `--format json` emits one machine-parsable document
  (`{"version":1,"files":[…],"project":[…],"summary":{…}}`) instead of the human
  lines; syntax errors and project audits are included.
- Opt-in rule **T064** (`wrong-argument-count`) — flags a call to a fully-modelled
  library method whose argument count matches none of that method's overload
  arities (union-aware; e.g. `Change.By` accepts 2 or 3). Off by default; enable
  with `--select T064` (or `[diagnostics] select`). It never flags unknown members
  (valid firmware absent from the intrinsics export stays opaque, never reported).
- Output: `path:line:col: severity[T0xx]: message`. Syntax errors print first.
- Exit codes: `0` no errors, `1` ≥1 error, `2` invocation error. Most v2 rules are
  `Warning` severity; the exception is **T070** (`Error`), which — like a syntax
  error — causes a non-zero exit.

## `m1-cfg-export` — export the expected `.m1cfg` parameter list

A second binary reuses the symbol model to generate the `.m1cfg` parameter list a
project expects ([C-Nucifora/m1-tools#10](https://github.com/C-Nucifora/m1-tools/issues/10)).
Use it to seed a new calibration file, as a diff target for drift, or — with
`--missing-only` — as a CI check for parameters absent from the committed `.m1cfg`.

```
m1-cfg-export [--output <file>] [--format cfg|json|csv]
              [--baseline <existing.m1cfg>] [--missing-only] <Project.m1prj>
```

```sh
m1-cfg-export Project.m1prj                                  # full skeleton (real .m1cfg XML)
m1-cfg-export Project.m1prj --format csv -o params.csv       # every parameter as a row
m1-cfg-export Project.m1prj --missing-only --baseline parameters.m1cfg
```

- Output is the real MoTeC `.m1cfg` XML (`<Configuration><Group><Parameter><Cell
  Type=… Unit=…>`), with names *un*prefixed (`Foo.Bar`, not `Root.Foo.Bar`), or
  `json`/`csv` (which also carry the `.m1prj` `security`, not part of a `.m1cfg`).
- A parameter's cell `Type` is the exact storage type the `.m1prj` declares
  (`f32`/`u32`/`s16`/…), `enum` for any enum-typed parameter (including
  package-qualified types like `MoTeC Types.Mode Enumeration`), or the type a
  `--baseline`/auto-discovered `.m1cfg` back-resolves. Parameters with no type in
  either source (e.g. the `*.Value` of an untyped reference that isn't yet
  calibrated) are emitted with a flagged `Type=""` placeholder rather than a
  guessed type.
- `--baseline` is required by `--missing-only`; otherwise a sibling
  `parameters.m1cfg` is auto-discovered (as the type checker does) purely to
  enrich types.
- Scope: exports `BuiltIn.Parameter` components. `BuiltIn.IOResourceConstant`
  resource selections (which a real `.m1cfg` also lists) are not included yet.

## Environment variables

- `M1_PROJECT` — path to `Project.m1prj` (overrides discovery; used by the corpus
  gate).
- `M1_CONFIG` — path to `parameters.m1cfg` (overrides the sibling auto-discovery).
- `M1_CORPUS_PATH` — directory of `.m1scr` scripts for the corpus gate
  (`tests/corpus.rs`), which loads the project and checks every script without
  panicking. The gate is skipped if neither the env vars nor the sibling example
  project are present.

## Note on examples

Example identifiers in the docs and fixtures are **synthetic placeholders**, not
drawn from any real project.

## License

Licensed under the GNU General Public License v3.0 or later (GPL-3.0-or-later) — see [LICENSE](LICENSE).

Copyright (C) 2026 The M1 Tools authors.

## Trademark

Independent, community-built open-source tooling for the MoTeC® M1 script
language. Not affiliated with, authorised, or endorsed by MoTeC Pty Ltd.
"MoTeC" and "M1" are trademarks of MoTeC Pty Ltd.
