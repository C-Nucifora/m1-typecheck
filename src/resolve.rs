//! Name resolution: locals, absolute, group-relative, opaque-root passthrough.
use crate::project::Project;
use crate::types::ValueType;
use std::collections::HashMap;

pub struct Scope<'p> {
    pub locals: HashMap<String, ValueType>,
    pub group: Option<String>,
    pub project: Option<&'p Project>,
}

#[derive(Debug)]
pub enum Resolution<'p> {
    Local(ValueType),
    Symbol(&'p crate::symbols::Symbol),
    /// A built-in library object, e.g. `Calculate` (carries its name for
    /// hover/completion of its methods).
    BuiltinObject(&'static str),
    /// A built-in library function / object-method call, e.g.
    /// `CanComms.GetUnsignedInteger` — its overload set (1+ signatures).
    BuiltinFn(Vec<&'static crate::intrinsics::Overload>),
    Opaque,     // resolves to a built-in/unknown root; type Unknown, never flagged
    Unresolved, // project-rooted path with no matching symbol
}

fn root_segment(path: &str) -> &str {
    match path.find('.') {
        Some(i) => &path[..i],
        None => path,
    }
}

/// Rewrite a `Parent[.Parent…].rest` reference, made from inside `group`, into
/// the absolute candidate path it denotes. Each leading `Parent` walks one level
/// up the group tree. Returns `None` if it is not a `Parent` reference or walks
/// above the root.
fn parent_target(path: &str, group: &str) -> Option<String> {
    let mut rest = path;
    let mut levels = 0usize;
    loop {
        if rest == "Parent" {
            levels += 1;
            rest = "";
            break;
        }
        match rest.strip_prefix("Parent.") {
            Some(r) => {
                rest = r;
                levels += 1;
            }
            None => break,
        }
    }
    if levels == 0 {
        return None;
    }
    let mut base = group.to_string();
    for _ in 0..levels {
        let i = base.rfind('.')?;
        base.truncate(i);
    }
    Some(if rest.is_empty() {
        base
    } else {
        format!("{base}.{rest}")
    })
}

pub fn resolve<'p>(path: &str, scope: &Scope<'p>) -> Resolution<'p> {
    // 1. Local (single segment).
    if !path.contains('.')
        && let Some(&t) = scope.locals.get(path)
    {
        return Resolution::Local(t);
    }

    // 1b. Special reference keywords (manual "Keywords"). `In`/`Out` are the
    //     function input-argument / return-value objects — function-local and
    //     never in the project table, so always opaque (never flagged). `Parent`
    //     is a relative reference: walk the enclosing group up one level per
    //     `Parent`, then resolve the remainder. A `Parent` that resolves is a real
    //     symbol; one that doesn't stays opaque (conservative — never a miss).
    match root_segment(path) {
        "In" | "Out" => return Resolution::Opaque,
        "Parent" => {
            if let (Some(project), Some(group)) = (scope.project, scope.group.as_ref())
                && let Some(target) = parent_target(path, group)
                && let Some(sym) = project.symbols().get(&target)
            {
                return Resolution::Symbol(sym);
            }
            return Resolution::Opaque;
        }
        _ => {}
    }

    // 2. Built-in library intrinsics (firmware objects: Calculate, CanComms, …).
    //    Project-independent and resolved before project symbols, per the M1
    //    scope order (local -> library -> project). `Object` -> the object;
    //    `Object.Method` -> its overload set. A project object that shadows a
    //    library name is reached via the `This.`/`Library.` anchors.
    let intr = crate::intrinsics::get();
    let lib_root = root_segment(path);
    if let Some(obj_name) = intr.library_object_name(lib_root) {
        if path == lib_root {
            return Resolution::BuiltinObject(obj_name);
        }
        let method = &path[lib_root.len() + 1..];
        let overloads = intr.library_overloads(lib_root, method);
        if !overloads.is_empty() {
            return Resolution::BuiltinFn(overloads);
        }
        // Known library object, unknown member: still a built-in root, not a miss.
        return Resolution::Opaque;
    }

    let Some(project) = scope.project else {
        // project-less mode: locals only, everything else opaque
        return Resolution::Opaque;
    };
    let table = project.symbols();

    // 2. Absolute (as-is, then Root.-prefixed).
    if let Some(sym) = table.get(path) {
        return Resolution::Symbol(sym);
    }
    let rooted = format!("Root.{path}");
    if let Some(sym) = table.get(&rooted) {
        return Resolution::Symbol(sym);
    }

    // 3. Group-relative: walk the enclosing group up to Root.
    if let Some(group) = &scope.group {
        let mut prefix = Some(group.clone());
        while let Some(g) = prefix {
            let candidate = format!("{g}.{path}");
            if let Some(sym) = table.get(&candidate) {
                return Resolution::Symbol(sym);
            }
            prefix = g.rfind('.').map(|i| g[..i].to_string());
        }
    }

    // 4. Accessor/method on a resolved symbol: if any *proper prefix* of the
    //    path resolves to a symbol, the trailing segments are a built-in
    //    accessor/method (e.g. `Channel.AsInteger`, `Param.Set`) — opaque, not a
    //    miss. This is the key guard against flagging accessor calls on existing
    //    channels/parameters (the symbol table stores stored symbols only, not
    //    the built-in operations they expose).
    if prefix_resolves(path, scope, table) {
        return Resolution::Opaque;
    }

    // 5. Opaque vs unresolved.
    //
    // Bare anchor keywords (`This`/`Library`/`Root`) and boolean literals are not
    // project references — they have no symbol-table entry but must never be
    // flagged as a miss.
    if !path.contains('.') && matches!(path, "This" | "Library" | "Root" | "True" | "False") {
        return Resolution::Opaque;
    }
    // A dotted path is flagged only when its root is a known PROJECT group (so a
    // typo under a firmware/opaque root stays silent). A *bare single-segment*
    // name, however, reached here only after failing local, library, absolute,
    // and group-relative lookup — when we actually had a group to resolve
    // against (`scope.group`), that is a genuine miss (a typo or deleted
    // channel), previously lost as Opaque (#109).
    let root = root_segment(path);
    let root_is_project_group = table
        .get(&format!("Root.{root}"))
        .map(|s| matches!(s.kind, crate::symbols::SymbolKind::Group))
        .unwrap_or(false)
        || root == "Root";
    // A bare enumerator (an enum member referenced without its type prefix, as
    // in `when…is (Idle)` or assigning an enum channel `= Off`) is resolved by
    // context, not a miss.
    let is_bare_enumerator = !path.contains('.') && !table.enums_with_member(path).is_empty();
    let bare_single_segment_miss =
        !path.contains('.') && scope.group.is_some() && !is_bare_enumerator;
    if root_is_project_group || bare_single_segment_miss {
        Resolution::Unresolved
    } else {
        Resolution::Opaque
    }
}

