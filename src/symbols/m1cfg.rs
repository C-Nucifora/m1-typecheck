//! Parse parameters.m1cfg and augment matching symbols with value type + unit.
use super::SymbolTable;
use crate::types::ValueType;

#[derive(Debug)]
pub enum ConfigError {
    Xml(roxmltree::Error),
}
impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Xml(e) => write!(f, "invalid .m1cfg XML: {e}"),
        }
    }
}
impl std::error::Error for ConfigError {}

/// Map a `.m1cfg` cell primitive type to a [`ValueType`]. `enum` cells are
/// handled separately in [`augment`] (back-resolved to a specific enum type);
/// any non-primitive name falls through to `Unknown`.
pub fn cell_type(t: &str) -> ValueType {
    crate::types::primitive_type(t).unwrap_or(ValueType::Unknown)
}

pub fn augment(table: &mut SymbolTable, xml: &str) -> Result<(), ConfigError> {
    let doc = roxmltree::Document::parse(xml).map_err(ConfigError::Xml)?;
    for param in doc.descendants().filter(|n| n.has_tag_name("Parameter")) {
        let Some(name) = param.attribute("Name") else {
            continue;
        };
        let Some(cell) = param.children().find(|c| c.has_tag_name("Cell")) else {
            continue;
        };
        // Real MoTeC `.m1cfg` exports name parameters WITHOUT the implicit `Root.`
        // group prefix the symbol table keys use (cfg `Foo.Bar` vs symbol
        // `Root.Foo.Bar`). Resolve to the actual symbol path so the cfg's types and
        // units are applied; without this every entry of a real export misses and
        // the whole cfg is silently dropped. An already-qualified name still matches.
        let Some(path) = resolve_path(table, name) else {
            continue;
        };
        let type_attr = cell.attribute("Type").unwrap_or("");
        let unit = cell.attribute("Unit").map(str::to_string);

        if type_attr == "enum" {
            // Back-resolve the cell's member value to a *unique* enum type.
            let member = cell.text().map(str::trim).unwrap_or("");
            let candidates = table.enums_with_member(member);
            if candidates.len() == 1 {
                let id = candidates[0];
                table.set_enum_assoc(&path, id);
            }
            // zero or many -> leave Unknown (silent), but still record the unit.
            if let Some(sym) = table.get_mut(&path)
                && sym.unit.is_none()
            {
                sym.unit = unit;
            }
            continue;
        }

        let vt = cell_type(type_attr);
        if let Some(sym) = table.get_mut(&path) {
            sym.value_type = vt;
            // Only override the unit when the cfg cell actually carries one, so a
            // unit already populated from the `.m1prj` `<Default Unit>` (#75) is
            // not wiped to `None` for a parameter the cfg lists without a unit.
            if unit.is_some() {
                sym.unit = unit;
            }
        }
    }
    Ok(())
}

/// Resolve a `.m1cfg` parameter name to an actual symbol path: the name verbatim,
/// else with the implicit `Root.` group prefix that real exports omit.
fn resolve_path(table: &SymbolTable, name: &str) -> Option<String> {
    if table.get(name).is_some() {
        return Some(name.to_string());
    }
    let rooted = format!("Root.{name}");
    table.get(&rooted).is_some().then_some(rooted)
}
