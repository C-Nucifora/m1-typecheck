use m1_typecheck::symbols::{SymbolKind, m1prj};
use std::path::Path;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mini.m1prj")
}

#[test]
fn parses_symbols_and_kinds() {
    let parsed = m1prj::parse(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    let table = &parsed.table;
    assert_eq!(
        table.get("Root.Foo.Speed").unwrap().kind,
        SymbolKind::Channel
    );
    assert_eq!(
        table.get("Root.Foo.Gain.Value").unwrap().kind,
        SymbolKind::Parameter
    );
    assert_eq!(
        table.get("Root.Foo.Limit").unwrap().kind,
        SymbolKind::Constant
    );
    assert_eq!(
        table.get("Root.Foo.Update").unwrap().kind,
        SymbolKind::Function
    );
    assert_eq!(
        table.get("Root.Foo.Startup").unwrap().kind,
        SymbolKind::Method
    );
    assert_eq!(table.get("Root.Foo").unwrap().kind, SymbolKind::Group);
}

#[test]
fn parses_enum_types() {
    let parsed = m1prj::parse(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    let e = parsed
        .table
        .enums()
        .iter()
        .find(|e| e.name == "Switch State")
        .unwrap();
    assert_eq!(e.default.as_deref(), Some("Off"));
    assert_eq!(
        e.members,
        vec![("Off".to_string(), 0), ("On".to_string(), 1)]
    );
}

#[test]
fn maps_filename_to_enclosing_group() {
    let parsed = m1prj::parse(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    assert_eq!(
        parsed
            .file_to_group
            .get("Foo Update.m1scr")
            .map(String::as_str),
        Some("Root.Foo")
    );
}

#[test]
fn collects_module_set_roots() {
    let parsed = m1prj::parse(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    assert!(parsed.module_roots.contains("MoTeC Comms"));
}

#[test]
fn def_line_records_the_component_source_line() {
    // The per-component `def_line` must be the 0-based source line of that
    // `<Component>` — the value the (now O(1)-amortized) line lookup computes (#95).
    let xml = "<?xml version=\"1.0\"?>\n\
        <Project>\n\
        <Component Classname=\"BuiltIn.ChannelScalar\" Name=\"Root.A\"/>\n\
        <Component Classname=\"BuiltIn.ChannelScalar\" Name=\"Root.B\"/>\n\
        </Project>\n";
    let parsed = m1prj::parse(xml).unwrap();
    // Lines are 0-based; the two components sit on source lines 2 and 3.
    assert_eq!(parsed.table.get("Root.A").unwrap().def_line, Some(2));
    assert_eq!(parsed.table.get("Root.B").unwrap().def_line, Some(3));
}

#[test]
fn many_component_project_parses_in_near_linear_time() {
    // #95: line numbers used to be computed with roxmltree's `text_pos_at`, which
    // rescans from byte 0 per `<Component>` — O(N^2) overall. With a precomputed
    // line-start index the parse is ~linear. A many-component project must parse
    // well within a generous wall-clock bound (the old O(N^2) path blew past it at
    // this size; the bound is loose enough that a slow CI box won't flake).
    let n = 50_000;
    let mut xml = String::with_capacity(n * 80 + 64);
    xml.push_str("<?xml version=\"1.0\"?>\n<Project>\n");
    for i in 0..n {
        xml.push_str(&format!(
            "<Component Classname=\"BuiltIn.ChannelScalar\" Name=\"Root.C{i}\"/>\n"
        ));
    }
    xml.push_str("</Project>\n");

    let start = std::time::Instant::now();
    let parsed = m1prj::parse(&xml).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(
        parsed
            .table
            .get(&format!("Root.C{}", n - 1))
            .map(|s| s.kind),
        Some(SymbolKind::Channel),
        "every component must parse"
    );
    // O(N^2) at 50k components ran into the multi-second range in the audit;
    // linear parsing is well under a second. 10s is a deliberately loose ceiling.
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "parsing {n} components took {elapsed:?}; expected near-linear (O(N^2) regression?)"
    );
}

#[test]
fn m1cfg_augments_parameter_types() {
    use m1_typecheck::symbols::{m1cfg, m1prj};
    use m1_typecheck::types::ValueType;
    let mut parsed = m1prj::parse(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    let cfg = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mini.m1cfg"),
    )
    .unwrap();
    m1cfg::augment(&mut parsed.table, &cfg).unwrap();
    let gain = parsed.table.get("Root.Foo.Gain.Value").unwrap();
    assert_eq!(gain.value_type, ValueType::Float);
    assert_eq!(gain.unit.as_deref(), Some("ratio"));
}
