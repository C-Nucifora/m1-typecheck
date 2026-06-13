//! The loaded project: symbol table + enum types + opaque roots + file->group.
use crate::diagnostics::{TypeCode, TypeDiagnostic, make_project_for};
use crate::resolve::Scope;
use crate::symbols::{Symbol, SymbolKind, SymbolTable, m1cfg, m1dbc, m1prj};
use crate::typer::{path_text, type_of};
use crate::types::ValueType;
use m1_core::{Field, Kind, Node, Severity};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// M1 standard-library roots always treated as opaque (never flagged).
const STATIC_OPAQUE_ROOTS: &[&str] = &[
    "This",
    "Calculate",
    "Limit",
    "Convert",
    "Math",
    "Time",
    "Filter",
    "CanComms",
    "Table",
    "Channel",
    "Constant",
    "Parameter",
    "Diagnostic",
];

pub struct Project {
    table: SymbolTable,
    opaque_roots: HashSet<String>,
    file_to_group: HashMap<String, String>,
    /// The parameter symbol paths a loaded `.m1cfg` declares (qualified to the
    /// symbol-table keys, e.g. `Root.Foo.Bar`). `None` when no cfg was loaded —
    /// the missing-parameter audit (T041) is then skipped.
    cfg_params: Option<HashSet<String>>,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(String),
}
impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "{e}"),
            LoadError::Parse(s) => write!(f, "{s}"),
        }
    }
}
impl std::error::Error for LoadError {}

impl Project {
    pub fn load(m1prj_path: &Path) -> Result<Project, LoadError> {
        let xml = m1_workspace::read_motec_xml(m1prj_path).map_err(LoadError::Io)?;
        Self::from_xml(&xml)
    }

    /// Build a project from `.m1prj` XML already in memory, without reading from
    /// disk. Used to rebuild the model from edited-but-unsaved project text (e.g.
    /// the LSP refreshing its symbol table immediately after a rename, before the
    /// client has written the file back). `.m1cfg`/`.m1dbc` augmentation, which is
    /// disk-sourced and unchanged by such edits, is applied separately via
    /// [`Project::with_config`] / [`Project::with_dbc`].
    pub fn from_xml(xml: &str) -> Result<Project, LoadError> {
        let parsed = m1prj::parse(xml).map_err(|e| LoadError::Parse(e.to_string()))?;
        let mut opaque_roots: HashSet<String> =
            STATIC_OPAQUE_ROOTS.iter().map(|s| s.to_string()).collect();
        opaque_roots.extend(parsed.module_roots);
        Ok(Project {
            table: parsed.table,
            opaque_roots,
            file_to_group: parsed.file_to_group,
            cfg_params: None,
        })
    }

    pub fn with_config(mut self, m1cfg_path: &Path) -> Result<Project, LoadError> {
        let xml = m1_workspace::read_motec_xml(m1cfg_path).map_err(LoadError::Io)?;
        m1cfg::augment(&mut self.table, &xml).map_err(|e| LoadError::Parse(e.to_string()))?;
        // Record which parameters the cfg declares (qualified to symbol keys) so
        // `missing_cfg_parameters` can flag project parameters absent from it.
        let names = m1cfg::parameter_names(&xml).map_err(|e| LoadError::Parse(e.to_string()))?;
        self.cfg_params = Some(
            names
                .iter()
                .map(|n| m1_workspace::qualify_root(n).into_owned())
                .collect(),
        );
        Ok(self)
    }

    /// Project-level audit (T041): every `BuiltIn.Parameter` declared in the
    /// `.m1prj` that has no entry in the loaded `.m1cfg` — M1-Build will fall back
    /// to its default value. Empty when no cfg is loaded (nothing to compare).
    ///
    /// Hint severity (#156): riding the default is *normal, supported* M1
    /// behaviour — most parameters in a real project are intentionally
    /// uncalibrated (EV-M1: 305, AV-M1: 173), so a Warning per parameter buries
    /// every actionable diagnostic. Teams that require full calibration
    /// coverage can still gate on it via `--select T041` + fail-on-warning
    /// policy of their choice.
    pub fn missing_cfg_parameters(&self) -> Vec<TypeDiagnostic> {
        let Some(cfg_params) = &self.cfg_params else {
            return Vec::new();
        };
        let mut out: Vec<TypeDiagnostic> = self
            .table
            .iter()
            .filter(|s| s.kind == SymbolKind::Parameter && !cfg_params.contains(&s.path))
            .map(|s| {
                make_project_for(
                    TypeCode::T041,
                    Severity::Hint,
                    format!(
                        "parameter `{}` is declared in the project but has no entry in the .m1cfg (will use its default value)",
                        s.path
                    ),
                    &s.path,
                )
            })
            .collect();
        // Deterministic order for stable CI output.
        out.sort_by(|a, b| a.inner.message.cmp(&b.inner.message));
        out
    }

