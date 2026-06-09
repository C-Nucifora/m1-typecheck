//! Cross-script invalid-value propagation (#78 P3): the project-wide fixpoint
//! solve plus the seeded per-file reporting pass, end-to-end against synthetic
//! projects. All identifiers are synthetic placeholders.
use m1_typecheck::cross_script::{ChannelTaints, solve};
use m1_typecheck::diagnostics::{TypeCode, TypeDiagnostic};
use m1_typecheck::project::Project;
use m1_typecheck::rules::check_script_with_channels;
use std::collections::HashSet;
use std::path::Path;

fn project() -> Project {
    Project::from_xml(
        r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Sensors"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Nav"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Control"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Sensors.Yaw"><Props Type="f32"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Nav.Heading"><Props Type="f32"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Control.Demand"><Props Type="f32"/></Component>
  <Component Classname="BuiltIn.Channel" Name="Root.Control.Count"><Props Type="u32"/></Component>
  <Component Classname="BuiltIn.FuncUser" Name="Root.Nav.Ratio"/>
</Project>"#,
    )
    .unwrap()
}

/// Run the solved reporting pass over one script.
fn report(p: &Project, file: &str, src: &str, taints: &ChannelTaints) -> Vec<TypeDiagnostic> {
    check_script_with_channels(&HashSet::new(), Some(p), Some(Path::new(file)), src, taints)
        .diagnostics
}

fn scripts(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(f, s)| (f.to_string(), s.to_string()))
        .collect()
}

fn find_code(diags: &[TypeDiagnostic], code: TypeCode) -> Option<&TypeDiagnostic> {
    diags.iter().find(|d| d.code == code)
}

const TAINTING_WRITER: (&str, &str) = (
    "Sensors.Update.m1scr",
    "Sensors.Yaw = 1 / Control.Demand;\n",
);

#[test]
fn tainted_write_is_solved_and_reaches_remote_annotated_sink() {
    let p = project();
    let reader_src = "// @m1:requires-finite\nControl.Demand = Sensors.Yaw * 2;\n";
    let taints = solve(
        &p,
        &scripts(&[TAINTING_WRITER, ("Control.Update.m1scr", reader_src)]),
    );
    assert!(
        taints.get("Root.Sensors.Yaw").is_some(),
        "the division-fed channel is solved tainted"
    );
    let diags = report(&p, "Control.Update.m1scr", reader_src, &taints);
    let d = find_code(&diags, TypeCode::T080).expect("cross-script T080 at the annotated sink");
    assert!(
        d.inner.message.contains("Sensors.Update.m1scr"),
        "provenance names the writing script: {}",
        d.inner.message
    );
}

#[test]
fn no_solve_no_cross_script_diagnostic() {
    // Without the solved taints the same sink stays quiet — the cross-script
    // story only appears when the project-wide pass ran.
    let p = project();
    let reader_src = "// @m1:requires-finite\nControl.Demand = Sensors.Yaw * 2;\n";
    let diags = report(
        &p,
        "Control.Update.m1scr",
        reader_src,
        &ChannelTaints::default(),
    );
    assert!(find_code(&diags, TypeCode::T080).is_none());
}

#[test]
fn clean_writer_produces_no_taint() {
    let p = project();
    let taints = solve(
        &p,
        &scripts(&[(
            "Sensors.Update.m1scr",
            "Sensors.Yaw = Control.Demand + 1;\n",
        )]),
    );
    assert!(taints.is_empty(), "no invalid-value source, no taint");
}

#[test]
fn fixpoint_propagates_through_a_copy_chain_regardless_of_script_order() {
    let p = project();
    // Listed reader-first: a single forward pass would miss the chain, the
    // fixpoint must not.
    let taints = solve(
        &p,
        &scripts(&[
            ("Nav.Update.m1scr", "Nav.Heading = Sensors.Yaw;\n"),
            TAINTING_WRITER,
        ]),
    );
    let t = taints
        .get("Root.Nav.Heading")
        .expect("taint propagates through the pure copy");
    assert_eq!(t.chain.len(), 2, "two-step provenance: {:?}", t.chain);
    assert!(t.chain[0].contains("Nav.Update.m1scr"), "{:?}", t.chain);
    assert!(t.chain[1].contains("Sensors.Update.m1scr"), "{:?}", t.chain);
}

#[test]
fn feedback_cycle_terminates_with_finite_chain() {
    let p = project();
    let taints = solve(
        &p,
        &scripts(&[
            (
                "Sensors.Update.m1scr",
                "Sensors.Yaw = Nav.Heading + (1 / Control.Demand);\n",
            ),
            ("Nav.Update.m1scr", "Nav.Heading = Sensors.Yaw;\n"),
        ]),
    );
    let yaw = taints
        .get("Root.Sensors.Yaw")
        .expect("cycle member tainted");
    let heading = taints
        .get("Root.Nav.Heading")
        .expect("cycle member tainted");
    assert!(yaw.chain.len() <= 3, "finite chain: {:?}", yaw.chain);
    assert!(
        heading.chain.len() <= 3,
        "finite chain: {:?}",
        heading.chain
    );
}

