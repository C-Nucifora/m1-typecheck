//! Diagnostics whose severity and trigger follow M1 Build's own validation.
//! Each test names the native error or warning it mirrors.
use m1_core::Severity;
use m1_typecheck::{diagnostics::TypeCode, project::Project, rules::check_script};
use std::path::Path;

fn project() -> Project {
    Project::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native_parity.m1prj"))
        .unwrap()
}

/// Findings for a body of the scheduled `Root.Parity.Update` function.
fn findings(source: &str) -> Vec<(TypeCode, Severity)> {
    check_script(&project(), Path::new("Parity.Update.m1scr"), source)
        .diagnostics
        .into_iter()
        .map(|d| (d.code, d.inner.severity))
        .collect()
}

fn has(source: &str, code: TypeCode, severity: Severity) -> bool {
    findings(source).contains(&(code, severity))
}

#[test]
fn unresolved_reads_are_errors_1338() {
    assert!(has(
        "local x = Parity.Missing;\nSystem.Unused(x);\n",
        TypeCode::T001,
        Severity::Error
    ));
}

#[test]
fn incompatible_assignments_are_errors_1320() {
    for source in ["B = PF;\n", "E = 3;\n", "I = PF;\n"] {
        assert!(has(source, TypeCode::T030, Severity::Error), "{source}");
    }
}

#[test]
fn integer_widening_and_signedness_are_accepted() {
    for source in ["F = PI;\n", "U = PI;\n", "I = PU;\n"] {
        assert!(
            !findings(source)
                .iter()
                .any(|(code, _)| *code == TypeCode::T030),
            "{source}"
        );
    }
}

#[test]
fn incompatible_typed_local_initialisers_are_errors_1316() {
    assert!(has(
        "local <Integer> x = 1.5;\nSystem.Unused(x);\n",
        TypeCode::T030,
        Severity::Error
    ));
}

#[test]
fn signed_unsigned_ordering_comparisons_are_errors_1367() {
    assert!(has(
        "local z = PI < PU;\nSystem.Unused(z);\n",
        TypeCode::T004,
        Severity::Error
    ));
}

#[test]
fn user_argument_type_mismatches_are_errors_1301() {
    assert!(has(
        "local r = This.Compute(PF);\nSystem.Unused(r);\n",
        TypeCode::T085,
        Severity::Error
    ));
}

fn codes(source: &str) -> Vec<TypeCode> {
    findings(source).into_iter().map(|(code, _)| code).collect()
}

/// Exactly one finding with `code`, an error unless `warning`.
fn single(source: &str, code: TypeCode, severity: Severity) {
    let all = findings(source);
    let matching: Vec<_> = all.iter().filter(|(c, _)| *c == code).collect();
    assert_eq!(matching.len(), 1, "{source}: {all:?}");
    assert_eq!(matching[0].1, severity, "{source}");
}

fn absent(source: &str, code: TypeCode) {
    assert!(
        !codes(source).contains(&code),
        "{source}: {:?}",
        findings(source)
    );
}

#[test]
fn unused_locals_are_errors_1350() {
    for source in [
        "local x = 1.0;\n",
        "static local x = 1.0;\n",
        "local x = 1.0;\nx = PF;\n",
        "local x = 1.0;\nx += PF;\n",
    ] {
        single(source, TypeCode::T118, Severity::Error);
    }
    for source in [
        "local x = PF > 1.0;\nif (x) { }\n",
        "local x = 1.0;\nSystem.Unused(x);\n",
        "local x = PF;\nF = x;\n",
    ] {
        absent(source, TypeCode::T118);
    }
}

#[test]
fn non_boolean_conditions_are_errors_1351_1359() {
    for source in [
        "if (PF) { }\n",
        "if (PI) { }\n",
        "if (PB) { } else if (PF) { }\n",
        "local x = PF ? 1.0 : 2.0;\nSystem.Unused(x);\n",
    ] {
        single(source, TypeCode::T119, Severity::Error);
    }
    for source in ["if (PB) { }\n", "if (PF > 1.0) { }\n", "if (PI > 0) { }\n"] {
        absent(source, TypeCode::T119);
    }
}

