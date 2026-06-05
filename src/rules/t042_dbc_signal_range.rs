//! T042 — dbc-signal-range
//!
//! Flags a **literal** numeric value assigned to a CAN signal when it falls
//! outside the signal's physical range (derived from the `.m1dbc`
//! `Length`/`Type`/`Multiplier`/`Offset`). Only fires on literal right-hand
//! sides — a computed expression (`a + b`, a channel read) is left alone, since
//! its value is unknown at check time. Naturally silent without a project (no
//! DBC signal symbols exist, so nothing resolves to one).

use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::typer::path_text;
use m1_core::{Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::AssignmentStatement {
            return;
        }
        // Plain `=` only (not `+=` etc., whose result depends on the prior value).
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
            return; // single child; nothing assigned
        }
        // Only a literal numeric RHS — skip computed expressions.
        let Some(lit) = literal_number(value) else {
            return;
        };
        // Target must resolve to a CAN signal that carries a range.
        let Resolution::Symbol(sym) = resolve(&path_text(*target), scope) else {
            return;
        };
        let Some((min, max)) = sym.dbc_range else {
            return;
        };
        if lit < min || lit > max {
            out.push(make(
                TypeCode::T042,
                node,
                Severity::Warning,
                format!(
                    "value {lit} assigned to CAN signal `{}` is outside its range [{min}, {max}]",
                    sym.path
                ),
            ));
        }
    }
}

fn is_expr(k: Kind) -> bool {
    matches!(
        k,
        Kind::Identifier
            | Kind::MemberExpression
            | Kind::CallExpression
            | Kind::UnaryExpression
            | Kind::BinaryExpression
            | Kind::TernaryExpression
            | Kind::ParenthesizedExpression
            | Kind::Number
    )
}

/// The value of a literal numeric expression (`5`, `1.5`, `0xFF`, `-3`, `(7)`),
/// or `None` for anything computed (identifiers, arithmetic, calls).
fn literal_number(node: &Node) -> Option<f64> {
    match node.kind() {
        Kind::Number => parse_number(node.text()),
        Kind::ParenthesizedExpression => node
            .named_children()
            .iter()
            .find(|c| is_expr(c.kind()))
            .and_then(literal_number),
        Kind::UnaryExpression => {
            let operand = node
                .named_children()
                .into_iter()
                .find(|c| is_expr(c.kind()))?;
            let v = literal_number(&operand)?;
            let op = node.text().trim_start();
            if op.starts_with('-') {
                Some(-v)
            } else if op.starts_with('+') {
                Some(v)
            } else {
                None // `not`, etc. — not a numeric literal
            }
        }
        _ => None,
    }
}

fn parse_number(text: &str) -> Option<f64> {
    let t = text.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return parse_hex(hex);
    }
    t.parse::<f64>().ok()
}

/// Magnitude of a hex literal as `f64`. M1 hex constants are unsigned bit
/// patterns, so parse as `u64` (not the old signed `i64`, which both rejected
/// any value above `i64::MAX` — skipping the range check, #97 — and mis-signed a
/// top-bit-set value). Falls back to `u128` and finally to a digit-by-digit `f64`
/// accumulation so even a hex constant wider than 128 bits still yields a (large)
/// magnitude that feeds the range check rather than being dropped.
fn parse_hex(hex: &str) -> Option<f64> {
    if hex.is_empty() {
        return None;
    }
    if let Ok(n) = u64::from_str_radix(hex, 16) {
        return Some(n as f64);
    }
    if let Ok(n) = u128::from_str_radix(hex, 16) {
        return Some(n as f64);
    }
    let mut acc = 0.0_f64;
    for c in hex.chars() {
        let d = c.to_digit(16)?;
        acc = acc * 16.0 + d as f64;
    }
    Some(acc)
}
