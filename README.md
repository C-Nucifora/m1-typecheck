# m1-typecheck

The semantic-analysis layer of the MoTeC M1 (`.m1scr`) toolchain. Where
`m1-core` (syntax) and `m1-lint` (style) know nothing about what a name *is*,
`m1-typecheck` loads the project's **symbol model** and uses it to resolve
identifiers, type expressions, and emit diagnostics that are impossible
without that knowledge — unresolved references, type mismatches, non-exhaustive
`when` blocks, NaN propagation into safety-critical values, and more.

It is both a **library** (consumed by `m1-lsp` for hover, completion, and as a
diagnostic source) and a **CLI** (the type-checking gate in
[m1-ci](https://github.com/C-Nucifora/m1-ci)).

## Install

Prebuilt binaries for Linux, macOS, and Windows are attached to each
[release](https://github.com/C-Nucifora/m1-typecheck/releases). Or build from
source:

```sh
cargo install --git https://github.com/C-Nucifora/m1-typecheck.git --tag <latest>
```

## Usage

```sh
m1-typecheck Scripts/*.m1scr                     # project + cfg auto-discovered
m1-typecheck --audit-names Scripts/*.m1scr       # opt-in project naming audit
m1-typecheck --format sarif Scripts/*.m1scr      # SARIF for GitHub code scanning
m1-typecheck --explain Demo.Rate Scripts/*.m1scr # trace a channel's NaN provenance
m1-typecheck --explain-units Demo.Rate Scripts/*.m1scr # trace a channel's unit provenance
m1-typecheck --rules                             # the full T-code catalogue
```

The nearest `Project.m1prj` and `parameters.m1cfg` are discovered
automatically from the input files (`.m1dbc` CAN definitions too), so a real
project gets full type-aware checking with no flags. Pass the whole project's
scripts in one run — cross-script analyses solve over every file they're
given. Output formats: human, JSON, SARIF. Exit code is non-zero on any
error-severity finding, so the CLI is directly usable as a CI gate.

For CI gating, `--strict` fails the run on **any** finding (not just
error-severity ones), and `-W`/`--no-warnings` drops warning-severity findings
entirely (so a team can gate on errors while triaging warnings). Per-symbol
suppression is `--ignore-symbol CODE:Symbol.Path` (overlaying the
`[diagnostics] ignore_symbols` list from `m1-tools.toml`).

Run `m1-typecheck --rules` for the complete catalogue of checks with
descriptions, and `--help` for all flags (rule selection, per-symbol ignores,
explicit project/config paths).

## How it works

The `.m1prj` (plus `parameters.m1cfg` for parameter value types and units, and
any `.m1dbc` files for CAN signals) is parsed into a symbol table keyed by
fully-qualified dotted path. References resolve the way M1 Build resolves
them: local → absolute → group-relative → built-in.

The central design rule is that **`Unknown` is absorbing and silent**: a check
fires only when every type or enum it needs is *known*, so gaps in the symbol
data (un-modelled firmware libraries, untyped parameters) produce silence, not
false positives. Real-world corpora run clean by construction, and every
diagnostic you do see is grounded in something the project actually declares.

The checks cover name resolution, type compatibility, the compile-time rules
the M1 Development Manual specifies (exhaustive `when` over enums, `static
local` initialisers, `expand` bounds, …), control-flow checks like
multiple-assignment, scheduling audits, and an annotation-driven NaN/Inf
provenance analysis: mark a channel `// @m1:safety-critical` and any
unsanitised invalid value that can reach it — including through channels
written by *other* scripts — is reported with its full backward provenance
chain.

Individual findings can be suppressed in source with `// @m1:allow(T0xx)` on
the construct, or filtered project-wide via the `[diagnostics]` section of the
workspace `m1-tools.toml` (see the
[m1-tools configuration docs](https://github.com/C-Nucifora/m1-tools#configuration)).

## `m1-cfg-export`

A second binary that reuses the symbol model to export the parameter list a
project expects as `.m1cfg` XML, JSON, or CSV — to seed a new calibration
file, diff for drift, or (with `--missing-only`) gate CI on calibration
coverage.

## Development

The CI gate is `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
`cargo fmt --all -- --check`. A corpus gate (`tests/corpus.rs`) checks every
script of a real project pointed to by `M1_PROJECT`/`M1_CORPUS_PATH` (or a
sibling `m1-example/` checkout) and skips when absent. `docs/OBJECTS.md`
documents the M1 object model the symbol table implements. Example
identifiers in docs and fixtures are synthetic placeholders, not drawn from
any real project.

## License

GPL-3.0-or-later — see [LICENSE](LICENSE).

## Trademark

Independent, community-built open-source tooling for the MoTeC® M1 script
language. Not affiliated with, authorised, or endorsed by MoTeC Pty Ltd.
"MoTeC" and "M1" are trademarks of MoTeC Pty Ltd.
