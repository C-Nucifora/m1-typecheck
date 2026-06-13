# AGENTS.md — m1-typecheck

Guidance for coding agents working in this repository.

## Purpose

The semantic layer of the M1 toolchain: it owns the project symbol model
(`.m1prj` / `parameters.m1cfg` / `.m1dbc`), name resolution, the type model,
and every diagnostic that needs to know what a name *is*. `m1-lsp` consumes it
as a library for hover/completion/diagnostics; `m1-ci` runs the CLI as a gate.
Syntax lives below in `m1-core`; style lives beside in `m1-lint` — semantic
questions belong here and nowhere else.

## Things that are deliberate (don't "fix" them)

- **`Unknown` is absorbing and silent.** A rule fires only when every type or
  enum it needs is *known*. This is the false-positive guard that keeps real
  corpora clean: gaps in the symbol data (un-modelled firmware, untyped
  parameters) must produce silence, never a guess. A new rule that flags on
  `Unknown` is wrong by definition, even if it happens to be right on the
  fixture you tested.
- **Opaque roots stay opaque.** Unknown and library roots are never flagged
  as unresolved; only a path rooted in a *known* project group whose leaf is
  absent reports. Same guard, resolution-side.
- **The M1 Development Manual is the spec.** The manual-conformance rules
  cite manual pages; when behaviour and manual disagree, the manual wins —
  the real corpora follow it.
- **Some rules are opt-in on purpose.** Naming audits and scheduling audits:
  real projects legitimately violate them en masse, so default-on would bury
  every actionable diagnostic. Noise policy is part of a rule's design, not an
  afterthought. (The tags audit T092 is the deliberate exception — it is
  **default-on** for M1-Build parity despite its volume; see `audit_tags`.)
- **Project-wide analyses solve over the files passed in.** Cross-script
  taint (NaN provenance) iterates to a fixpoint over the writer→reader
  channel graph; it can only see edges from scripts it was given. That's why
  CI passes the whole project per run.

## Where to look

- `docs/OBJECTS.md` — the M1 object model (`Classname` tree) the symbol table
  implements, including what is and isn't modelled yet. Read it before
  touching `project.rs`/`symbols/`.
- `m1-typecheck --rules` prints the live T-code catalogue; the rule
  implementations live in `src/rules/` (per-node) and as whole-tree passes
  (`flow.rs`, `cross_script.rs`, `schedule.rs`, `audit.rs`).

## Build / test gate

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

CI also runs rustdoc with `-D warnings`, a security audit, and an MSRV job.
The MSRV pin in CI (`dtolnay/rust-toolchain@<version>`) must stay in sync with
`rust-version` in `Cargo.toml` — never bump one without the other.

The corpus gate (`tests/corpus.rs`) needs a real project
(`M1_PROJECT`/`M1_CORPUS_PATH` or a sibling `m1-example/`) and skips when
absent — a green corpus-less run proves less than it appears to.

## Dependencies and releases

Depends on `m1-core` and `m1-workspace` via **versioned git tags** — never
`branch`/`path`/`[patch]`; the repo must build exactly like a public clone,
and everything in one lockfile must pin the same m1-core tag. This is a
binary repo: a version bump on `main` makes `release.yml` tag it and upload
prebuilt binaries (`m1-typecheck` + `m1-cfg-export`). After releasing, open
the consumer bump PRs (`m1-lsp`, and the `m1-ci` tool-version pin) immediately
rather than waiting for Dependabot.
