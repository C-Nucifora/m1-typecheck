//! T107 dbc-init-missing — a whole-project rule mirroring M1 Build **Error
//! 1375** ("DBC Init function is not called").
//!
//! For every DBC object registered in the project (`BuiltIn.CAN.DBC` — an
//! object at path `DBC.<Name>`), if any script uses one of its generated CAN
//! accessors (`DBC.<Name>.<signal>`, `DBC.<Name>.Transmit(…)`, …) but no script
//! calls its `DBC.<Name>.Init(<bus>)`, emit an error. `uses(X) ∧ ¬calls(X.Init)`.
//!
//! This is a whole-project property — the Init legitimately lives in a
//! different script from the use (the real corpora keep every `DBC.*.Init` call
//! in one `CAN Init` script) — so it belongs in the cross-script layer, not
//! per-file analysis.
//!
//! Detection is textual against the set of registered DBC object paths: a use
//! is any call callee or member reference whose path starts with `<object>.`
//! (so it catches deep accessors like
//! `DBC.<Name>.<Message>.<Signal>.GetUnsignedInteger()`, exactly as the real
//! corpus writes them), and an Init is a call whose callee is precisely
//! `<object>.Init`. The trailing `.` in the prefix keeps `DBC.Dash` from
//! matching `DBC.DashSwitches`.
//!
//! A DBC is registered **twice** in a loaded project — once from the `.m1prj`
//! at `DBC.<Name>` and once bare at `<Name>` — and a script may reference it
//! either way (Rx signals as `DBC.<Name>.…`, a Tx object as `<Name>.…`), while
//! the Init is called on just one spelling. So init and use are tracked by the
//! DBC's **leaf name** (the shared last path segment), unifying both
//! registrations; otherwise the `DBC.<Name>.Init` call would not count as
//! initialising the bare `<Name>` registration and every Tx DBC would
//! false-positive.

use crate::diagnostics::{TypeCode, TypeDiagnostic, make_project_for};
use crate::parsed::ParsedScript;
use crate::project::Project;
use crate::symbols::Symbol;
use crate::typer::path_text;
use m1_core::{Kind, Node, Severity};
use std::collections::{BTreeMap, HashSet};

/// A registered DBC object. The classname is the reliable discriminator: a DBC
/// declared in the `.m1prj` resolves to `SymbolKind::Other`, while the `.m1dbc`
/// augment uses `SymbolKind::Object` — so we key on the classname, not the kind.
/// (The DBC *group* is `BuiltIn.CAN.DBCRoot`, which this deliberately excludes.)
fn is_dbc(s: &Symbol) -> bool {
    s.classname.as_deref() == Some("BuiltIn.CAN.DBC")
}

/// The leaf (last `.`-segment) of a DBC object path — `DBC.BMU` and bare `BMU`
/// share the leaf `BMU`, which is how the two registrations are unified.
fn leaf(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or(path)
}

/// Attribute a reference `text` (from a call callee or member access) to a
/// registered DBC by its **leaf name**. Returns `(leaf, is_init_call)`.
fn attribute<'a>(text: &str, is_call: bool, dbc_paths: &'a [String]) -> Option<(&'a str, bool)> {
    for obj in dbc_paths {
        // An Init is exactly `<object>.Init` as a call — matched before the
        // generic prefix so it is not also counted as a use.
        if is_call
            && text.len() == obj.len() + 5
            && text.starts_with(obj)
            && text.ends_with(".Init")
        {
            return Some((leaf(obj), true));
        }
        // Any other reference under `<object>.` is a use of a generated accessor.
        if text.len() > obj.len() && text.as_bytes()[obj.len()] == b'.' && text.starts_with(obj) {
            return Some((leaf(obj), false));
        }
    }
    None
}

fn walk(
    n: Node,
    dbc_paths: &[String],
    script: &str,
    inited: &mut HashSet<String>,
    used: &mut BTreeMap<String, String>,
) {
    // A call: `<DBC>.Init(…)` initialises; any other call under `<DBC>.`
    // (`.Transmit(…)`, `.Switches.Receive()`, …) uses it.
    if n.kind() == Kind::CallExpression
        && let Some(callee) = n
            .named_children()
            .into_iter()
            .find(|c| matches!(c.kind(), Kind::Identifier | Kind::MemberExpression))
    {
        if let Some((leaf, is_init)) = attribute(&path_text(callee), true, dbc_paths) {
            if is_init {
                inited.insert(leaf.to_string());
            } else {
                used.entry(leaf.to_string())
                    .or_insert_with(|| script.to_string());
            }
        }
        // A generated accessor can also appear in the arguments; walk them (the
        // callee is fully handled above, so it is deliberately not descended).
        if let Some(args) = n
            .children()
            .into_iter()
            .find(|c| c.kind() == Kind::ArgumentList)
        {
            walk(args, dbc_paths, script, inited, used);
        }
        return;
    }
    // A bare member access `DBC.<Name>.<signal>` reads a generated accessor.
    if n.kind() == Kind::MemberExpression {
        if let Some((leaf, _is_init)) = attribute(&path_text(n), false, dbc_paths) {
            used.entry(leaf.to_string())
                .or_insert_with(|| script.to_string());
        }
        // A member chain resolves as a whole; do not descend into it.
        return;
    }
    for c in n.children() {
        walk(c, dbc_paths, script, inited, used);
    }
}

