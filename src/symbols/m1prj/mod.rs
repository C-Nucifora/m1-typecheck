//! Parse Project.m1prj into a SymbolTable, enum types, and a file->group map.
//!
//! Split across submodules:
//!   * `parse` — the XML walk that builds the `SymbolTable` and maps.
//!   * `type_inference` — value-type inference for channels with no explicit
//!     `<Props Type>`/`Qty` (owner-class + leaf-name heuristics).
//!
//! The public surface (`parse`, `classify`, `Parsed`) is re-exported here so
//! `crate::symbols::m1prj::{parse, classify, Parsed}` paths are unchanged.

mod parse;
mod type_inference;

pub use parse::parse;

use super::SymbolKind;
use std::collections::{HashMap, HashSet};

/// The result of parsing a `.m1prj`: the symbol table plus the script-file →
/// group map and the set of selected module roots.
pub struct Parsed {
    pub table: super::SymbolTable,
    pub file_to_group: HashMap<String, String>,
    pub module_roots: HashSet<String>,
}

impl SymbolKind {
    /// Classify a `.m1prj`/`.m1dbc` component `Classname` into a [`SymbolKind`].
    ///
    /// This is the canonical home of the classname → kind mapping. The free
    /// function [`classify`] is a thin backward-compatible delegate kept so
    /// existing call sites (and any external consumer) keep compiling.
    pub fn from_classname(classname: &str) -> SymbolKind {
        use SymbolKind::*;
        if classname.starts_with("BuiltIn.Channel") || classname == "BuiltIn.CalibrationChannel" {
            Channel
        } else if classname.starts_with("BuiltIn.Parameter")
            || classname == "BuiltIn.CalibrationParameter"
            || classname == "BuiltIn.IOResourceParameter"
        {
            Parameter
        } else if classname.starts_with("BuiltIn.Constant")
            || classname == "BuiltIn.IOResourceConstant"
        {
            Constant
        } else if classname.starts_with("BuiltIn.FuncUser")
            || classname.starts_with("BuiltIn.CalFuncUser")
            || classname.starts_with("BuiltIn.FuncGenerated")
        {
            Function
        } else if classname == "BuiltIn.MethodUser" || classname == "BuiltIn.EventKernel" {
            Method
        } else if classname.starts_with("BuiltIn.Table") || classname == "BuiltIn.CalibrationTable"
        {
            Table
        } else if classname == "BuiltIn.GroupCompound" {
            Group
        } else if classname == "BuiltIn.Reference" {
            Reference
        } else if !classname.starts_with("BuiltIn.") && !classname.is_empty() {
            // Any non-`BuiltIn.*` component is an instance of a package-defined
            // object class (e.g. "MoTeC Input.Sensor", "MoTeC Output.Switched
            // Output", "_IOMethod.abs"). Its members are separate components whose
            // path is prefixed by this object's path.
            Object
        } else {
            Other
        }
    }
}

/// Backward-compatible delegate to [`SymbolKind::from_classname`]. Retained so
/// existing `m1prj::classify(...)` call sites keep working.
pub fn classify(classname: &str) -> SymbolKind {
    SymbolKind::from_classname(classname)
}

/// Enclosing group path for a fully-qualified symbol path (drop the last segment,
/// honouring space-containing segments — segments are split on '.').
pub(super) fn parent_group(path: &str) -> Option<String> {
    let idx = path.rfind('.')?;
    Some(path[..idx].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_package_objects_and_keeps_builtins() {
        assert_eq!(classify("MoTeC Input.Sensor"), SymbolKind::Object);
        assert_eq!(classify("MoTeC Output.Switched Output"), SymbolKind::Object);
        assert_eq!(classify("_IOMethod.abs"), SymbolKind::Object);
        // BuiltIn.* primitives are unchanged.
        assert_eq!(classify("BuiltIn.Channel"), SymbolKind::Channel);
        assert_eq!(classify("BuiltIn.MethodUser"), SymbolKind::Method);
        // An unhandled BuiltIn.* stays Other, not Object.
        assert_eq!(classify("BuiltIn.IOCharacteristic"), SymbolKind::Other);
    }
}
