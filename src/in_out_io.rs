//! M1 `In`/`Out` function-I/O model (#233).
//!
//! M1 has **no `return` keyword** (manual → *The M1 Programming Language* →
//! *Keywords*). A user function's arguments live on the `In` object and its
//! return value is set by assigning the `Out` object:
//!
//! ```text
//! local result = In.flagIn;   // read the input argument
//! Out = result;               // set the return value
//! ```
//!
//! M1 Build "Validate Project" rejects the C-style alternatives. This pass
//! mirrors those errors for a `FuncUserParam` / user method backed by the script
//! under check whose `.m1prj` declares a `<Signature>`:
//!
//! - **T100** — a declared `<Param>` referenced by its *bare* name instead of
//!   `In.<name>` (M1 Build `1338` "… does not exist"). Only a name that does not
//!   otherwise resolve is flagged, so a same-named channel is never a false
//!   positive; T100 supersedes the generic [`crate::rules::t001_unresolved`]
//!   (T001) at that location.
//! - **T101** — a C-style `return <expr>` statement (M1 Build `1338` on the
//!   `return …` token; M1 has no `return` keyword). Also supersedes T001.
//! - **T098** — a declared `<Param>` never read via `In.<name>` (M1 Build `1355`
//!   "Unused argument").
//! - **T099** — a non-empty `<Signature ReturnType="…">` with no `Out = …`
//!   assignment in the body (M1 Build `1353` "Return value … is not assigned").
//!
//! Every diagnostic is an **Error** (M1 Build rejects the build). The whole pass
//! is gated on the script backing a user `Function`/`Method` that declares a
//! `<Signature>`; a function with no signature (a plain `FuncUser`) is left
//! entirely alone, keeping the real corpora — which pass M1 Build — clean.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make, related_to_def};
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use m1_core::{Field, Kind, Node, Severity};
use std::collections::HashSet;

