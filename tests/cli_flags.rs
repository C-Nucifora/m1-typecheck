//! CLI surface (#88): `--select` / `--ignore` / `--format json` / `--rules`, plus
//! the cfg/dbc visibility notes (#87 part 2). End-to-end via the built binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_m1-typecheck")
}

// A project declaring a parameter (`Root.Foo.Gain.Value`). Paired with a cfg that
// loads but omits that parameter (`EMPTY_CONFIG`), the T041 project audit emits a
// warning, giving us a stable T-code to filter on.
const PROJECT: &str = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="Demo" TargetHardware="ecu120">
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Foo"/>
    <Component Classname="BuiltIn.Parameter" Name="Root.Foo.Gain.Value"><Props/></Component>
    <Component Classname="BuiltIn.FuncUser" Filename="Foo Update.m1scr" Name="Root.Foo.Update"/>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

const CONFIG: &str = r#"<?xml version="1.0"?>
<Configuration>
 <Group Name="">
  <Parameter Name="Root.Foo.Gain.Value"><Cell Type="u16" Unit="ratio"><![CDATA[1]]></Cell></Parameter>
 </Group>
</Configuration>
"#;

// A cfg that loads but does NOT cover `Root.Foo.Gain.Value` — so the T041
// project audit fires (the parameter is declared in the .m1prj but absent here).
const EMPTY_CONFIG: &str = r#"<?xml version="1.0"?>
<Configuration>
 <Group Name=""/>
</Configuration>
"#;

// A float compared with == (T002) plus a wrong-arity library call (opt-in T064).
const SCRIPT: &str = "local f = 1.5; if (f == 2.5) { }\nlocal x = Calculate.Max(1, 2, 3);\n";

fn setup(name: &str, cfg: Option<&str>) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Project.m1prj"), PROJECT).unwrap();
    if let Some(c) = cfg {
        fs::write(dir.join("parameters.m1cfg"), c).unwrap();
    }
    let script = dir.join("Foo Update.m1scr");
    fs::write(&script, SCRIPT).unwrap();
    script
}

