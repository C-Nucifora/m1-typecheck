# m1-typecheck

The semantic-analysis layer of the MoTeC M1 (`.m1scr`) toolchain. Where `m1-core`
(syntax) and `m1-lint` (CST-only style rules) know nothing about what a name *is*,
`m1-typecheck` loads the project's **symbol model** and uses it to resolve
identifiers, type expressions, and emit type/naming diagnostics that are
impossible without that knowledge.

It is both a **library** (consumed by `m1-lsp` for hover/types and as a third
diagnostic source) and a **CLI**.

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

## CLI usage

```
m1-typecheck [--project <Project.m1prj>] [--config <parameters.m1cfg>] <file.m1scr>...
```

```sh
m1-typecheck --project Project.m1prj --config parameters.m1cfg Scripts/*.m1scr
```

- `--project` defaults to the nearest `Project.m1prj` upward from the first input
  file, or `$M1_PROJECT`. With no project found, runs in project-less mode (T001
  disabled).
- Output: `path:line:col: severity[T0xx]: message`. Syntax errors print first.
- Exit codes: `0` no errors, `1` ≥1 error, `2` invocation error.

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