    /// Merge a `.m1dbc`'s CAN objects (DBC/Message/Signal) into the symbol
    /// table. `rel_filename` is the `.m1dbc` path relative to the project root
    /// (stored on each symbol for goto). May be called once per DBC file.
    pub fn with_dbc(mut self, dbc_path: &Path, rel_filename: &str) -> Result<Project, LoadError> {
        self.augment_dbc(dbc_path, rel_filename)?;
        Ok(self)
    }

    /// Merge a `.m1dbc` in place. Same as [`Project::with_dbc`] but borrows
    /// instead of consuming, so a caller can keep the partially-built project if
    /// one DBC fails — a single malformed CAN file should degrade gracefully
    /// (skip that DBC), not blank the whole model.
    pub fn augment_dbc(&mut self, dbc_path: &Path, rel_filename: &str) -> Result<(), LoadError> {
        let xml = m1_workspace::read_motec_xml(dbc_path).map_err(LoadError::Io)?;
        m1dbc::augment(&mut self.table, &xml, rel_filename)
            .map_err(|e| LoadError::Parse(e.to_string()))?;
        Ok(())
    }

    pub fn symbols(&self) -> &SymbolTable {
        &self.table
    }

    pub fn is_opaque_root(&self, root: &str) -> bool {
        self.opaque_roots.contains(root)
    }

    /// Enclosing group for a script file name (the bare `Filename`, e.g.
    /// "Foo Update.m1scr"); None if the project has no such function/method.
    pub fn group_for_script(&self, file_name: &str) -> Option<String> {
        self.file_to_group.get(file_name).cloned()
    }

    /// Audit the project's own symbol names (T050).
    pub fn audit(&self) -> Vec<crate::diagnostics::TypeDiagnostic> {
        crate::audit::audit_project(self)
    }

    /// Tags audit (T092, default-on): M1 Build tag-warning parity — see
    /// [`crate::audit::audit_tags`].
    pub fn audit_tags(&self) -> Vec<crate::diagnostics::TypeDiagnostic> {
        crate::audit::audit_tags(self)
    }

    /// Display-unit audit (T095, default-on): M1 Build Error 1017 parity — see
    /// [`crate::audit::audit_display_units`]. Exposed separately from
    /// [`Self::audit`] (the `--audit-names` convention audit) because it is an
    /// M1-Build-parity check that should run on every default run, like
    /// [`Self::audit_tags`].
    pub fn audit_display_units(&self) -> Vec<crate::diagnostics::TypeDiagnostic> {
        crate::audit::audit_display_units(self)
    }

