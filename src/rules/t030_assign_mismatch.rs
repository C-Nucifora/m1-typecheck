use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use crate::typer::{is_expr, path_text, type_of};
use crate::types::ValueType;
use m1_core::{Kind, Node, Severity};

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
        // Only plain `=` (not `+=` etc., which imply the existing type).
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
        // Target must resolve to a Channel/Parameter with a known type.
        let target_sym = match resolve(&path_text(*target), scope) {
            Resolution::Symbol(s)
                if matches!(s.kind, SymbolKind::Channel | SymbolKind::Parameter) =>
            {
                s
            }
            _ => return,
        };
        let target_ty = target_sym.value_type;
        let value_ty = type_of(*value, scope);
        if !target_ty.is_known() || !value_ty.is_known() {
            return; // silent under Unknown
        }
        if !compatible(target_ty, value_ty) {
            let mut d = make(
                TypeCode::T030,
                node,
                Severity::Warning,
                format!("assigning {value_ty:?} to a target of type {target_ty:?}"),
            );
            // The declaration is the other end of this fact (#200).
            d.related.extend(crate::diagnostics::related_to_def(
                target_sym,
                format!("`{}` declared {target_ty:?} here", target_sym.path),
            ));
            out.push(d);
        }
    }
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
            format!("initialising a local declared {declared:?} with a {init_ty:?} value"),
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
