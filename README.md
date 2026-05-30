# m1-typecheck

The semantic-analysis layer of the MoTeC M1 (`.m1scr`) toolchain. Where `m1-core`
(syntax) and `m1-lint` (CST-only style rules) know nothing about what a name *is*,
`m1-typecheck` loads the project's **symbol model** and uses it to resolve
identifiers, type expressions, and emit type/naming diagnostics that are
impossible without that knowledge.

It is both a **library** (consumed by `m1-lsp` for hover/types and as a third
diagnostic source) and a **CLI**.

## Workspace layout

The M1 toolchain lives in **six separate repositories** coupled through Cargo
**path** dependencies. They are not published to crates.io, so this crate does
**not** build from a standalone clone — check out the whole set as siblings under
one parent directory:

```
<parent>/
├── tree-sitter-m1/   # grammar (root)
├── m1-core/          # parse / CST / diagnostics
├── m1-lint/          # linter
├── m1-fmt/           # formatter
├── m1-typecheck/     # this crate
└── m1-lsp/           # language server; depends on the four above
```

**`m1-typecheck` depends on `../m1-core`** (`m1-core = { path = "../m1-core" }`,
which in turn needs `../tree-sitter-m1`), so both must be checked out alongside
it. It is in turn depended on by `m1-lsp`. (The `m1-example` example project, used by
the corpus gate, is an optional further sibling — see the env-var notes below.)

Because the repos are independent on GitHub, this coupling is **not visible
there**: each repo's CI and PRs see only itself. Build/merge ordering across the
stack is a manual, local-workspace concern.

## The symbol model

`m1-typecheck` parses the project's MoTeC `Project.m1prj` (XML) into a flat
`SymbolTable` keyed by fully-qualified dotted path (`Root.Foo.Bar`). Each symbol
carries a `SymbolKind` (Channel, Parameter, Constant, Function, Method, Table,
Group, Reference, Other), derived from the component `Classname`. Enum data types
(`<DataTypes>`) become `EnumType`s. The optional sibling `parameters.m1cfg`
augments parameter symbols with concrete value types (`f32`, `u16`, `enum`, …)
and units.

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
| T003 | Warning | **int-float-mixing** — arithmetic mixing `Float` with `Integer`/`Unsigned` without an explicit conversion. |
| T010 | Warning | **local-hungarian-prefix** — a `local` lacking a recognised type prefix. |
| T011 | Warning | **hungarian-prefix-mismatch** — a `local` whose prefix disagrees with its initialiser's known type. |

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

This unlocks four new rules (added to the new `Registry::default_v2()`, which is
now the default for `check_script`; `default_v1()` is kept for callers that want
to pin the v1 rule set):

| Code | Severity | Rule |
|------|----------|------|
| T020 | Warning | **enum-non-member** — a typed member path `<EnumType>.<Member>` whose member the enum does not declare. |
| T021 | Warning | **enum-numeric-comparison** — a comparison with one *known* `Enum` operand and the other *known* numeric. |
| T030 | Warning | **assignment-type-mismatch** — `target = value` where the target is a Channel/Parameter with a known type and the value's known type is incompatible. |
| T040 | Warning | **channel-multiple-assignment** — a Channel assigned more than once on a single code path (control-flow aware: mutually-exclusive `if`/`else` arms are not a conflict). Implemented as a whole-tree pass in `flow.rs`, not a node-at-a-time rule. |

Every v2 rule keeps the v1 invariant: it fires only when every type/enum it needs
is **known**, and stays silent under `Unknown` — so the m1-example corpus stays free of
spurious diagnostics.

### T050 project-name audit (opt-in)

`--audit-names` runs a project-wide symbol-name audit (`Project::audit()`),
checking each symbol's leaf name against the m1-example naming conventions: Channels and
Functions/Methods `lowerCamelCase`, Parameters/Groups and enum types
`UpperCamelCase`, Constants `CAPITALISATION` (spaces allowed). It emits **T050**
diagnostics at the project level (no source line/col). It is **off by default**:
the example project massively violates these conventions, so running it as part of
the per-script CI flow would be pure noise. The corpus gate records only a baseline
count for it.

## CLI usage

```
m1-typecheck [--project <Project.m1prj>] [--config <parameters.m1cfg>]
             [--audit-names] <file.m1scr>...
```

```sh
m1-typecheck --project Project.m1prj --config parameters.m1cfg Scripts/*.m1scr
m1-typecheck --project Project.m1prj --audit-names
```

- `--project` defaults to the nearest `Project.m1prj` upward from the first input
  file, or `$M1_PROJECT`. With no project found, runs in project-less mode (T001
  disabled).
- `--audit-names` prints the project-name audit (T050) as
  `<project-path>: warning[T050]: <message>`; requires a loaded project.
- Output: `path:line:col: severity[T0xx]: message`. Syntax errors print first.
- Exit codes: `0` no errors, `1` ≥1 error, `2` invocation error. All v2 rules are
  `Warning` severity, so they never by themselves cause a non-zero exit.

## Environment variables

- `M1_PROJECT` — path to `Project.m1prj` (overrides discovery; used by the corpus
  gate).
- `M1_CORPUS_PATH` — directory of `.m1scr` scripts for the corpus gate
  (`tests/corpus.rs`), which loads the project and checks every script without
  panicking. The gate is skipped if neither the env vars nor the sibling example
  project are present.

## Note on examples

Example identifiers in the docs and fixtures are **synthetic placeholders**, not
drawn from any real project.
