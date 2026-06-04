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