/// True if some proper prefix of `path` (dropping one or more trailing segments)
/// resolves to a symbol via absolute, `Root.`-prefixed, or group-relative lookup.
fn prefix_resolves(path: &str, scope: &Scope, table: &crate::symbols::SymbolTable) -> bool {
    let mut current = path;
    // Strip trailing segments one at a time, testing each shorter prefix. `tail`
    // is the single segment immediately following the prefix under test — it is
    // the accessor/member being applied to that prefix.
    while let Some(i) = current.rfind('.') {
        let tail = &current[i + 1..];
        current = &current[..i];
        if symbol_exists(current, tail, scope, table) {
            return true;
        }
    }
    false
}

/// The accessor names valid on a value-compound group: the auto-created scalar
/// `Value` child plus every modelled object method (`AsInteger`, `Set`, `Lookup`,
/// the CAN/timer accessors, …). Sourced from the intrinsics so new methods are
/// covered automatically.
fn is_known_accessor(seg: &str) -> bool {
    seg == "Value"
        || crate::intrinsics::get()
            .object_methods
            .iter()
            .any(|m| m.name == seg)
}

/// Does `path` resolve to a symbol that exposes `tail` as an accessor/member?
/// Any non-group symbol (channel/parameter/constant/…) exposes the built-in
/// operations, so any `tail` is opaque. A value-bearing compound group (one with
/// a `.Value` child — the M1 enum/channel-compound idiom) exposes only the known
/// accessor set, so a *typo'd* trailing segment (`Mode.Valuee`) is a genuine miss
/// rather than an opaque accessor (#108). Pure namespace groups expose nothing.
fn symbol_exists(
    path: &str,
    tail: &str,
    scope: &Scope,
    table: &crate::symbols::SymbolTable,
) -> bool {
    let is_accessor_base = |full: &str, s: &crate::symbols::Symbol| {
        if !matches!(s.kind, crate::symbols::SymbolKind::Group) {
            true
        } else {
            // A value compound exposes the known accessor set; it may also be
            // addressed by an enumerator of its enum where the channel name
            // collides with the enum type name (`Control.Drive State.<Member>`),
            // so an enum member is a valid trailing segment too.
            table.has_child(full, "Value")
                && (is_known_accessor(tail) || !table.enums_with_member(tail).is_empty())
        }
    };
    for full in [path.to_string(), format!("Root.{path}")] {
        if table
            .get(&full)
            .map(|s| is_accessor_base(&full, s))
            .unwrap_or(false)
        {
            return true;
        }
    }
    if let Some(group) = &scope.group {
        let mut prefix = Some(group.clone());
        while let Some(g) = prefix {
            let full = format!("{g}.{path}");
            if table
                .get(&full)
                .map(|s| is_accessor_base(&full, s))
                .unwrap_or(false)
            {
                return true;
            }
            prefix = g.rfind('.').map(|i| g[..i].to_string());
        }
    }
    false
}

