//! M1 Build Error 1339 — ambiguous reference (T103, #234).
//!
//! M1 Build rejects a **bare name** that resolves to more than one symbol *kind*
//! in the same scope. The shape the issue targets: a group has a child channel
//! `State` whose declared type is an enum **also** named `State`; a script scoped
//! to that group writes the bare member access `State.Member`. The firmware
//! compiler cannot tell whether `State` is the sibling channel (member access on
//! its value) or the enum *type* (an enumerator), so it emits:
//!
//! > `1339` : Ambiguous reference 'State'. Multiple matches found.
//!
//! The idiomatic fix is to qualify the channel as `This.State.Member`.
//!
//! ## Why this is corpus-safe (precision)
//!
//! The real corpora reference an enum-named-after-something *unambiguously*:
//! `Precharge State.Run` and `Timeout.EPOS` are bare enumerators whose
//! same-named channel/parameter (`…Accumulator.Precharge State`,
//! `…CAN.<bus>.Timeout`) is **not** reachable by the *bare* name from the using
//! script's scope (it lives under a deeper group, only ever accessed qualified).
//! So the check fires only when a bare name simultaneously resolves, *in the
//! current script's scope*, to:
//!
//! 1. a plain **Channel/Parameter** reachable by that bare name (group-relative,
//!    mirroring [`crate::resolve`]'s walk — *not* a value-compound `Group`, which
//!    is the accepted `Drive State.Member` enum-compound idiom), **and**
//! 2. an enum **type** of the same name ([`SymbolTable::enum_by_name`]).
//!
//! Both conditions hold only for a genuinely ambiguous bare reference, never for
//! the qualified (`This.`/`Parent.`/path-prefixed) accesses the corpora use.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::symbols::{SymbolKind, SymbolTable};
use crate::typer::path_text;
use m1_core::{Kind, Node, Severity};
use std::collections::HashSet;

/// The leading segment of a (possibly dotted) reference path. `Precharge
/// State.Run` → `Precharge State` (M1 names embed spaces; only `.` separates
/// member segments).
fn head_segment(path: &str) -> &str {
    match path.find('.') {
        Some(i) => &path[..i],
        None => path,
    }
}

/// Append T103 diagnostics for ambiguous bare references in this script.
pub fn check(root: Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    let Some(project) = scope.project else {
        return;
    };
    let Some(group) = scope.group.as_deref() else {
        return;
    };
    let table = project.symbols();
    // One diagnostic per ambiguous name per script (a name used many times is one
    // finding, not a swarm).
    let mut reported: HashSet<String> = HashSet::new();

    for n in root.descendants() {
        if !matches!(n.kind(), Kind::Identifier | Kind::MemberExpression) {
            continue;
        }
        // Only the head of a reference chain — an inner member segment is handled
        // at its chain root.
        if n.parent().map(|p| p.kind()) == Some(Kind::MemberExpression) {
            continue;
        }
        let path = path_text(n);
        let name = head_segment(&path);
        // Anchor keywords are explicit qualifiers, never bare names (`This.State`
        // is the *fix*, not a violation).
        if matches!(name, "This" | "Parent" | "Root" | "Library" | "In" | "Out") {
            continue;
        }
        // A local shadows project/enum names — not a project-symbol ambiguity.
        if scope.locals.contains_key(name) {
            continue;
        }
        if reported.contains(name) {
            continue;
        }
        // Both kinds must be in scope for the name to be ambiguous.
        if table.enum_by_name(name).is_none() {
            continue;
        }
        if !channel_reachable_bare(name, group, table) {
            continue;
        }
        reported.insert(name.to_string());
        out.push(make(
            TypeCode::T103,
            &n,
            Severity::Error,
            format!(
                "ambiguous reference `{name}`: the bare name matches both a sibling channel and \
                 an enum type named `{name}` — qualify the channel as `This.{name}` (M1 Build \
                 Error 1339: \"Multiple matches found\")"
            ),
        ));
    }
}