fn run(args: &[&str]) -> Output {
    Command::new(bin()).args(args).output().unwrap()
}
fn out_of(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err_of(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn rules_lists_every_t_code() {
    let o = run(&["--rules"]);
    let s = out_of(&o);
    assert!(s.contains("T002  float-equality"), "got:\n{s}");
    assert!(s.contains("T041  missing-cfg-parameter"));
    assert!(s.contains("T064  wrong-argument-count"));
}

#[test]
fn rules_json_is_valid_and_covers_codes() {
    let o = run(&["--rules", "--format", "json"]);
    let s = out_of(&o);
    let v: serde_json::Value = serde_json::from_str(&s).expect("valid JSON");
    let rules = v["rules"].as_array().unwrap();
    let codes: Vec<&str> = rules.iter().map(|r| r["code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"T001"));
    assert!(codes.contains(&"T064"));
}

#[test]
fn format_json_emits_valid_json() {
    let script = setup("cli_json", Some(EMPTY_CONFIG));
    let o = run(&["--format", "json", script.to_str().unwrap()]);
    let s = out_of(&o);
    let v: serde_json::Value = serde_json::from_str(&s).expect("valid JSON document");
    assert_eq!(v["version"], 1);
    // The T002 float-equality diagnostic is present in the file's diagnostics.
    let diags = v["files"][0]["diagnostics"].as_array().unwrap();
    assert!(
        diags.iter().any(|d| d["code"] == "T002"),
        "expected T002 in JSON, got:\n{s}"
    );
    // The T041 project audit lands in the project array (cfg omits the param).
    let proj = v["project"].as_array().unwrap();
    assert!(
        proj.iter().any(|d| d["code"] == "T041"),
        "expected T041 audit"
    );
}

#[test]
fn ignore_suppresses_a_code() {
    let script = setup("cli_ignore", Some(EMPTY_CONFIG));
    // Without --ignore, T041 is reported.
    let base = out_of(&run(&[script.to_str().unwrap()]));
    assert!(base.contains("T041"), "baseline should show T041:\n{base}");
    // --ignore T041 removes it (but keeps the others, e.g. T002).
    let filtered = out_of(&run(&["--ignore", "T041", script.to_str().unwrap()]));
    assert!(
        !filtered.contains("T041"),
        "T041 should be gone:\n{filtered}"
    );
    assert!(filtered.contains("T002"), "T002 should remain:\n{filtered}");
}

#[test]
fn select_whitelists_only_named_codes() {
    let script = setup("cli_select", Some(EMPTY_CONFIG));
    let s = out_of(&run(&["--select", "T002", script.to_str().unwrap()]));
    assert!(s.contains("T002"), "T002 should be present:\n{s}");
    assert!(!s.contains("T041"), "T041 should be filtered out:\n{s}");
}

#[test]
fn select_activates_opt_in_t064() {
    let script = setup("cli_optin", None);
    // T064 is opt-in: silent by default...
    let off = out_of(&run(&[script.to_str().unwrap()]));
    assert!(!off.contains("T064"), "T064 must be off by default:\n{off}");
    // ...and on only when explicitly selected.
    let on = out_of(&run(&["--select", "T064", script.to_str().unwrap()]));
    assert!(on.contains("T064"), "T064 should fire when selected:\n{on}");
}

#[test]
fn note_when_no_cfg_or_dbc_discovered() {
    // #87 part 2: a project that loads but finds no cfg/dbc must announce the
    // skipped audits on stderr, so a green CI run isn't mistaken for "all clean".
    let script = setup("cli_note_absent", None);
    let e = err_of(&run(&[script.to_str().unwrap()]));
    assert!(
        e.contains("no parameters.m1cfg found") && e.contains("T041"),
        "expected the cfg-skipped note, got:\n{e}"
    );
    assert!(
        e.contains("no .m1dbc found") && e.contains("T042"),
        "expected the dbc-skipped note, got:\n{e}"
    );
}

#[test]
fn no_cfg_note_when_cfg_present() {
    // With a cfg present, the cfg-skipped note must NOT appear (the dbc note may,
    // since this fixture ships no .m1dbc).
    let script = setup("cli_note_present", Some(CONFIG));
    let e = err_of(&run(&[script.to_str().unwrap()]));
    assert!(
        !e.contains("no parameters.m1cfg found"),
        "cfg note must be absent when a cfg is present, got:\n{e}"
    );
}

#[test]
fn unknown_filter_code_is_rejected() {
    // #111: a bogus --ignore/--select code must be rejected, not silently
    // accepted (which hid typos that disabled nothing).
    let o = run(&["--ignore", "NOTACODE"]);
    assert_ne!(
        o.status.code(),
        Some(0),
        "an unknown --ignore code must exit non-zero; stderr:\n{}",
        err_of(&o)
    );
    let e = err_of(&o).to_lowercase();
    assert!(
        e.contains("notacode") || e.contains("unknown"),
        "stderr should name the bad code:\n{}",
        err_of(&o)
    );
    // A valid code is still accepted (no spurious rejection).
    let ok = run(&["--ignore", "T041"]);
    assert_eq!(ok.status.code(), Some(0), "a valid code must be accepted");
}

// A clean script (no diagnostics) so a tolerated filter run exits 0, mirroring
// the bug repro (`printf 'Out = 1;\n'`).
const CLEAN_SCRIPT: &str = "Out = 1;\n";

fn setup_clean(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let script = dir.join("clean.m1scr");
    fs::write(&script, CLEAN_SCRIPT).unwrap();
    script
}

#[test]
fn empty_filter_value_is_a_noop() {
    // An empty `--ignore ""` (and whitespace-only `--select "  "`) must be
    // tolerated as "no codes" rather than rejected as an unknown empty code
    // (exit 2), matching m1-lint. On a clean file this is a clean exit 0.
    let script = setup_clean("cli_empty_ignore");
    let o = run(&["--ignore", "", script.to_str().unwrap()]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "empty --ignore must be a no-op (exit 0); stderr:\n{}",
        err_of(&o)
    );

    let ws = run(&["--select", "  ", script.to_str().unwrap()]);
    assert_eq!(
        ws.status.code(),
        Some(0),
        "whitespace-only --select must be a no-op (exit 0); stderr:\n{}",
        err_of(&ws)
    );

    // The empty token must not poison validation of a real run with diagnostics:
    // an empty --ignore leaves the baseline diagnostics intact.
    let proj = setup("cli_empty_ignore_proj", Some(EMPTY_CONFIG));
    let p = run(&["--ignore", "", proj.to_str().unwrap()]);
    assert_ne!(
        p.status.code(),
        Some(2),
        "empty --ignore must not be a validation error; stderr:\n{}",
        err_of(&p)
    );
    assert!(
        out_of(&p).contains("T041"),
        "empty --ignore must not suppress anything:\n{}",
        out_of(&p)
    );
}

#[test]
fn trailing_comma_in_filter_is_tolerated() {
    // A trailing comma (`T041,`) splits to an empty trailing token; that empty
    // entry must be skipped, while the real code is still applied.
    let script = setup("cli_trailing_comma", Some(EMPTY_CONFIG));
    let o = run(&["--ignore", "T041,", script.to_str().unwrap()]);
    assert_ne!(
        o.status.code(),
        Some(2),
        "trailing comma must be tolerated, not a validation error; stderr:\n{}",
        err_of(&o)
    );
    let s = out_of(&o);
    assert!(!s.contains("T041"), "T041 should still be ignored:\n{s}");
    assert!(s.contains("T002"), "T002 should remain:\n{s}");
}

#[test]
fn genuinely_unknown_filter_code_still_errors() {
    // A real (non-empty) unknown code must STILL exit 2 — only empty tokens are
    // tolerated.
    let o = run(&["--ignore", "T999"]);
    assert_eq!(
        o.status.code(),
        Some(2),
        "an unknown non-empty code must still exit 2; stderr:\n{}",
        err_of(&o)
    );
}

#[test]
fn version_flag_prints_version() {
    // #112: report the version like the other CLIs.
    let o = run(&["--version"]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", err_of(&o));
    assert!(
        out_of(&o).contains(env!("CARGO_PKG_VERSION")),
        "stdout: {}",
        out_of(&o)
    );
}

#[test]
fn project_level_error_sets_exit_code() {
    // #170: T093/T094 are Errors (M1 Build 1627/1631 fail the build), so a
    // project-level error must make the run exit non-zero — not just print.
    // `Root.Foo.Gain.Value` is a parameter no script reads → error[T094].
    let script = setup("exit-code-project-error", Some(CONFIG));
    let dir = script.parent().unwrap();
    let o = run(&[
        "--project",
        dir.join("Project.m1prj").to_str().unwrap(),
        "--select",
        "T094",
        script.to_str().unwrap(),
    ]);
    let s = out_of(&o);
    assert!(s.contains("error[T094]"), "got:\n{s}");
    assert_eq!(o.status.code(), Some(1), "stdout:\n{s}");
}

// #199: a warning-only run exits 0 by default; --strict escalates any finding to
// a failure; --no-warnings drops warnings from output and the exit status. We
// isolate a warning-only run with `--select T092` (the tag audit, Warning), so
// the script's own errors (T002/T094) don't muddy the exit code.
fn warning_only_run(name: &str, extra: &[&str]) -> Output {
    let script = setup(name, None);
    let prj = script.parent().unwrap().join("Project.m1prj");
    let mut args = vec![
        "--project".to_string(),
        prj.to_str().unwrap().to_string(),
        "--select".to_string(),
        "T092".to_string(),
    ];
    for a in extra {
        args.push(a.to_string());
    }
    args.push(script.to_str().unwrap().to_string());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&refs)
}

#[test]
fn warnings_alone_exit_zero_by_default() {
    let o = warning_only_run("strict_default", &[]);
    assert!(out_of(&o).contains("warning[T092]"), "got:\n{}", out_of(&o));
    assert_eq!(
        o.status.code(),
        Some(0),
        "warnings alone must not fail by default"
    );
}

#[test]
fn strict_fails_on_a_warning_only_run() {
    let o = warning_only_run("strict_on", &["--strict"]);
    assert!(
        out_of(&o).contains("warning[T092]"),
        "the warning is still reported under --strict:\n{}",
        out_of(&o)
    );
    assert_eq!(
        o.status.code(),
        Some(1),
        "--strict must fail on any finding"
    );
}

#[test]
fn no_warnings_suppresses_output_and_does_not_fail() {
    let o = warning_only_run("nowarn", &["--no-warnings"]);
    assert!(
        !out_of(&o).contains("T092"),
        "--no-warnings must suppress the warning:\n{}",
        out_of(&o)
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "suppressed warnings must not fail"
    );
}

#[test]
fn no_warnings_drops_warnings_before_strict_counts_them() {
    // Both flags together: the warning is dropped, so --strict sees no finding.
    let o = warning_only_run("nowarn_strict", &["--no-warnings", "--strict"]);
    assert!(!out_of(&o).contains("T092"), "got:\n{}", out_of(&o));
    assert_eq!(
        o.status.code(),
        Some(0),
        "a dropped warning is not a finding for --strict"
    );
}

// ---- T095 invalid-display-unit (default-on, M1 Build Error 1017) ------------

// A project with a channel whose display unit is from the wrong physical
// dimension: `rad/s` (Angular Speed) shown as `%` (a unit of ratio).
const PROJECT_T095: &str = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="T095Demo" TargetHardware="ecu120">
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Ctrl"/>
    <Component Classname="BuiltIn.Channel" Name="Root.Ctrl.Speed">
     <Props Qty="rad/s"><Locale><Default Unit="%"/></Locale></Props>
    </Component>
    <Component Classname="BuiltIn.FuncUser" Filename="Ctrl.Update.m1scr" Name="Root.Ctrl.Update"/>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

fn setup_t095(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Project.m1prj"), PROJECT_T095).unwrap();
    let script = dir.join("Ctrl.Update.m1scr");
    fs::write(&script, "").unwrap();
    script
}

#[test]
fn t095_invalid_display_unit_fires_and_is_error() {
    // T095 is default-on (M1 Build Error 1017 parity): a channel whose display
    // unit belongs to a different physical dimension than its Qty must be
    // flagged as an error (exit non-zero) even without --select.
    let script = setup_t095("t095_default_on");
    let prj = script.parent().unwrap().join("Project.m1prj");
    let o = run(&["--project", prj.to_str().unwrap(), script.to_str().unwrap()]);
    let s = out_of(&o);
    assert!(s.contains("T095"), "expected T095 in output:\n{s}");
    assert_eq!(
        o.status.code(),
        Some(1),
        "T095 is Error severity — must exit 1:\n{s}"
    );
}

#[test]
fn t095_ignore_suppresses_the_code() {
    // Isolate T095 with --select T095 first to confirm it fires, then verify
    // --ignore T095 removes it (the select/ignore pair is the standard pattern).
    let script = setup_t095("t095_ignore");
    let prj = script.parent().unwrap().join("Project.m1prj");
    // With --select T095 it appears.
    let on = run(&[
        "--project",
        prj.to_str().unwrap(),
        "--select",
        "T095",
        script.to_str().unwrap(),
    ]);
    let s_on = out_of(&on);
    assert!(
        s_on.contains("T095"),
        "expected T095 with --select:\n{s_on}"
    );
    // With --ignore T095 it disappears (use --select T095 to avoid noise from
    // other default-on codes that would also fail the run).
    let off = run(&[
        "--project",
        prj.to_str().unwrap(),
        "--select",
        "T095",
        "--ignore",
        "T095",
        script.to_str().unwrap(),
    ]);
    let s_off = out_of(&off);
    assert!(
        !s_off.contains("T095"),
        "--ignore T095 must suppress it:\n{s_off}"
    );
    assert_eq!(
        off.status.code(),
        Some(0),
        "no remaining findings must exit 0:\n{s_off}"
    );
}

// ---- T096 multiple-scheduled-writers (default-on, M1 Build Error 1022) ------

// A project with two periodically-scheduled functions and a channel that both
// write to — exactly the scenario that triggers M1 Build Error 1022.
const PROJECT_T096: &str = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="T096Demo" TargetHardware="ecu120">
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Events"/>
    <Component Classname="BuiltIn.EventKernel" Name="Root.Events.On 100Hz"/>
    <Component Classname="BuiltIn.EventKernel" Name="Root.Events.On 50Hz"/>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.Ctrl"/>
    <Component Classname="BuiltIn.Channel" Name="Root.Ctrl.Shared"><Props Type="f32"/></Component>
    <Component Classname="BuiltIn.FuncUser" Filename="Alpha.Update.m1scr" Name="Root.Ctrl.Alpha">
     <Props SelectedTrigger="Root.Events.On 100Hz"/>
    </Component>
    <Component Classname="BuiltIn.FuncUser" Filename="Beta.Update.m1scr" Name="Root.Ctrl.Beta">
     <Props SelectedTrigger="Root.Events.On 50Hz"/>
    </Component>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

fn setup_t096(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Project.m1prj"), PROJECT_T096).unwrap();
    // Both scripts write to Root.Ctrl.Shared — the multi-writer scenario.
    fs::write(dir.join("Alpha.Update.m1scr"), "Ctrl.Shared = 1.0;\n").unwrap();
    fs::write(dir.join("Beta.Update.m1scr"), "Ctrl.Shared = 2.0;\n").unwrap();
    dir.join("Alpha.Update.m1scr")
}

#[test]
fn t096_multiple_scheduled_writers_fires_and_is_error() {
    // T096 is default-on (M1 Build Error 1022 parity): a channel assigned by
    // more than one periodically-scheduled function must be flagged as an error
    // (exit non-zero) even without --select.
    let script = setup_t096("t096_default_on");
    let prj = script.parent().unwrap().join("Project.m1prj");
    let o = run(&["--project", prj.to_str().unwrap(), script.to_str().unwrap()]);
    let s = out_of(&o);
    assert!(s.contains("T096"), "expected T096 in output:\n{s}");
    assert_eq!(
        o.status.code(),
        Some(1),
        "T096 is Error severity — must exit 1:\n{s}"
    );
}

#[test]
fn t096_ignore_suppresses_the_code() {
    let script = setup_t096("t096_ignore");
    let prj = script.parent().unwrap().join("Project.m1prj");
    let o = run(&[
        "--project",
        prj.to_str().unwrap(),
        "--ignore",
        "T096",
        script.to_str().unwrap(),
    ]);
    let s = out_of(&o);
    assert!(!s.contains("T096"), "--ignore T096 must suppress it:\n{s}");
    assert_eq!(
        o.status.code(),
        Some(0),
        "suppressed T096 must not fail:\n{s}"
    );
}

// ---- T050 --audit-names disposition under --strict / --no-warnings ----------

// A project with a lowercase group leaf (`Root.foo`), so the naming audit (T050,
// Warning) fires under --audit-names. Names beginning lowercase violate the
// manual's "Naming Objects" convention.
const PROJECT_T050: &str = r#"<?xml version="1.0"?>
<MoTeCM1BuildSession>
 <Project Name="T050Demo" TargetHardware="ecu120">
  <ComponentStream>
   <List>
    <Component Classname="BuiltIn.GroupCompound" Name="Root.foo"/>
    <Component Classname="BuiltIn.FuncUser" Filename="foo.Update.m1scr" Name="Root.foo.Update"/>
   </List>
  </ComponentStream>
 </Project>
</MoTeCM1BuildSession>
"#;

fn setup_t050(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Project.m1prj"), PROJECT_T050).unwrap();
    let script = dir.join("foo.Update.m1scr");
    fs::write(&script, "").unwrap();
    script
}

fn run_t050(name: &str, extra: &[&str]) -> Output {
    let script = setup_t050(name);
    let prj = script.parent().unwrap().join("Project.m1prj");
    let mut args = vec![
        "--project".to_string(),
        prj.to_str().unwrap().to_string(),
        "--audit-names".to_string(),
        "--select".to_string(),
        "T050".to_string(),
    ];
    for a in extra {
        args.push(a.to_string());
    }
    args.push(script.to_str().unwrap().to_string());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&refs)
}

#[test]
fn audit_names_t050_warning_fires_by_default() {
    // Baseline: --audit-names surfaces the lowercase-name T050 warning, and a
    // warning alone exits 0 (no --strict).
    let o = run_t050("t050_default", &[]);
    let s = out_of(&o);
    assert!(s.contains("warning[T050]"), "expected T050 warning:\n{s}");
    assert_eq!(
        o.status.code(),
        Some(0),
        "a warning alone must not fail by default:\n{s}"
    );
}

#[test]
fn audit_names_t050_warning_fails_under_strict() {
    // #199 disposition parity: every other project-level diagnostic fails the run
    // under --strict; the --audit-names T050 path must too.
    let o = run_t050("t050_strict", &["--strict"]);
    let s = out_of(&o);
    assert!(
        s.contains("warning[T050]"),
        "the warning is still reported under --strict:\n{s}"
    );
    assert_eq!(
        o.status.code(),
        Some(1),
        "--strict must fail on a T050 warning, like every other project diagnostic:\n{s}"
    );
}

#[test]
fn audit_names_t050_warning_dropped_under_no_warnings() {
    // #199 disposition parity: --no-warnings drops the T050 warning from output
    // and the exit status, like every other project-level diagnostic.
    let o = run_t050("t050_nowarn", &["--no-warnings"]);
    let s = out_of(&o);
    assert!(
        !s.contains("T050"),
        "--no-warnings must suppress the T050 warning:\n{s}"
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "a dropped warning must not fail the run:\n{s}"
    );
}