#[test]
fn literal_conditions_are_errors_1343_1344() {
    for source in [
        "if (true) { }\n",
        "if (false) { }\n",
        "if ((true)) { }\n",
        "local x = true ? 1.0 : 2.0;\nSystem.Unused(x);\n",
    ] {
        single(source, TypeCode::T120, Severity::Error);
    }
    absent("if (PB or true) { }\n", TypeCode::T120);
}

#[test]
fn literal_logic_results_are_warnings_1342() {
    for source in [
        "local b = true and false;\nSystem.Unused(b);\n",
        "if (1 > 0) { }\n",
        "local z = 1 > 0;\nSystem.Unused(z);\n",
    ] {
        single(source, TypeCode::T121, Severity::Warning);
    }
    for source in [
        "local z = PB and true;\nSystem.Unused(z);\n",
        "if (PB or true) { }\n",
    ] {
        absent(source, TypeCode::T121);
    }
}

#[test]
fn logic_and_comparison_statements_are_errors_1347() {
    for source in ["PB and PB;\n", "PF > 1.0;\n"] {
        single(source, TypeCode::T122, Severity::Error);
    }
    for source in ["PB;\n", "not PB;\n", "PF + 1.0;\n"] {
        absent(source, TypeCode::T122);
    }
}

#[test]
fn duplicate_locals_are_errors_1312() {
    for source in [
        "local a = 1.0;\nlocal a = 2.0;\nSystem.Unused(a);\n",
        "local a = 1.0;\nif (PB) {\n  local a = 2.0;\n  System.Unused(a);\n}\nSystem.Unused(a);\n",
    ] {
        single(source, TypeCode::T123, Severity::Error);
    }
    for source in [
        "local aa = 1.0;\nlocal AA = 2.0;\nSystem.Unused(aa);\nSystem.Unused(AA);\n",
        "if (PB) {\n  local aa = 1.0;\n  System.Unused(aa);\n} else {\n  local aa = 2.0;\n  System.Unused(aa);\n}\n",
    ] {
        absent(source, TypeCode::T123);
    }
}

#[test]
fn read_only_targets_are_errors_1318() {
    for source in ["PF = 1.0;\n", "Cap = 2;\n"] {
        single(source, TypeCode::T124, Severity::Error);
    }
    absent(
        "local p = PF;\np = p + 1.0;\nSystem.Unused(p);\n",
        TypeCode::T124,
    );
}

