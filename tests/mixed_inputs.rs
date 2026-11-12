//! Unreadable positional inputs must not suppress readable source reports.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str, source: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Project.m1prj"), "<Project/>\n").unwrap();
    fs::write(dir.join("parameters.m1cfg"), "<Configuration/>\n").unwrap();
    fs::write(dir.join("m1-tools.toml"), "").unwrap();
    fs::write(dir.join("Readable.m1scr"), source).unwrap();
    dir
}

fn run(dir: &Path, format: &str, inputs: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_m1-typecheck"))
        .current_dir(dir)
        .args([
            "--project",
            "Project.m1prj",
            "--config",
            "parameters.m1cfg",
            "--select",
            "T002",
            "--format",
            format,
        ])
        .args(inputs)
        .output()
        .unwrap()
}

#[test]
fn mixed_missing_inputs_preserve_semantic_and_syntax_reports_in_every_format() {
    for (name, source, expected) in [
        ("mixed_semantic", "local f = 1.5; if (f eq 2) {}\n", "T002"),
        ("mixed_syntax", "local f = ;\n", "syntax"),
    ] {
        let dir = fixture(name, source);
        for format in ["human", "json", "sarif"] {
            let readable = run(&dir, format, &["Readable.m1scr"]);
            assert_eq!(readable.status.code(), Some(1));
            let stdout = String::from_utf8_lossy(&readable.stdout);
            assert!(stdout.contains(expected), "{format}: {stdout}");
            if format == "json" {
                let json: serde_json::Value = serde_json::from_slice(&readable.stdout).unwrap();
                assert_eq!(json["files"].as_array().unwrap().len(), 1);
                if expected == "T002" {
                    assert_eq!(json["files"][0]["diagnostics"][0]["code"], "T002");
                } else {
                    assert!(
                        !json["files"][0]["syntax_errors"]
                            .as_array()
                            .unwrap()
                            .is_empty()
                    );
                }
            } else if format == "sarif" {
                let json: serde_json::Value = serde_json::from_slice(&readable.stdout).unwrap();
                assert!(
                    json["runs"][0]["results"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|result| result["ruleId"] == expected)
                );
            }
            for inputs in [
                ["Readable.m1scr", "Absent.m1scr"],
                ["Absent.m1scr", "Readable.m1scr"],
            ] {
                let mixed = run(&dir, format, &inputs);
                assert_eq!(mixed.status.code(), Some(1), "{inputs:?}");
                assert!(String::from_utf8_lossy(&mixed.stderr).contains("Absent.m1scr"));
                // Compare complete reports, including ranges and summaries.
                assert_eq!(mixed.stdout, readable.stdout, "{format}: {inputs:?}");
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn missing_inputs_still_fail_when_readable_source_is_valid_or_suppressed() {
    for (name, source) in [
        ("mixed_valid", "Out = 1;\n"),
        (
            "mixed_suppressed",
            "local f = 1.5;\n// @m1:allow(T002)\nif (f eq 2) {}\n",
        ),
    ] {
        let dir = fixture(name, source);
        let readable = run(&dir, "json", &["Readable.m1scr"]);
        assert_eq!(readable.status.code(), Some(0));
        let json: serde_json::Value = serde_json::from_slice(&readable.stdout).unwrap();
        assert_eq!(json["files"].as_array().unwrap().len(), 1);
        assert!(
            json["files"][0]["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for inputs in [
            ["Readable.m1scr", "Absent.m1scr"],
            ["Absent.m1scr", "Readable.m1scr"],
        ] {
            let mixed = run(&dir, "json", &inputs);
            assert_eq!(mixed.status.code(), Some(1));
            assert!(String::from_utf8_lossy(&mixed.stderr).contains("Absent.m1scr"));
            assert_eq!(mixed.stdout, readable.stdout);
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