/// True when the bare `name` resolves, in `group`'s scope, to a plain
/// Channel/Parameter symbol — the same lookup order [`crate::resolve`] uses for a
/// bare reference (absolute, `Root.`-prefixed, then the group tree up to Root),
/// restricted to value symbols. A value-compound `Group` is deliberately excluded
/// (its `Group.Member` enum access is unambiguous and corpus-legitimate).
fn channel_reachable_bare(name: &str, group: &str, table: &SymbolTable) -> bool {
    let is_value =
        |s: &crate::symbols::Symbol| matches!(s.kind, SymbolKind::Channel | SymbolKind::Parameter);
    for cand in [name.to_string(), format!("Root.{name}")] {
        if table.get(&cand).is_some_and(is_value) {
            return true;
        }
    }
    let mut prefix = Some(group.to_string());
    while let Some(g) = prefix {
        if table.get(&format!("{g}.{name}")).is_some_and(is_value) {
            return true;
        }
        prefix = g.rfind('.').map(|i| g[..i].to_string());
    }
    false
}

#[cfg(test)]
mod tests {
    use crate::project::Project;
    use crate::rules::check_script;
    use std::path::Path;

    /// A group `Root.Ctrl` with a child channel `State` typed as the enum also
    /// named `State`, plus a far-away channel `Precharge State` (enum `Precharge
    /// State`) that is *not* a bare sibling of the `Ctrl` script.
    const XML: &str = r#"<?xml version="1.0"?>
<Project>
  <DataTypes>
    <Type Name="State" Storage="enum" Default="Idle">
      <Enum Name="Idle" ContainerOrder="0"/>
      <Enum Name="Run" ContainerOrder="1"/>
    </Type>
    <Type Name="Precharge State" Storage="enum" Default="Off">
      <Enum Name="Off" ContainerOrder="0"/>
      <Enum Name="Run" ContainerOrder="1"/>
    </Type>
  </DataTypes>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Ctrl"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Ctrl.State"><Props Type="::This.State" Security="Tune"/></Component>
  <Component Classname="BuiltIn.FuncUser" Filename="Ctrl.Update.m1scr" Name="Root.Ctrl.Update"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Accumulator"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Accumulator.Precharge State"><Props Type="::This.Precharge State" Security="Tune"/></Component>
  <Component Classname="BuiltIn.FuncUser" Filename="Other.Update.m1scr" Name="Root.Other.Update"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Other"/>
</Project>"#;

    fn codes(file: &str, src: &str) -> Vec<String> {
        let project = Project::from_xml(XML).unwrap();
        check_script(&project, Path::new(file), src)
            .diagnostics
            .into_iter()
            .filter(|d| d.code == crate::diagnostics::TypeCode::T103)
            .map(|d| d.inner.message)
            .collect()
    }

    #[test]
    fn bare_name_matching_sibling_channel_and_enum_is_flagged() {
        // `State.Run` from the `Ctrl` script: `State` is both the sibling channel
        // and an enum type → 1339.
        let diags = codes("Ctrl.Update.m1scr", "local x = State.Run;\n");
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].contains("`State`") && diags[0].contains("This.State"));
    }

    #[test]
    fn flagged_at_error_severity() {
        let project = Project::from_xml(XML).unwrap();
        let d = check_script(
            &project,
            Path::new("Ctrl.Update.m1scr"),
            "local x = State.Run;\n",
        )
        .diagnostics
        .into_iter()
        .find(|d| d.code == crate::diagnostics::TypeCode::T103)
        .expect("T103");
        assert_eq!(d.inner.severity, m1_core::Severity::Error);
    }

    #[test]
    fn qualified_this_reference_is_not_flagged() {
        // The documented fix: `This.State.Run` is unambiguous.
        assert!(codes("Ctrl.Update.m1scr", "local x = This.State.Run;\n").is_empty());
    }

    #[test]
    fn bare_enumerator_whose_channel_is_not_a_sibling_is_not_flagged() {
        // The corpus shape: `Precharge State.Run` from a script whose scope does
        // NOT reach a bare `Precharge State` channel (it lives under Accumulator).
        assert!(codes("Other.Update.m1scr", "local x = Precharge State.Run;\n").is_empty());
    }

    #[test]
    fn name_matching_only_an_enum_is_not_flagged() {
        // `State` enum exists but no bare-reachable `State` channel from `Other`.
        assert!(codes("Other.Update.m1scr", "local x = State.Run;\n").is_empty());
    }

    #[test]
    fn name_used_once_reports_once() {
        let diags = codes(
            "Ctrl.Update.m1scr",
            "local a = State.Run;\nlocal b = State.Idle;\n",
        );
        assert_eq!(diags.len(), 1, "deduped per name: {diags:?}");
    }
}
