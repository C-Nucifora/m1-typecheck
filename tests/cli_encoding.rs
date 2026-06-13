//! #86: a `.m1scr` source containing a Windows-1252 byte (e.g. `°` = 0xB0 in a
//! comment, as MoTeC writes yaw-rate units) must be read tolerantly, not abort
//! with "stream did not contain valid UTF-8". End-to-end via the built binary.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_m1-typecheck")
}

fn run(args: &[&str]) -> Output {
    Command::new(bin()).args(args).output().unwrap()
}

#[test]
fn script_with_windows_1252_byte_is_decoded_not_rejected() {
    // A `.m1scr` whose comment carries `°/s` stores the degree sign as the single
    // CP1252 byte 0xB0, which is invalid UTF-8. A strict `read_to_string` errors
    // with "stream did not contain valid UTF-8" before the file is even parsed.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_encoding_script");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let script = dir.join("Deg.m1scr");
    let mut bytes = b"// yaw ".to_vec();
    bytes.push(0xB0); // ° in Windows-1252
    bytes.extend_from_slice(b"/s\n");
    fs::write(&script, &bytes).unwrap();

    let o = run(&[script.to_str().unwrap()]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        !err.contains("valid UTF-8"),
        "the 1252 `.m1scr` must decode tolerantly, not fail the read; stderr:\n{err}"
    );
}

#[test]
fn leading_utf8_bom_does_not_produce_a_spurious_syntax_error() {
    // #213: a UTF-8 BOM (U+FEFF) at the start of a `.m1scr` must be stripped
    // before parsing. Otherwise the parser reports a spurious `1:1 syntax error`
    // and the real T002 float-equality warning on line 2 is masked.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_encoding_bom");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let body = "local x = 1.0;\nif (x eq 2.0) {\n}\n";
    let with_bom = dir.join("Bom.m1scr");
    let mut bytes = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
    bytes.extend_from_slice(body.as_bytes());
    fs::write(&with_bom, &bytes).unwrap();

    let o = run(&[with_bom.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&o.stdout);
    let stderr = String::from_utf8_lossy(&o.stderr);

    assert!(
        !stdout.contains("error[syntax]") && !stderr.contains("error[syntax]"),
        "a leading BOM must not yield a syntax error; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // The real diagnostic (T002 float-equality) must survive, exactly as it does
    // for the identical file without the BOM.
    assert!(
        stdout.contains("T002"),
        "the real T002 diagnostic must not be masked by the BOM; stdout:\n{stdout}"
    );
}

#[test]
fn library_api_strips_leading_bom_before_parsing() {
    // #213: the analysis entry points take a source string directly (the LSP and
    // other in-process callers never go through `m1_workspace::read_text`, which
    // strips the BOM on the file path). A BOM that reaches `run_with` raw must be
    // stripped there too, or it parses as a stray token: a spurious `1:1` syntax
    // error that also masks the real diagnostics. Exercises the in-repo strip.
    use m1_typecheck::rules::check_script_no_project;

    let body = "local x = 1.0;\nif (x eq 2.0) {\n}\n";
    let with_bom = format!("\u{feff}{body}");

    let r = check_script_no_project(&with_bom);
    assert!(
        r.syntax_errors.is_empty(),
        "a leading BOM must not produce a syntax error: {:?}",
        r.syntax_errors
    );
    // The real T002 float-equality diagnostic must survive, exactly as for the
    // identical BOM-less source.
    let bom_codes: Vec<_> = r.diagnostics.iter().map(|d| d.code).collect();
    let clean_codes: Vec<_> = check_script_no_project(body)
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect();
    assert_eq!(
        bom_codes, clean_codes,
        "BOM-prefixed source must yield the same diagnostics as the clean source"
    );
    assert!(
        clean_codes
            .iter()
            .any(|c| format!("{c:?}").contains("T002")),
        "sanity: the clean source should report T002, got {clean_codes:?}"
    );
}
