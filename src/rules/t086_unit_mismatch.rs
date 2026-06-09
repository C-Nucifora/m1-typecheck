//! T086 — unit-mismatch (#144). The manual ("Quantities and Base Units"):
//! every floating-point object carries a physical quantity, and all
//! calculations happen in base units. Copying a value between channels of
//! *different* quantities (`Pressure = Wheel Speed;`) is a physical-units bug
//! the type lattice can't see — both sides are just Float.
//!
//! Deliberately conservative (first consumer of `units.rs`): only DIRECT
//! symbol-to-symbol assignments are compared, exact base-unit equality.
//! Arithmetic legitimately changes dimension (`m/s * s` is a length), so any
//! computed right-hand side stays unchecked until a full dimensional algebra
//! exists. A unitless side (no `Qty`) never flags.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::symbols::SymbolKind;
use crate::typer::{is_expr, path_text};
use m1_core::{Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::AssignmentStatement {
            return;
        }
        // Plain `=` only: a compound op is arithmetic on the target itself.
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
            return;
        }
        // Unwrap redundant parens around a bare symbol read.
        let mut rhs = *value;
        while rhs.kind() == Kind::ParenthesizedExpression {
            match rhs.named_children().into_iter().find(|c| is_expr(c.kind())) {
                Some(inner) => rhs = inner,
                None => return,
            }
        }
        if !matches!(rhs.kind(), Kind::Identifier | Kind::MemberExpression) {
            return; // computed RHS: dimension may legitimately change
        }
        let unit_of = |n: Node| match resolve(&path_text(n), scope) {
            Resolution::Symbol(s)
                if matches!(s.kind, SymbolKind::Channel | SymbolKind::Parameter) =>
            {
                s.unit.clone().map(|u| (s.path.clone(), u))
            }
            _ => None,
        };
        let (Some((tpath, tunit)), Some((spath, sunit))) = (unit_of(*target), unit_of(rhs)) else {
            return;
        };
        if tunit != sunit {
            out.push(make(
                TypeCode::T086,
                node,
                Severity::Warning,
                format!("unit mismatch: `{tpath}` is in {tunit} but `{spath}` is in {sunit}"),
            ));
        }
    }
}
