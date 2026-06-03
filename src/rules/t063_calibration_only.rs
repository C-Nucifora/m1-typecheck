//! T063 `calibration-only-call`: the calibration-method library functions
//! (`Math.*`, `UI.*`, `System.StrCat`) are valid only inside M1 Tune calibration
//! methods, never in ECU `.m1scr` scripts. Flag a direct call to one. An object
//! whose function set is mixed (e.g. `System`, which has both ECU and calibration
//! functions) is flagged only for the calibration-only members.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::{Resolution, Scope, resolve};
use crate::typer::path_text;
use m1_core::{Field, Kind, Node, Severity};

pub struct Rule;

impl super::Rule for Rule {
    fn check_node(&self, node: &Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
        if node.kind() != Kind::CallExpression {
            return;
        }
        let Some(callee) = node.child_by_field(Field::Function) else {
            return;
        };
        let path = path_text(callee);
        let Resolution::BuiltinFn(overloads) = resolve(&path, scope) else {
            return;
        };
        // Flag only when every overload bound to this call is calibration-only, so
        // an ECU function that shares a name (e.g. `System.Debug`) is not caught.
        if overloads.is_empty() || !overloads.iter().all(|o| o.calibration_only) {
            return;
        }
        out.push(make(
            TypeCode::T063,
            node,
            Severity::Error,
            format!(
                "`{path}` is a calibration-only function: it can be used only in \
                 M1 Tune calibration methods, not in an ECU script"
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use crate::diagnostics::TypeCode;
    use crate::rules::check_script_no_project;

    fn codes(src: &str) -> Vec<TypeCode> {
        check_script_no_project(src)
            .diagnostics
            .into_iter()
            .map(|d| d.code)
            .collect()
    }

    #[test]
    fn calibration_only_call_is_flagged() {
        assert!(codes("x = Math.Sqrt(a);\n").contains(&TypeCode::T063));
        assert!(codes("UI.PromptOK(\"t\", \"m\");\n").contains(&TypeCode::T063));
        assert!(codes("x = System.StrCat(a, b);\n").contains(&TypeCode::T063));
    }

    #[test]
    fn ecu_library_call_is_not_flagged() {
        // ECU functions, including the ECU System.Preserve / System.Debug, are fine.
        assert!(!codes("x = Calculate.Max(a, b);\n").contains(&TypeCode::T063));
        assert!(!codes("System.Preserve();\n").contains(&TypeCode::T063));
        assert!(!codes("System.Debug(\"hi\");\n").contains(&TypeCode::T063));
    }
}