/// Entry point: append T098/T099/T100/T101 diagnostics, and drop any T001 that a
/// T100/T101 finding supersedes at the same location.
pub fn check(root: Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    let Some(project) = scope.project else {
        return;
    };
    let Some(fn_path) = scope.fn_symbol.as_deref() else {
        return;
    };
    let Some(sym) = project.symbols().get(fn_path) else {
        return;
    };
    if !matches!(sym.kind, SymbolKind::Function | SymbolKind::Method) {
        return;
    }
    // No `<Signature>` element → the function has no declared I/O contract, so
    // none of these checks apply (this is the corpus-safety gate).
    let Some(params) = sym.in_params.as_ref() else {
        return;
    };

    // One iterative walk gathers every fact: which params are read via `In.`,
    // whether the return sink `Out` is assigned, and the bare-ref / return-stmt
    // nodes. `descendants()` is iterative, matching the depth-guard discipline
    // the rest of the crate uses (#94).
    let mut read_params: HashSet<&str> = HashSet::new();
    let mut has_out = false;
    let mut bare_refs: Vec<(Node, String)> = Vec::new();
    let mut returns: Vec<Node> = Vec::new();

    for n in root.descendants() {
        match n.kind() {
            // `In.<param>` (or `In.<param>.<accessor>`) is an argument read.
            Kind::MemberExpression => {
                if n.parent().map(|p| p.kind()) == Some(Kind::MemberExpression) {
                    continue; // inner segment of a longer path; handled at the root
                }
                if let Some(rest) = n.text().strip_prefix("In.") {
                    for (pname, _) in params {
                        if rest == pname || rest.starts_with(&format!("{pname}.")) {
                            read_params.insert(pname.as_str());
                        }
                    }
                }
            }
            // `Out = …` / `Out.<member> = …` is the return-value assignment.
            Kind::AssignmentStatement => {
                if let Some(tgt) = n.child_by_field(Field::Target) {
                    let t = tgt.text();
                    if t == "Out" || t.starts_with("Out.") {
                        has_out = true;
                    }
                }
            }
            Kind::Identifier => {
                let t = n.text();
                let parent_kind = n.parent().map(|p| p.kind());
                if parent_kind == Some(Kind::MemberExpression) {
                    continue; // part of a longer path, not a bare reference
                }
                // A C-style `return <expr>;` parses as one identifier token
                // (`"return result"`) in statement position — M1 has no `return`
                // keyword, so the grammar reads the whole thing as one name.
                if parent_kind == Some(Kind::ExpressionStatement) && t.starts_with("return ") {
                    returns.push(n);
                    continue;
                }
                // A bare reference to a declared parameter name, in read position,
                // that does not otherwise resolve — the C-style argument access
                // the issue targets. The `Unresolved` guard means a same-named
                // channel/local is never a false positive.
                if params.iter().any(|(p, _)| p == t)
                    && !is_plain_assignment_target(&n)
                    && matches!(resolve(t, scope), Resolution::Unresolved)
                {
                    bare_refs.push((n, t.to_string()));
                }
            }
            _ => {}
        }
    }

    // Byte ranges a T100/T101 finding owns, so the generic T001 emitted earlier
    // for the same token is dropped (no double-report).
    let mut superseded: Vec<std::ops::Range<usize>> = Vec::new();

    for (node, pname) in &bare_refs {
        out.push(make(
            TypeCode::T100,
            node,
            Severity::Error,
            format!(
                "parameter `{pname}` is referenced by its bare name; read the argument via \
                 `In.{pname}` (M1 Build Error 1338: \"does not exist\" — M1 has no C-style \
                 argument access)"
            ),
        ));
        superseded.push(node.byte_range());
    }
    for node in &returns {
        out.push(make(
            TypeCode::T101,
            node,
            Severity::Error,
            format!(
                "`{}` is a C-style return statement; M1 has no `return` keyword — set the \
                 return value with `Out = <expr>` instead (M1 Build Error 1338: \
                 \"does not exist\")",
                node.text()
            ),
        ));
        superseded.push(node.byte_range());
    }
    if !superseded.is_empty() {
        out.retain(|d| d.code != TypeCode::T001 || !superseded.contains(&d.inner.byte_range));
    }

    // T098 — a declared argument never read via `In.<name>`.
    for (pname, _) in params {
        if !read_params.contains(pname.as_str()) {
            let mut d = make(
                TypeCode::T098,
                &root,
                Severity::Error,
                format!(
                    "argument `{pname}` of `{fn_path}` is never read; read it via `In.{pname}` \
                     (M1 Build Error 1355: \"Unused argument\")"
                ),
            );
            d.related.extend(related_to_def(
                sym,
                format!("argument `{pname}` declared in the signature of `{fn_path}`"),
            ));
            out.push(d);
        }
    }

    // T099 — a declared, non-empty `ReturnType` with no `Out = …` assignment.
    // `return_type` is `Some` for a declared `<Signature ReturnType>`; an
    // inferred one only exists when an `Out =` is present, so `Some` here with
    // no `Out` in the body can only be a declared return type left unassigned.
    if sym.return_type.is_some() && !has_out {
        let mut d = make(
            TypeCode::T099,
            &root,
            Severity::Error,
            format!(
                "function `{fn_path}` declares a return type but never assigns `Out`; set the \
                 return value with `Out = <expr>` (M1 Build Error 1353: \"Return value … is not \
                 assigned\")"
            ),
        );
        d.related.extend(related_to_def(
            sym,
            format!("return type declared in the signature of `{fn_path}`"),
        ));
        out.push(d);
    }
}

/// True when `node` is the `target` of a plain `=` assignment (a pure write).
/// A write to an unknown target is [`crate::rules::t001_unresolved`]'s T031, not
/// a bare-argument *read*, so it is excluded from T100.
fn is_plain_assignment_target(node: &Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != Kind::AssignmentStatement {
        return false;
    }
    if !parent.children().iter().any(|c| c.kind() == Kind::Assign) {
        return false;
    }
    parent
        .child_by_field(Field::Target)
        .is_some_and(|t| t.byte_range() == node.byte_range())
}