#[cfg(test)]
mod parent_tests {
    use super::parent_target;

    #[test]
    fn one_parent_resolves_against_the_enclosing_group_tree() {
        // A function in `Root.Inputs.Calculations` referencing `Parent.Result A`
        // means the parent group `Root.Inputs`'s `Result A` (manual example).
        assert_eq!(
            parent_target("Parent.Result A", "Root.Inputs.Calculations").as_deref(),
            Some("Root.Inputs.Result A")
        );
    }

    #[test]
    fn chained_parents_walk_up_multiple_levels() {
        assert_eq!(
            parent_target("Parent.Parent.Value", "Root.Inputs.Calculations").as_deref(),
            Some("Root.Value")
        );
    }

    #[test]
    fn parent_alone_denotes_the_parent_group() {
        assert_eq!(
            parent_target("Parent", "Root.Inputs.Calculations").as_deref(),
            Some("Root.Inputs")
        );
    }

    #[test]
    fn walking_above_root_is_none() {
        assert_eq!(parent_target("Parent.Parent.X", "Root.A"), None);
    }

    #[test]
    fn a_non_parent_path_is_none() {
        assert_eq!(parent_target("Result A", "Root.Inputs"), None);
    }
}

#[cfg(test)]
mod intrinsics_tests {
    use super::*;
    use std::collections::HashMap;

    fn scope() -> Scope<'static> {
        Scope {
            locals: HashMap::new(),
            group: None,
            project: None,
        }
    }

    #[test]
    fn resolves_library_object_and_method() {
        // Project-less scope -> a library object/method still resolves via 4b.
        match resolve("Calculate", &scope()) {
            Resolution::BuiltinObject(n) => assert_eq!(n, "Calculate"),
            other => panic!("expected BuiltinObject, got {other:?}"),
        }
        match resolve("CanComms.GetUnsignedInteger", &scope()) {
            Resolution::BuiltinFn(ov) => {
                assert!(!ov.is_empty());
                assert_eq!(ov[0].name, "GetUnsignedInteger");
            }
            other => panic!("expected BuiltinFn, got {other:?}"),
        }
    }

    #[test]
    fn non_library_root_stays_opaque() {
        assert!(matches!(
            resolve("Engine.Speed", &scope()),
            Resolution::Opaque
        ));
    }
}