    /// Infer user-function/method return types from their script bodies (#110).
    ///
    /// `scripts` carries each script's file name (e.g. `"Engine.Update.m1scr"`)
    /// with its already-parsed CST (parsed once per run — #192). For every script
    /// that backs a `Function`/`Method` symbol, type each `Out = <expr>`
    /// right-hand side (the manual's return-value object); when they all agree on
    /// one *known* type, store it as that symbol's `return_type`. No `Out`,
    /// disagreeing types, a syntax error, or an Unknown result leaves the type
    /// `None` — never a guess. Call this after the project (and any
    /// `.m1cfg`/`.m1dbc`) is loaded and *before* checking callers, so user-function
    /// call sites resolve to a concrete type in `type_of_call`.
    ///
    /// Two-phase to avoid a borrow conflict: phase 1 types the bodies against
    /// `&self` (the in-progress table, where not-yet-inferred callees stay
    /// Unknown — conservative); phase 2 writes the results back via `&mut self`.
    pub fn infer_return_types(&mut self, scripts: &[crate::parsed::ParsedScript]) {
        let mut inferred: Vec<(String, ValueType)> = Vec::new();
        for script in scripts {
            let file_name = &script.name;
            let Some(sym_path) = self.function_symbol_for_script(file_name) else {
                continue;
            };
            // A `<Signature ReturnType>` declared in the .m1prj is
            // authoritative (#110); only infer when nothing is declared.
            if self
                .table
                .get(&sym_path)
                .is_some_and(|s| s.return_type.is_some())
            {
                continue;
            }
            let cst = &script.cst;
            if !cst.syntax_diagnostics().is_empty() {
                continue;
            }
            // Parity with `rules::run_with`'s depth guard (#94): the inference
            // passes below (`collect_locals`, `out_return_type`/`collect_out_types`,
            // and the `type_of` they call) recurse over the CST, so a
            // pathologically deep — yet syntactically valid — body would overflow
            // the stack here. `max_depth()` is computed iteratively, so probe it
            // and skip return-type inference for an over-deep file rather than
            // crash. (`collect_out_types` is additionally iterative as a belt-and-
            // braces measure.)
            if cst.root().max_depth() > m1_core::MAX_RECURSION_DEPTH {
                continue;
            }
            let group = self.group_for_script(file_name);
            let scope = Scope {
                locals: crate::rules::collect_locals(cst.root(), Some(self), group.as_deref()),
                group: group.clone(),
                project: Some(self),
                fn_symbol: Some(sym_path.clone()),
            };
            if let Some(ty) = out_return_type(cst.root(), &scope) {
                inferred.push((sym_path, ty));
            }
        }
        for (path, ty) in inferred {
            self.table.set_return_type(&path, ty);
        }
    }

    /// The path of the `Function`/`Method` symbol backed by `file_name`: an
    /// explicit `Filename=` match first, else the `Root.<stem>` path convention
    /// real projects use (`Engine.Update.m1scr` → `Root.Engine.Update`).
    pub fn function_symbol_for_script(&self, file_name: &str) -> Option<String> {
        let is_fn = |s: &&Symbol| matches!(s.kind, SymbolKind::Function | SymbolKind::Method);
        if let Some(path) = self.table.function_path_for_filename(file_name) {
            return Some(path.to_string());
        }
        let stem = file_name.strip_suffix(".m1scr")?;
        let path = format!("Root.{stem}");
        self.table.get(&path).filter(is_fn).map(|s| s.path.clone())
    }
}

/// The concrete type all `Out = <expr>` assignments in a function body agree on,
/// or `None` when there are none, they disagree, or any is Unknown (#110).
fn out_return_type(root: Node, scope: &Scope) -> Option<ValueType> {
    let mut types = Vec::new();
    collect_out_types(root, scope, &mut types);
    let first = *types.first()?;
    (first.is_known() && types.iter().all(|t| *t == first)).then_some(first)
}

