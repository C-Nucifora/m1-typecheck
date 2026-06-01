//! T003 — float→integer narrowing.
//!
//! M1 Build implicitly *widens* integral operands to float in arithmetic, so a
//! plain `float OP integer` expression is well-defined and not flagged (#17:
//! flagging all such mixing floods the corpus once call-return types are known —
//! 243 diagnostics on cleanly-building code). The genuinely lossy case is the
//! reverse: storing a float *back into an integral target* via a compound
//! assignment (`intCh += floatVal`), which M1 will not do implicitly. That
//! narrowing is what T003 now flags; widening in expressions, and plain `=`
//! assignment mismatches (T030), are out of scope here.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::typer::type_of;
use crate::types::ValueType;
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() == Kind::AssignmentStatement {
            self.check_compound_assign(node, scope, out);
        }
    }
}

impl Rule {
    /// Compound assignment (`+=`, `-=`, `*=`, `/=`) whose target is integral and
    /// whose value is a float narrows on store: the float result is truncated
    /// back into the integer target. Flag it; the reverse (`floatCh += intVal`)
    /// is safe widening and is left alone.
    fn check_compound_assign(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        let Some(op) = node.child_by_field(Field::Operator) else {
            return;
        };
        if !matches!(
            op.kind(),
            Kind::PlusEq | Kind::MinusEq | Kind::StarEq | Kind::SlashEq
        ) {
            return;
        }
        let (Some(target), Some(value)) = (
            node.child_by_field(Field::Target),
            node.child_by_field(Field::Value),
        ) else {
            return;
        };
        if type_of(target, scope).is_integral() && type_of(value, scope) == ValueType::Float {
            out.push(make(
                TypeCode::T003,
                node,
                Severity::Warning,
                "float value narrows to an integer target; use Convert.ToInteger".into(),
            ));
        }
    }
}