/// Run the DBC-Init audit over the whole project. Returns one T107 error per
/// registered DBC that is used by some script but initialised by none.
pub fn check(project: &Project, scripts: &[ParsedScript]) -> Vec<TypeDiagnostic> {
    let dbc_paths: Vec<String> = project
        .symbols()
        .iter()
        .filter(|s| is_dbc(s))
        .map(|s| s.path.clone())
        .collect();
    if dbc_paths.is_empty() {
        return Vec::new();
    }
    let mut inited: HashSet<String> = HashSet::new();
    // Keyed by DBC leaf name; BTreeMap → deterministic (sorted) diagnostic order.
    let mut used: BTreeMap<String, String> = BTreeMap::new();

    for s in scripts {
        // Skip unparseable scripts — their reference shapes are unreliable.
        if !s.cst.syntax_diagnostics().is_empty() {
            continue;
        }
        walk(s.cst.root(), &dbc_paths, &s.name, &mut inited, &mut used);
    }

    used.iter()
        .filter(|(leaf, _)| !inited.contains(*leaf))
        .map(|(leaf, script)| {
            make_project_for(
                TypeCode::T107,
                Severity::Error,
                format!(
                    "DBC `{leaf}` is used (e.g. in `{script}`) but its Init function is never \
                     called by any script — call `{leaf}`'s Init function (`DBC.{leaf}.Init(<bus>)`) \
                     in an initialisation script (M1 Build Error 1375)"
                ),
                leaf.as_str(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.FuncUser" Filename="CAN.Init.m1scr" Name="Root.CAN.Init"/>
  <Component Classname="BuiltIn.FuncUser" Filename="CAN.Rx.m1scr" Name="Root.CAN.Rx"/>
  <Component Classname="BuiltIn.CAN.DBCRoot" Name="DBC"/>
  <Component Classname="BuiltIn.CAN.DBC" Name="DBC.Inited"/>
  <Component Classname="BuiltIn.CAN.DBC" Name="DBC.Missing"/>
</Project>"#;

    fn run(srcs: &[(&str, &str)]) -> Vec<String> {
        let project = Project::from_xml(XML).unwrap();
        let scripts = crate::parsed::parse_all(
            &srcs
                .iter()
                .map(|(n, s)| (n.to_string(), s.to_string()))
                .collect::<Vec<_>>(),
        );
        check(&project, &scripts)
            .into_iter()
            .map(|d| d.inner.message)
            .collect()
    }

    #[test]
    fn used_without_init_is_flagged() {
        // `DBC.Missing` is read but never initialised → T107; `DBC.Inited` is
        // both initialised and used → clean.
        let msgs = run(&[
            ("CAN.Init.m1scr", "DBC.Inited.Init(Active Bus);\n"),
            (
                "CAN.Rx.m1scr",
                "Speed = DBC.Missing.Wheel Speed;\nTorque = DBC.Inited.Torque;\n",
            ),
        ]);
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("`Missing`"), "{}", msgs[0]);
        assert!(msgs[0].contains("CAN.Rx.m1scr"), "{}", msgs[0]);
        assert!(msgs[0].contains("1375"), "{}", msgs[0]);
    }

    #[test]
    fn init_call_alone_is_not_a_use() {
        // Initialised but otherwise never used → no diagnostic (not an error).
        let msgs = run(&[("CAN.Init.m1scr", "DBC.Inited.Init(Active Bus);\n")]);
        assert!(msgs.is_empty(), "{msgs:?}");
    }

    #[test]
    fn init_in_a_different_script_counts() {
        // The Init lives in a separate script from the use — still initialised.
        let msgs = run(&[
            ("CAN.Init.m1scr", "DBC.Inited.Init(Active Bus);\n"),
            ("CAN.Rx.m1scr", "X = DBC.Inited.Torque;\n"),
        ]);
        assert!(msgs.is_empty(), "{msgs:?}");
    }

    #[test]
    fn transmit_call_is_a_use() {
        // A non-Init method call (`.Transmit`) counts as use.
        let msgs = run(&[("CAN.Rx.m1scr", "DBC.Missing.Transmit(Active Bus);\n")]);
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("`Missing`"), "{}", msgs[0]);
    }

    #[test]
    fn dual_registration_is_unified_by_leaf() {
        // A loaded project registers a DBC twice — `DBC.BMU` (from the .m1prj)
        // and bare `BMU`. A Tx script uses the bare spelling while the Init is
        // called on the prefixed spelling; keyed by leaf, that is initialised,
        // not a false positive.
        let project = Project::from_xml(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.CAN.DBCRoot" Name="DBC"/>
  <Component Classname="BuiltIn.CAN.DBC" Name="DBC.BMU"/>
  <Component Classname="BuiltIn.CAN.DBC" Name="BMU"/>
</Project>"#,
        )
        .unwrap();
        let scripts = crate::parsed::parse_all(&[
            (
                "CAN.Init.m1scr".into(),
                "DBC.BMU.Init(Active Bus);\n".into(),
            ),
            (
                "CAN.Tx.m1scr".into(),
                "BMU.Ignition.Transmit(Active Bus);\n".into(),
            ),
        ]);
        assert!(check(&project, &scripts).is_empty());
    }

    #[test]
    fn no_dbc_objects_is_noop() {
        let project = Project::from_xml(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Project><Component Classname="BuiltIn.GroupCompound" Name="Root"/></Project>"#,
        )
        .unwrap();
        assert!(check(&project, &[]).is_empty());
    }
}