/// Push the type of each `Out = <expr>` right-hand side in `node` onto `out`.
///
/// Walks the subtree with the **iterative** `descendants()` traversal (which
/// includes `node` itself) rather than recursing per child: a pathologically
/// deep function body — e.g. tens of thousands of nested parens in an `Out =`
/// expression — would otherwise overflow the stack here, since this runs during
/// `infer_return_types` *before* `run_with`'s `max_depth` guard ever sees the
/// file. The iterative walk has no such depth limit (#94, parity with the main
/// `rules::walk` protection).
fn collect_out_types(node: Node, scope: &Scope, out: &mut Vec<ValueType>) {
    for n in node.descendants() {
        if n.kind() == Kind::AssignmentStatement
            && let Some(target) = n.child_by_field(Field::Target)
            && path_text(target) == "Out"
            && let Some(value) = n.child_by_field(Field::Value)
        {
            out.push(type_of(value, scope));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::TypeCode;

    fn write_temp(name: &str, content: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("m1tc_{}_{}", std::process::id(), name));
        std::fs::write(&p, content).unwrap();
        p
    }

    // #110: a user function's return type is inferred from its `Out = <expr>`
    // body and stored on the symbol, so callers can be type-checked.
    #[test]
    fn infers_user_function_return_type_from_out_assignment() {
        use crate::types::ValueType;
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Demo"/>
  <Component Classname="BuiltIn.FuncUser" Name="Root.Demo.Compute"/>
</Project>"#;
        let mut project = Project::from_xml(xml).unwrap();
        project.infer_return_types(&crate::parsed::parse_all(&[(
            "Demo.Compute.m1scr".to_string(),
            "Out = 1.5;\n".to_string(),
        )]));
        assert_eq!(
            project
                .symbols()
                .get("Root.Demo.Compute")
                .unwrap()
                .return_type,
            Some(ValueType::Float)
        );
    }

    // #110: once a function's Float return is modelled, assigning its call result
    // into an unsigned-integer channel is a T030 mismatch (the issue's repro).
    #[test]
    fn float_returning_call_into_integer_channel_flags_t030() {
        use crate::rules::check_script;
        use std::path::Path;
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.GroupCompound" Name="Root.Demo"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Demo.Widget Count"><Props Type="u32"/></Component>
  <Component Classname="BuiltIn.FuncUser" Name="Root.Demo.Compute"/>
</Project>"#;
        let mut project = Project::from_xml(xml).unwrap();
        project.infer_return_types(&crate::parsed::parse_all(&[(
            "Demo.Compute.m1scr".to_string(),
            "Out = 1.5;\n".to_string(),
        )]));
        let caller = "local r = Demo.Compute();\nDemo.Widget Count = r;\n";
        let result = check_script(&project, Path::new("Demo.Caller.m1scr"), caller);
        assert!(
            result.diagnostics.iter().any(|d| d.code == TypeCode::T030),
            "expected T030, got {:?}",
            result.diagnostics
        );
    }

    // #110 (no-guess): a function with no `Out =` assignment leaves return_type
    // None, and disagreeing `Out =` types also stay None (never a guessed type).
    #[test]
    fn return_type_none_without_consistent_out() {
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.FuncUser" Name="Root.NoOut"/>
  <Component Classname="BuiltIn.FuncUser" Name="Root.Mixed"/>
</Project>"#;
        let mut project = Project::from_xml(xml).unwrap();
        project.infer_return_types(&crate::parsed::parse_all(&[
            ("NoOut.m1scr".to_string(), "local x = 1;\n".to_string()),
            (
                "Mixed.m1scr".to_string(),
                "Out = 1.5;\nOut = 1;\n".to_string(),
            ),
        ]));
        assert_eq!(
            project.symbols().get("Root.NoOut").unwrap().return_type,
            None
        );
        assert_eq!(
            project.symbols().get("Root.Mixed").unwrap().return_type,
            None
        );
    }

    // #94: a pathologically deep function body must not stack-overflow during
    // return-type inference. `collect_out_types` runs in `infer_return_types`,
    // which is reached *before* `run_with`'s `max_depth` guard, so it carries its
    // own protection (an iterative `descendants()` walk). The nesting here is
    // structural (thousands of nested `if` blocks) so the deep walk is exercised
    // while the `Out = 1.5` value itself stays shallow.
    #[test]
    fn deeply_nested_body_does_not_overflow_collecting_out_types() {
        const DEPTH: usize = 20_000;
        let mut body = String::new();
        for _ in 0..DEPTH {
            body.push_str("if (1)\n{\n");
        }
        body.push_str("Out = 1.5;\n");
        for _ in 0..DEPTH {
            body.push_str("}\n");
        }
        let xml = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.FuncUser" Name="Root.Deep"/>
</Project>"#;
        let mut project = Project::from_xml(xml).unwrap();
        // Must return (not SIGABRT). If m1-core rejects the depth as a syntax
        // error, inference simply skips the file; either way it does not crash.
        project.infer_return_types(&crate::parsed::parse_all(&[(
            "Deep.m1scr".to_string(),
            body,
        )]));
    }

    #[test]
    fn from_xml_builds_a_project_from_in_memory_text() {
        // The LSP rebuilds the project model from the post-rename `.m1prj` text
        // (which the client hasn't written to disk yet), so a string constructor
        // must produce the same symbol table `load` would.
        let xml = "<?xml version=\"1.0\"?>\n<Project>\n\
             <Component Classname=\"BuiltIn.GroupCompound\" Name=\"Root.Engine\"/>\n\
             <Component Classname=\"BuiltIn.Channel\" Name=\"Root.Engine.Speed\"><Props Type=\"f32\"/></Component>\n\
             </Project>";
        let project = Project::from_xml(xml).unwrap();
        assert!(project.symbols().get("Root.Engine.Speed").is_some());
        // Static opaque roots are still applied (parity with `load`).
        assert!(project.is_opaque_root("Calculate"));
    }

    #[test]
    fn missing_cfg_parameters_flags_only_uncovered_params() {
        let prj = write_temp(
            "Project.m1prj",
            "<?xml version=\"1.0\"?>\n<Project>\n\
             <Component Classname=\"BuiltIn.Parameter\" Name=\"Root.A.Covered\"><Props Type=\"u32\"/></Component>\n\
             <Component Classname=\"BuiltIn.Parameter\" Name=\"Root.A.Missing\"><Props Type=\"u32\"/></Component>\n\
             <Component Classname=\"BuiltIn.Channel\" Name=\"Root.A.Chan\"><Props Type=\"u32\"/></Component>\n\
             </Project>",
        );
        // A real `.m1cfg` lists parameters with the `Root.` prefix stripped.
        let cfg = write_temp(
            "parameters.m1cfg",
            "<?xml version=\"1.0\"?>\n<Configuration>\n <Group Name=\"\">\n\
             <Parameter Name=\"A.Covered\"><Cell Type=\"u32\"><![CDATA[1]]></Cell></Parameter>\n\
             </Group>\n</Configuration>",
        );
        let project = Project::load(&prj).unwrap().with_config(&cfg).unwrap();
        let diags = project.missing_cfg_parameters();
        // Only `Root.A.Missing` is flagged: `Covered` is in the cfg, `Chan` is a
        // channel (not a parameter).
        assert_eq!(diags.len(), 1, "exactly one missing parameter");
        assert_eq!(diags[0].code, TypeCode::T041);
        assert!(diags[0].inner.message.contains("Root.A.Missing"));
        // #156: Hint, not Warning — riding the default is normal M1 behaviour,
        // and at hundreds per healthy project a Warning buries real findings.
        assert_eq!(diags[0].inner.severity, Severity::Hint);
        let _ = std::fs::remove_file(&prj);
        let _ = std::fs::remove_file(&cfg);
    }

    #[test]
    fn no_cfg_means_no_missing_param_diagnostics() {
        let prj = write_temp(
            "Project_nocfg.m1prj",
            "<?xml version=\"1.0\"?>\n<Project>\n\
             <Component Classname=\"BuiltIn.Parameter\" Name=\"Root.A.X\"><Props Type=\"u32\"/></Component>\n\
             </Project>",
        );
        // Without a loaded cfg there is nothing to compare against.
        let project = Project::load(&prj).unwrap();
        assert!(project.missing_cfg_parameters().is_empty());
        let _ = std::fs::remove_file(&prj);
    }

    fn write_temp_bytes(name: &str, content: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("m1tc_{}_{}", std::process::id(), name));
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn with_dbc_decodes_windows_1252_units() {
        // MoTeC writes `.m1dbc` as Windows-1252: a yaw-rate unit `°/s` stores the
        // degree sign as the single byte 0xB0, which is invalid UTF-8. The model
        // must still load (and keep the unit), not fail the whole build — this is
        // the AV-M1 `Datalogger.m1dbc` regression.
        let mut dbc = b"<?xml version=\"1.0\"?>\n<DBC>\n <ComponentStream>\n  <List>\n   \
            <Component Classname=\"BuiltIn.CAN.DBC\" Name=\"Bus\"/>\n   \
            <Component Classname=\"BuiltIn.CAN.Message\" Name=\"Bus.Imu\"><Props CANId=\"100\" DLC=\"8\"/></Component>\n   \
            <Component Classname=\"BuiltIn.CAN.Signal\" Name=\"Bus.Imu.Yaw Rate\"><Props Type=\"s32\" Qty=\""
            .to_vec();
        dbc.push(0xB0); // ° in Windows-1252
        dbc.extend_from_slice(
            b"/s\" StartBit=\"0\" Length=\"16\"/></Component>\n  </List>\n </ComponentStream>\n</DBC>",
        );
        let dbc_path = write_temp_bytes("Bus.m1dbc", &dbc);
        let prj = write_temp("dbc_Project.m1prj", "<?xml version=\"1.0\"?>\n<Project/>");

        let project = Project::load(&prj)
            .unwrap()
            .with_dbc(&dbc_path, "dbc/Bus.m1dbc")
            .expect("a Windows-1252 DBC must load, not abort the model build");

        let sig = project
            .symbols()
            .get("Bus.Imu.Yaw Rate")
            .expect("signal resolves despite the 1252 byte");
        assert_eq!(sig.unit.as_deref(), Some("°/s"));
        let _ = std::fs::remove_file(&dbc_path);
        let _ = std::fs::remove_file(&prj);
    }
}