#[test]
fn argument_assignment_is_error_1357() {
    let report = check_script(
        &project(),
        Path::new("Parity.Compute.m1scr"),
        "In.Flag = true;\nOut = PF;\n",
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == TypeCode::T124 && d.inner.message.contains("1357")),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn reading_out_is_error_1356_and_void_functions_have_no_out_1338() {
    let compute = |source: &str| -> Vec<String> {
        check_script(&project(), Path::new("Parity.Compute.m1scr"), source)
            .diagnostics
            .into_iter()
            .filter(|d| d.code == TypeCode::T125)
            .map(|d| d.inner.message)
            .collect()
    };
    for source in [
        "Out = PF;\nlocal y = Out;\nSystem.Unused(y);\nSystem.Unused(In.Flag);\n",
        "Out = PF;\nif (Out > 0.0) { }\nSystem.Unused(In.Flag);\n",
    ] {
        let messages = compute(source);
        assert_eq!(messages.len(), 1, "{source}: {messages:?}");
        assert!(messages[0].contains("1356"));
    }
    assert!(compute("Out = PF;\nSystem.Unused(In.Flag);\n").is_empty());
    // `Root.Parity.Update` is scheduled and has no <Signature>: unknown, silent.
    absent("local y = Out;\nSystem.Unused(y);\n", TypeCode::T125);
}

#[test]
fn mixed_and_or_without_parentheses_is_error_1365() {
    for source in [
        "local z = PB and PB or PB;\nSystem.Unused(z);\n",
        "local z = PB or PB and PB;\nSystem.Unused(z);\n",
    ] {
        single(source, TypeCode::T126, Severity::Error);
    }
    for source in [
        "local z = (PB and PB) or PB;\nSystem.Unused(z);\n",
        "local z = PB and PB and PB;\nSystem.Unused(z);\n",
    ] {
        absent(source, TypeCode::T126);
    }
}

#[test]
fn operand_types_follow_1324_1329_1367() {
    for source in [
        "local z = not PF;\nSystem.Unused(z);\n",
        "local z = not PI;\nSystem.Unused(z);\n",
        "local z = PI and PB;\nSystem.Unused(z);\n",
        "local z = PF or PB;\nSystem.Unused(z);\n",
        "local z = PF > PI;\nSystem.Unused(z);\n",
    ] {
        single(source, TypeCode::T127, Severity::Error);
    }
    for source in [
        "local z = PI > 1.5;\nSystem.Unused(z);\n",
        "local z = PF > 1;\nSystem.Unused(z);\n",
        "local z = PU > 1;\nSystem.Unused(z);\n",
        "local z = not PB;\nSystem.Unused(z);\n",
        "local z = -PB;\nSystem.Unused(z);\n",
        "local z = ~PI;\nSystem.Unused(z);\n",
    ] {
        absent(source, TypeCode::T127);
    }
}

#[test]
fn calling_a_value_object_is_error_1336() {
    for source in ["F();\n", "PF();\n"] {
        single(source, TypeCode::T128, Severity::Error);
    }
}

#[test]
fn locals_named_like_visible_objects_are_errors_1311() {
    for name in ["F", "pf", "Driver", "Compute"] {
        single(
            &format!("local {name} = 1.0;\nSystem.Unused({name});\n"),
            TypeCode::T129,
            Severity::Error,
        );
    }
    // Library objects and objects deeper in the tree are not in scope.
    for name in ["Average", "Calculate"] {
        absent(
            &format!("local {name} = 1.0;\nSystem.Unused({name});\n"),
            TypeCode::T129,
        );
    }
}

#[test]
fn rejected_local_declarations_are_errors() {
    // 1025: no initial value.
    single(
        "local x;\nx = 1.0;\nSystem.Unused(x);\n",
        TypeCode::T130,
        Severity::Error,
    );
    // 1313: not M1 type names.
    for alias in ["Float", "Bool"] {
        single(
            &format!("local <{alias}> x = PB;\nSystem.Unused(x);\n"),
            TypeCode::T130,
            Severity::Error,
        );
    }
    for name in [
        "Boolean",
        "boolean",
        "Floating Point",
        "floating point",
        "Integer",
        "Unsigned Integer",
        "Drive State",
    ] {
        absent(
            &format!("local <{name}> x = PB;\nSystem.Unused(x);\n"),
            TypeCode::T130,
        );
    }
    // 1348: a library call returning nothing.
    let messages: Vec<String> = check_script(
        &project(),
        Path::new("Parity.Update.m1scr"),
        "local x = System.Unused(PF);\n",
    )
    .diagnostics
    .into_iter()
    .filter(|d| d.code == TypeCode::T130)
    .map(|d| d.inner.message)
    .collect();
    assert!(messages.iter().any(|m| m.contains("1348")), "{messages:?}");
}

#[test]
fn locals_initialised_from_void_user_functions_are_error_1314() {
    let project = Project::from_xml(
        r#"<?xml version="1.0"?>
<MoTeCM1BuildSession><Project Name="Void"><ComponentStream><List>
<Component Classname="BuiltIn.GroupCompound" Name="Root.G"/>
<Component Classname="BuiltIn.FuncUser" Filename="G.Update.m1scr" Name="Root.G.Update"/>
<Component Classname="BuiltIn.FuncUserParam" Filename="G.Act.m1scr" Name="Root.G.Act"><Signature Name=""><Params/></Signature></Component>
</List></ComponentStream></Project></MoTeCM1BuildSession>"#,
    )
    .unwrap();
    let report = check_script(
        &project,
        Path::new("G.Update.m1scr"),
        "local x = This.Act();\nSystem.Unused(x);\n",
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == TypeCode::T130 && d.inner.message.contains("1314")),
        "{:?}",
        report.diagnostics
    );
}
