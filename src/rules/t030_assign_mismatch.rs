use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use crate::typer::{is_expr, path_text, type_of};
use crate::types::ValueType;
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() == Kind::LocalDeclaration {
            check_local_declaration(node, scope, out);
            return;
        }
        if node.kind() != Kind::AssignmentStatement {
            return;
        }
        let is_compound = node
            .child_by_field(Field::Operator)
            .map(|op| {
                matches!(
                    op.kind(),
                    Kind::PlusEq | Kind::MinusEq | Kind::StarEq | Kind::SlashEq
                )
            })
            .unwrap_or(false);
        if is_compound {
            check_compound_assign(node, scope, out);
        } else {
            check_plain_assign(node, scope, out);
        }
    }
}

/// Plain `=` assignment: flag any type-incompatible RHS, including float→integer
/// (per manual p.44, #210) and wrong-enum targets.
fn check_plain_assign(node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    // Only plain `=`.
    if !node.children().iter().any(|c| c.kind() == Kind::Assign) {
        return;
    }
    let kids = node.named_children();
    let (Some(target), Some(value)) = (
        kids.iter().find(|c| is_expr(c.kind())),
        kids.iter().rev().find(|c| is_expr(c.kind())),
    ) else {
        return;
    };
    if std::ptr::eq(target, value) {
        return; // single child; nothing to compare
    }
    // Target may be a Channel/Parameter symbol, or a local — locals have a
    // fully-known declared type (the lattice and `type_of` support them, per
    // T003), so a float→integer write to a local is the same mismatch the
    // manual gives as invalid (`low = high;`, p.44) and must flag too (#210).
    let value_ty = type_of(*value, scope);
    let (target_ty, target_sym): (ValueType, Option<&crate::symbols::Symbol>) =
        match resolve(&path_text(*target), scope) {
            Resolution::Symbol(s)
                if matches!(s.kind, SymbolKind::Channel | SymbolKind::Parameter) =>
            {
                (s.value_type, Some(s))
            }
            Resolution::Local(ty) => (ty, None),
            _ => return,
        };
    if !target_ty.is_known() || !value_ty.is_known() {
        return; // silent under Unknown
    }
    if !compatible(target_ty, value_ty) {
        emit_mismatch(node, scope, target_ty, value_ty, target_sym, out);
    }
}

/// Compound assignment (`+=`, `-=`, `*=`, `/=`): flag type-incompatible RHS,
/// parallel to plain `=`. Defers to T003 for the integral-target + Float-value
/// narrowing case (to avoid double-flagging the same construct).
fn check_compound_assign(node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    let (Some(target), Some(value)) = (
        node.child_by_field(Field::Target),
        node.child_by_field(Field::Value),
    ) else {
        return;
    };
    let value_ty = type_of(value, scope);
    let (target_ty, target_sym): (ValueType, Option<&crate::symbols::Symbol>) =
        match resolve(&path_text(target), scope) {
            Resolution::Symbol(s)
                if matches!(s.kind, SymbolKind::Channel | SymbolKind::Parameter) =>
            {
                (s.value_type, Some(s))
            }
            Resolution::Local(ty) => (ty, None),
            _ => return,
        };
    if !target_ty.is_known() || !value_ty.is_known() {
        return; // silent under Unknown
    }
    // T003 already flags integral-target + Float-value compound assigns (narrowing).
    // Skip that case here to avoid double-flagging the same node.
    if target_ty.is_integral() && value_ty == ValueType::Float {
        return;
    }
    if !compatible(target_ty, value_ty) {
        emit_mismatch(node, scope, target_ty, value_ty, target_sym, out);
    }
}

/// Emit a T030 diagnostic for a type mismatch, optionally linking the
/// declaration site for Channel/Parameter targets (#200).
fn emit_mismatch(
    node: &Node,
    scope: &Scope,
    target_ty: ValueType,
    value_ty: ValueType,
    target_sym: Option<&crate::symbols::Symbol>,
    out: &mut Vec<TypeDiagnostic>,
) {
    let target_str = render_type(target_ty, scope);
    let mut d = make(
        TypeCode::T030,
        node,
        Severity::Warning,
        format!(
            "assigning {} to a target of type {target_str}",
            render_type(value_ty, scope)
        ),
    );
    // The declaration is the other end of this fact (#200). Locals are
    // declared inline (no separate symbol), so there is no def to link.
    if let Some(sym) = target_sym {
        d.related.extend(crate::diagnostics::related_to_def(
            sym,
            format!("`{}` declared {target_str} here", sym.path),
        ));
    }
    out.push(d);
}

/// Render a `ValueType` for a user-facing message. Enums resolve to their name
/// via the symbol table (like T020), rather than leaking the internal id as
/// `Enum(0)` (#214); every other type uses its `Debug` shape.
fn render_type(ty: ValueType, scope: &Scope) -> String {
    if let (ValueType::Enum(id), Some(project)) = (ty, scope.project) {
        return format!("enum `{}`", project.symbols().enum_type(id).name);
    }
    format!("{ty:?}")
}

/// `local <Type> x = init;` — the declared annotation is authoritative (#142),
/// so an initialiser whose type is incompatible with it is the same class of
/// bug as a mismatched channel assignment. Also catches the sign error the
/// annotation makes expressible: a negative initialiser on an
/// `<Unsigned Integer>` local (numerically "compatible", semantically wrong).
fn check_local_declaration(node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    use m1_core::Field;
    let Some(declared) = node
        .child_by_field(Field::TypeAnnotation)
        .and_then(|a| a.child_by_field(Field::Type))
        .and_then(|t| crate::types::declared_local_type(t.text()))
    else {
        return;
    };
    let Some(init) = node.child_by_field(Field::Value) else {
        return;
    };
    if declared == ValueType::Unsigned && is_negative_literal(&init) {
        out.push(make(
            TypeCode::T030,
            node,
            Severity::Warning,
            "negative initial value for an `<Unsigned Integer>` local".into(),
        ));
        return;
    }
    let init_ty = type_of(init, scope);
    if !init_ty.is_known() {
        return;
    }
    if !compatible(declared, init_ty) {
        out.push(make(
            TypeCode::T030,
            node,
            Severity::Warning,
            format!(
                "initialising a local declared {} with a {} value",
                render_type(declared, scope),
                render_type(init_ty, scope)
            ),
        ));
    }
}

/// `-<number>` (a unary minus over a numeric literal).
fn is_negative_literal(n: &Node) -> bool {
    n.kind() == Kind::UnaryExpression
        && n.children().iter().any(|c| c.kind() == Kind::Minus)
        && n.named_children()
            .into_iter()
            .any(|c| c.kind() == Kind::Number)
}

/// Assignment compatibility (conservative).
fn compatible(target: ValueType, value: ValueType) -> bool {
    use ValueType::*;
    match (target, value) {
        (a, b) if a == b => true,
        // Integer/Unsigned interchangeable.
        (Integer, Unsigned) | (Unsigned, Integer) => true,
        // Integer channel accepts integral values; float channel accepts numerics.
        (Float, Integer) | (Float, Unsigned) => true,
        // Enum target only accepts the *same* enum.
        (Enum(a), Enum(b)) => a == b,
        _ => false, // e.g. Float -> Integer channel, Enum vs numeric, Boolean vs numeric
    }
}
