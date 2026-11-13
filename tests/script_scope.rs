//! Script ownership must select a function and its group with the same policy.

use m1_typecheck::diagnostics::TypeCode;
use m1_typecheck::parsed;
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script;
use m1_typecheck::symbols::m1prj;
use m1_typecheck::types::ValueType;
use std::path::Path;

const EXPLICIT: &str = r#"
<Component Classname="BuiltIn.FuncUserParam" Name="Root.A.First" Filename="Run.m1scr">
 <Signature><Params><Param Name="Argument" Type="f32"/></Params></Signature>
</Component>"#;
const CONVENTIONAL: &str = r#"
<Component Classname="BuiltIn.FuncUserParam" Name="Root.Run">
 <Signature><Params><Param Name="Argument" Type="s32"/></Params></Signature>
</Component>"#;
const DUPLICATE: &str = r#"
<Component Classname="BuiltIn.FuncUserParam" Name="Root.Run" Filename="Run.m1scr">
 <Signature><Params><Param Name="Argument" Type="s32"/></Params></Signature>
</Component>"#;

#[test]
fn selected_function_group_rules_and_inference_agree_in_each_declaration_order() {
    for (declarations, function, group, expected_type) in [
        (
            format!("{EXPLICIT}{CONVENTIONAL}"),
            "Root.A.First",
            "Root.A",
            ValueType::Float,
        ),
        (
            format!("{CONVENTIONAL}{EXPLICIT}"),
            "Root.A.First",
            "Root.A",
            ValueType::Float,
        ),
        (
            format!("{EXPLICIT}{DUPLICATE}"),
            "Root.A.First",
            "Root.A",
            ValueType::Float,
        ),
        (
            format!("{DUPLICATE}{EXPLICIT}"),
            "Root.Run",
            "Root",
            ValueType::Integer,
        ),
        (
            EXPLICIT.to_string(),
            "Root.A.First",
            "Root.A",
            ValueType::Float,
        ),
        (
            CONVENTIONAL.to_string(),
            "Root.Run",
            "Root",
            ValueType::Integer,
        ),
    ] {
        let xml = format!(
            r#"<Project>
<Component Classname="BuiltIn.GroupCompound" Name="Root"/>
<Component Classname="BuiltIn.GroupCompound" Name="Root.A"/>
<Component Classname="BuiltIn.Channel" Name="Root.A.Value"><Props Type="f32"/></Component>
<Component Classname="BuiltIn.Channel" Name="Root.Value"><Props Type="s32"/></Component>
{declarations}
</Project>"#,
        );
        let mut project = Project::from_xml(&xml).unwrap();
        assert_eq!(
            project.function_symbol_for_script("Run.m1scr").as_deref(),
            Some(function)
        );
        assert_eq!(
            project.group_for_script("Run.m1scr").as_deref(),
            Some(group)
        );
        let parsed = m1prj::parse(&xml).unwrap();
        assert_eq!(
            parsed.file_to_group.get("Run.m1scr").map(String::as_str),
            Some(group)
        );

        let result = check_script(
            &project,
            Path::new("Run.m1scr"),
            "Out = This.Value eq 1; local x = In.Argument eq 1;\n",
        );
        assert!(result.syntax_errors.is_empty());
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|d| d.code == TypeCode::T002)
                .count(),
            if expected_type == ValueType::Float {
                2
            } else {
                0
            },
            "{declarations}"
        );
        // Both group-relative reads and function inputs must infer in the
        // selected scope, and only the selected function receives that type.
        for source in ["Out = This.Value;\n", "Out = In.Argument;\n"] {
            let scripts = parsed::parse_all(&[("Run.m1scr".into(), source.into())]);
            project.infer_return_types(&scripts);
            assert_eq!(
                project.symbols().get(function).unwrap().return_type,
                Some(expected_type)
            );
            let other = if function == "Root.Run" {
                "Root.A.First"
            } else {
                "Root.Run"
            };
            if let Some(symbol) = project.symbols().get(other) {
                assert_eq!(symbol.return_type, None);
            }
        }
    }
}