#[test]
fn stateful_function_latches_across_scripts() {
    let p = project();
    let sink_src = "// @m1:requires-finite\nControl.Demand = Nav.Heading;\n";
    let taints = solve(
        &p,
        &scripts(&[
            TAINTING_WRITER,
            (
                "Nav.Update.m1scr",
                "Nav.Heading = Filter.FirstOrder(Sensors.Yaw, 0.1);\n",
            ),
            ("Control.Update.m1scr", sink_src),
        ]),
    );
    assert!(
        taints.get("Root.Nav.Heading").expect("tainted").latched,
        "the filter latches the remote taint"
    );
    let diags = report(&p, "Control.Update.m1scr", sink_src, &taints);
    let d = find_code(&diags, TypeCode::T080).expect("T080 at the sink");
    assert_eq!(
        d.inner.severity,
        m1_core::Severity::Error,
        "latched taint escalates a @requires-finite sink to Error"
    );
}

#[test]
fn sanitizes_annotation_is_a_cross_script_barrier() {
    let p = project();
    let sink_src = "// @m1:requires-finite\nControl.Demand = Nav.Heading;\n";
    let taints = solve(
        &p,
        &scripts(&[
            TAINTING_WRITER,
            (
                "Nav.Update.m1scr",
                "// @m1:sanitizes\nNav.Heading = Sensors.Yaw;\n",
            ),
            ("Control.Update.m1scr", sink_src),
        ]),
    );
    assert!(
        taints.get("Root.Nav.Heading").is_none(),
        "the sanitised rewrite is a taint barrier"
    );
    let diags = report(&p, "Control.Update.m1scr", sink_src, &taints);
    assert!(find_code(&diags, TypeCode::T080).is_none());
}

#[test]
fn nan_to_integer_fires_cross_script() {
    let p = project();
    // No annotation anywhere in the reader: the implicit integer-conversion
    // sink (#120) catches the remote NaN.
    let reader_src = "Control.Count = Sensors.Yaw;\n";
    let taints = solve(
        &p,
        &scripts(&[TAINTING_WRITER, ("Control.Update.m1scr", reader_src)]),
    );
    let diags = report(&p, "Control.Update.m1scr", reader_src, &taints);
    let d = find_code(&diags, TypeCode::T081).expect("cross-script T081 at the integer store");
    assert!(
        d.inner.message.contains("Sensors.Update.m1scr"),
        "provenance names the writing script: {}",
        d.inner.message
    );
}

#[test]
fn function_out_taint_reaches_call_sites() {
    let p = project();
    // `Nav.Ratio.m1scr` backs `Root.Nav.Ratio` by the path convention; its
    // `Out =` is the function's return value, read at the remote call site.
    let caller_src = "// @m1:requires-finite\nControl.Demand = Nav.Ratio();\n";
    let taints = solve(
        &p,
        &scripts(&[
            ("Nav.Ratio.m1scr", "Out = 1 / Control.Demand;\n"),
            ("Control.Update.m1scr", caller_src),
        ]),
    );
    assert!(
        taints.get("Root.Nav.Ratio").is_some(),
        "the function symbol carries its Out taint"
    );
    let diags = report(&p, "Control.Update.m1scr", caller_src, &taints);
    let d = find_code(&diags, TypeCode::T080).expect("T080 at the calling sink");
    assert!(
        d.inner.message.contains("Nav.Ratio.m1scr"),
        "provenance names the callee script: {}",
        d.inner.message
    );
}

#[test]
fn explain_resolves_bare_and_canonical_spellings() {
    use m1_typecheck::cross_script::explain;
    let p = project();
    let taints = solve(&p, &scripts(&[TAINTING_WRITER]));

    // Both spellings of the tainted channel resolve to the same explanation.
    let bare = explain(&p, &taints, "Sensors.Yaw").expect("bare spelling resolves");
    let full = explain(&p, &taints, "Root.Sensors.Yaw").expect("canonical spelling resolves");
    assert_eq!(bare.channel, "Root.Sensors.Yaw");
    assert_eq!(full.channel, "Root.Sensors.Yaw");
    assert!(bare.taint.is_some(), "tainted channel explains its chain");

    // A known-but-clean channel explains as untainted.
    let clean = explain(&p, &taints, "Nav.Heading").expect("known channel");
    assert!(clean.taint.is_none());

    // An unknown symbol is a caller error (the CLI exits 2 on None).
    assert!(explain(&p, &taints, "Nope.Missing").is_none());
}
