//! T105 local-use-before-definition (manual p.34).
//!
//! The manual's *local/static local* section: "Inside the function, a local
//! variable can only be used after it has been defined." M1 Build enforces this
//! ordering — a read of a local that textually precedes its `local x = …;`
//! declaration is a build error.
//!
//! The base resolver registers every local up-front in a flat, position-less map
//! ([`crate::rules::collect_locals`]), so a forward reference still resolves as
//! [`Resolution::Local`] and the per-node rules never see the ordering problem.
//! This separate positional pass closes that gap: it records each local's
//! declaration byte-offset, then flags any single-segment identifier that
//! resolves to a `Local` and starts *before* the earliest declaration of that
//! name.
//!
//! Whole-tree pass (like `t091_local_case` / `flow.rs`): it needs the full set of
//! declarations before it can judge any reference. Scoped strictly to names that
//! resolve to [`Resolution::Local`] — Unknown/Opaque/project symbols stay silent,
//! matching the resolver's conservative policy. A reference nested inside the
//! declaration's own initializer (`local x = x;`) is exempt: that is the
//! manual's own self-reference shape and types as Unknown, not an ordering error.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use m1_core::{Field, Kind, Node, Severity};
use std::collections::HashMap;

pub fn check(root: Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    // 1. Earliest declaration offset per local name, and the initializer byte
    //    range of each so that `local x = x;` self-references are exempt.
    let mut first_decl: HashMap<String, usize> = HashMap::new();
    let mut init_ranges: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    collect_decls(root, &mut first_decl, &mut init_ranges);
    if first_decl.is_empty() {
        return;
    }

    // 2. Every single-segment identifier reference that resolves to a Local and
    //    starts before that local's earliest declaration is a use-before-def.
    walk_refs(root, scope, &first_decl, &init_ranges, out);
}

/// Record, per local name, the byte-offset of its earliest declaration, plus the
/// byte range of every declaration's initializer (for the self-reference exempt).
fn collect_decls(
    node: Node,
    first_decl: &mut HashMap<String, usize>,
    init_ranges: &mut Vec<(String, std::ops::Range<usize>)>,
) {
    if node.kind() == Kind::LocalDeclaration
        && let Some(name_node) = node.child_by_field(Field::Name)
    {
        let name = name_node.text().to_string();
        let start = node.byte_range().start;
        first_decl
            .entry(name.clone())
            .and_modify(|s| *s = (*s).min(start))
            .or_insert(start);
        if let Some(init) = node.child_by_field(Field::Value) {
            init_ranges.push((name, init.byte_range()));
        }
    }
    for c in node.children() {
        collect_decls(c, first_decl, init_ranges);
    }
}

/// Flag single-segment identifier references that precede their declaration.
fn walk_refs(
    node: Node,
    scope: &Scope,
    first_decl: &HashMap<String, usize>,
    init_ranges: &[(String, std::ops::Range<usize>)],
    out: &mut Vec<TypeDiagnostic>,
) {
    if node.kind() == Kind::Identifier {
        let name = node.text();
        // Only a *reference* identifier matters: the declaration's own `name`
        // node, a `<Type>` annotation name, and a member-path segment are not
        // value reads of a local.
        let is_reference = node.parent().is_none_or(|p| {
            !matches!(
                p.kind(),
                Kind::LocalDeclaration | Kind::TypeAnnotation | Kind::MemberExpression
            )
        });
        if is_reference && let Some(&decl_start) = first_decl.get(name) {
            let start = node.byte_range().start;
            // A reference nested inside *some* declaration's initializer of the
            // same name is the `local x = x;` self-reference shape — exempt.
            let in_own_init = init_ranges
                .iter()
                .any(|(n, r)| n == name && r.start <= start && start < r.end);
            if start < decl_start
                && !in_own_init
                && matches!(resolve(name, scope), Resolution::Local(_))
            {
                out.push(make(
                    TypeCode::T105,
                    &node,
                    Severity::Error,
                    format!(
                        "local `{name}` is used before it is defined; a local variable \
                         can only be used after its `local` declaration (manual p.34)"
                    ),
                ));
            }
        }
    }
    for c in node.children() {
        walk_refs(c, scope, first_decl, init_ranges, out);
    }
}

#[cfg(test)]
mod tests {
    use crate::diagnostics::CheckResult;
    use crate::rules::check_script_no_project;

    fn has(r: &CheckResult, code: &str) -> bool {
        r.diagnostics.iter().any(|d| d.code.as_str() == code)
    }

    #[test]
    fn use_before_definition_is_flagged() {
        // `b` is read before its own `local b = …;` declaration line.
        let src = "local a = b + 1;\nlocal b = 2;\n";
        let r = check_script_no_project(src);
        assert!(
            has(&r, "T105"),
            "expected T105 use-before-def, got {:?}",
            r.diagnostics
        );
    }

    #[test]
    fn declaration_then_use_is_clean() {
        // Backward reference (the legal direction) must not fire.
        let src = "local a = 1;\nlocal b = a + 1;\n";
        let r = check_script_no_project(src);
        assert!(!has(&r, "T105"), "unexpected T105: {:?}", r.diagnostics);
    }

    #[test]
    fn self_reference_initializer_is_exempt() {
        // `local x = x;` reads `x` inside its own initializer — the manual's
        // self-reference shape, not an ordering error.
        let src = "local x = x;\n";
        let r = check_script_no_project(src);
        assert!(!has(&r, "T105"), "unexpected T105: {:?}", r.diagnostics);
    }

    #[test]
    fn unknown_name_is_silent() {
        // A bare name that does not resolve to a local (no such declaration) is
        // not a use-before-def — the resolver leaves it Opaque/Unresolved.
        let src = "local a = SomethingElse + 1;\n";
        let r = check_script_no_project(src);
        assert!(!has(&r, "T105"), "unexpected T105: {:?}", r.diagnostics);
    }
}
