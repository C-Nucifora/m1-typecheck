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

## 2. Built-in global objects — TODO (how to add)

The most-called things in real scripts are **built-in/firmware objects** that are
**not in `Project.m1prj` at all**:

```
Calculate.Min(a, b)      CanComms.GetUnsignedInteger(h, 48, 16)
Convert.ToInteger(x)     Delay.Rising(cond)      Output.SetState(s)
```

`resolve()` currently returns `Resolution::Opaque` for these (silently OK, but no
type, no method signature, no completion, no argument checking). They come from
M1 Build's **firmware/SDK**, versioned by `<System VersionBuild="…">`.

### Recommended design

**a) A definitions registry.** Add `src/symbols/builtins.rs`:

```rust
pub struct BuiltinMethod {
    pub name: String,                 // "Min"
    pub params: Vec<ValueType>,       // [Float, Float]
    pub variadic: bool,
    pub returns: ValueType,           // Float
}
pub struct BuiltinObject {
    pub name: String,                 // "Calculate"
    pub methods: Vec<BuiltinMethod>,
}
pub struct Builtins { /* by_name: HashMap<String, BuiltinObject> */ }
impl Builtins {
    pub fn method(&self, object: &str, method: &str) -> Option<&BuiltinMethod>;
    pub fn object(&self, name: &str) -> Option<&BuiltinObject>;
}
```

**b) Source of the data (authoritative).** The definitions are in M1 Build's
firmware packages. Export them from M1 Build (the package/firmware definitions for
the project's firmware build) into a machine-readable file — a `builtins.json` /
`.m1pkg` listing each built-in object, its methods, parameter types and return
types — and parse that into `Builtins`. This keeps the model version-matched and
complete. (Absent an export, the registry can be hand-seeded from M1 docs + the
most-used calls in the corpus, but that's manual and partial.)

Load it next to the project: `Project::load()` should look for the firmware
definitions (e.g. a sibling `builtins.json`, or one keyed by `VersionBuild`) and
build a `Builtins` alongside the `SymbolTable`.

**c) Wire into `resolve()`.** In `src/resolve.rs`, before returning `Opaque` for a
project-unrooted path, split it into `object.method` and consult `Builtins`:

```
resolve("Calculate.Min", scope):
  not a local, not a project symbol ->
    if let Some(m) = builtins.method("Calculate", "Min") {
        return Resolution::Builtin(m)   // new variant carrying the signature
    }
    Opaque
```

Add a `Resolution::Builtin(&BuiltinMethod)` (or reuse `Symbol` with
`SymbolKind::Method` + a synthetic signature). The typer then knows the return
type; a new rule can check argument arity/types.

**d) It surfaces everywhere for free.** Hover, completion, go-to-definition,
signature-help and diagnostics in m1-lsp all go through `resolve()`/`Scope`, so
once `Builtins` is wired in, every editor gets built-in object understanding with
no extra feature code (just the re-vendor + the new `Resolution` arm).

### Effort checklist for the built-in library

1. Obtain/define `builtins.json` from M1 Build (firmware export).
2. `src/symbols/builtins.rs` — types + JSON loader.
3. `Project::load()` — load the firmware definitions.
4. `src/resolve.rs` — consult `Builtins`, add `Resolution::Builtin`.
5. `src/typer.rs` — return type from the builtin method; optional arity/type rule.
6. Re-vendor into m1-lsp; add hover/completion/signature-help handling.
