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

It is **auto-discovered**: the first `*.m1cfg` sibling of the project file is
loaded automatically (the CLI, the language server, and therefore m1-ci all do
this), so a project that ships a `parameters.m1cfg` gets type-aware checking with
no extra configuration. `--config` overrides the discovered file.

`FuncUser`/`MethodUser` components carry a `Filename`, mapping each script to its
**enclosing group** — the base for relative name resolution.

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
| T070 | Error | **when-is-exhaustive** — a `when (subject) { is (…) … }` over an enum-typed subject that does not cover every enumerator (`or` groups enumerators within one arm). One of the remaining §6 compile-time checks M1 Build enforces. An arm naming a non-member label is treated as a catch-all (the `when` is then exhaustive); a non-enum subject is skipped silently. |

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

The same audit also emits **T071 name-case-collision** (Warning, the other
remaining §6 compile-time check): (a) any two project symbols whose fully-qualified
dotted paths are equal case-insensitively but differ in actual case (reported once
per colliding pair), and (b) any project symbol whose leaf name case-insensitively
matches a firmware library object name (`Calculate`, `CanComms`, …), which it would
shadow. Both flow through `Project::audit()` / `--audit-names` alongside T050.

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
- `--config` defaults to the first `*.m1cfg` sibling of the project file (or
  `$M1_CONFIG`); pass it explicitly to override. Without a cfg, parameters are
  untyped (`Unknown`) and type-dependent rules stay silent for them.
- `--audit-names` prints the project-name audit (T050 and T071) as
  `<project-path>: warning[T0xx]: <message>`; requires a loaded project.
- Output: `path:line:col: severity[T0xx]: message`. Syntax errors print first.
- Exit codes: `0` no errors, `1` ≥1 error, `2` invocation error. Most v2 rules are
  `Warning` severity; the exception is **T070** (`Error`), which — like a syntax
  error — causes a non-zero exit.

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
