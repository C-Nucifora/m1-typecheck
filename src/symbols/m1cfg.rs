//! Parse parameters.m1cfg and augment matching symbols with value type + unit.
use super::{SymbolTable, TableAxis, TableMeta, XmlParseError};
use crate::types::ValueType;

/// Map a `.m1cfg` cell primitive type to a [`ValueType`]. `enum` cells are
/// handled separately in [`augment`] (back-resolved to a specific enum type);
/// any non-primitive name falls through to `Unknown`.
pub fn cell_type(t: &str) -> ValueType {
    crate::types::primitive_type(t).unwrap_or(ValueType::Unknown)
}

pub fn augment(table: &mut SymbolTable, xml: &str) -> Result<(), XmlParseError> {
    let doc = roxmltree::Document::parse(xml).map_err(XmlParseError::in_context(".m1cfg"))?;
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
            // The authoritative unit is the quantity's base unit set from the
            // `.m1prj` `<Props Qty>`. A cfg cell unit may be a display unit, so it
            // only *fills* a missing unit — it never overrides the base unit.
            if sym.unit.is_none() && unit.is_some() {
                sym.unit = unit;
            }
        }
    }

    // Tables: the `.m1cfg` `<Table>` is the authoritative source for both the
    // interpolated output type/unit (which the `.m1prj` leaves untyped, #25) and
    // the table's shape. A table named `Foo.Bar` drives the channel
    // `Root.Foo.Bar.Value`; the shape attaches to the `Root.Foo.Bar` table symbol.
    for tbl in doc.descendants().filter(|n| n.has_tag_name("Table")) {
        let Some(name) = tbl.attribute("Name") else {
            continue;
        };
        let (meta, output_vt) = parse_table(tbl);
        // Output channel: take the Body cell type (authoritative over the
        // owner-class Float fallback) and unit.
        if let Some(vpath) = resolve_path(table, &format!("{name}.Value"))
            && let Some(sym) = table.get_mut(&vpath)
        {
            if output_vt != ValueType::Unknown {
                sym.value_type = output_vt;
            }
            if meta.output_unit.is_some() {
                sym.unit = meta.output_unit.clone();
            }
        }
        // Table symbol: record the shape for hover.
        if let Some(tpath) = resolve_path(table, name)
            && let Some(sym) = table.get_mut(&tpath)
        {
            sym.table_meta = Some(meta);
        }
    }
    Ok(())
}

/// Parse a `.m1cfg` `<Table>` into its [`TableMeta`] (input-axis shape + output
/// unit) and the output value type. Input axes are the `<X>`/`<Y>`/`<Z>` children
/// in that order — each axis's breakpoint count is the number of `<Cell>`s in its
/// `<Cells>` — and the interpolated output is the `<Body><Cells>`.
fn parse_table(tbl: roxmltree::Node<'_, '_>) -> (TableMeta, ValueType) {
    let cells_of = |parent: roxmltree::Node<'_, '_>| {
        parent
            .children()
            .find(|c| c.has_tag_name("Cells"))
            .map(|cells| {
                let size = cells.children().filter(|c| c.has_tag_name("Cell")).count() as u32;
                let unit = cells
                    .attribute("Unit")
                    .filter(|u| !u.is_empty())
                    .map(str::to_string);
                let ty = cells.attribute("Type").unwrap_or("");
                (size, unit, ty.to_string())
            })
    };
    let mut axes = Vec::new();
    for tag in ["X", "Y", "Z"] {
        if let Some(axis) = tbl.children().find(|c| c.has_tag_name(tag))
            && let Some((size, unit, _)) = cells_of(axis)
        {
            axes.push(TableAxis { size, unit });
        }
    }
    let body = tbl.children().find(|c| c.has_tag_name("Body"));
    let (output_unit, output_vt) = match body.and_then(cells_of) {
        Some((_, unit, ty)) => (unit, cell_type(&ty)),
        None => (None, ValueType::Unknown),
    };
    (TableMeta { axes, output_unit }, output_vt)
}

/// The parameter names a `.m1cfg` declares, as written there (unprefixed, e.g.
/// `Foo.Bar`). Used to flag project parameters that have no cfg entry (T041).
pub fn parameter_names(xml: &str) -> Result<Vec<String>, XmlParseError> {
    let doc = roxmltree::Document::parse(xml).map_err(XmlParseError::in_context(".m1cfg"))?;
    Ok(doc
        .descendants()
        .filter(|n| n.has_tag_name("Parameter"))
        .filter_map(|n| n.attribute("Name"))
        .map(str::to_string)
        .collect())
}

/// Resolve a `.m1cfg` parameter name to an actual symbol path: the name verbatim,
/// else with the implicit `Root.` group prefix that real exports omit.
fn resolve_path(table: &SymbolTable, name: &str) -> Option<String> {
    if table.get(name).is_some() {
        return Some(name.to_string());
    }
    let rooted = m1_workspace::qualify_root(name);
    table
        .get(rooted.as_ref())
        .is_some()
        .then(|| rooted.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::m1prj;

    #[test]
    fn table_output_and_shape_from_cfg() {
        // The cfg is authoritative for a table: its `.Value` output channel takes
        // the Body cell type + unit, and the table symbol takes the input-axis
        // shape (#25 + table shape).
        let prj = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Table" Name="Root.Control.Limiting.Torque"/>
  <Component Classname="BuiltIn.Channel" Name="Root.Control.Limiting.Torque.Value" Caps="AutoCreated"/>
</Project>"#;
        let mut table = m1prj::parse(prj).unwrap().table;
        // Owner-class fallback gives Float but no unit before the cfg is applied.
        let before = table.get("Root.Control.Limiting.Torque.Value").unwrap();
        assert_eq!(before.value_type, ValueType::Float);
        assert_eq!(before.unit, None);

        // A 2-D table: X has 3 breakpoints (unit A), Y has 2 (unit rpm).
        let cfg = r#"<?xml version="1.0"?>
<Configuration>
  <Table Name="Control.Limiting.Torque">
    <X><Cells Type="f32" Unit="A"><Cell>0</Cell><Cell>1</Cell><Cell>2</Cell></Cells></X>
    <Y><Cells Type="f32" Unit="rpm"><Cell>0</Cell><Cell>1</Cell></Cells></Y>
    <Body><Cells Type="f32" Unit="N.m"><Cell>10</Cell></Cells></Body>
  </Table>
</Configuration>"#;
        augment(&mut table, cfg).unwrap();

        let out = table.get("Root.Control.Limiting.Torque.Value").unwrap();
        assert_eq!(out.value_type, ValueType::Float, "from Body Type=f32");
        assert_eq!(out.unit.as_deref(), Some("N.m"), "from Body Unit");

        let meta = table
            .get("Root.Control.Limiting.Torque")
            .unwrap()
            .table_meta
            .as_ref()
            .expect("table shape");
        assert_eq!(meta.axes.len(), 2, "2-D table");
        assert_eq!(
            meta.axes[0],
            TableAxis {
                size: 3,
                unit: Some("A".into())
            }
        );
        assert_eq!(
            meta.axes[1],
            TableAxis {
                size: 2,
                unit: Some("rpm".into())
            }
        );
        assert_eq!(meta.output_unit.as_deref(), Some("N.m"));
    }

    #[test]
    fn cfg_does_not_override_authoritative_quantity_base_unit() {
        // The base unit derived from `Props@Qty` is authoritative. A `.m1cfg`
        // cell unit (which may be a display unit) must NOT clobber it; it only
        // fills in a unit when the symbol has none.
        let prj = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.Channel" Name="Root.Boost.Value">
    <Props Qty="rad/s"/>
  </Component>
</Project>"#;
        let mut table = m1prj::parse(prj).unwrap().table;
        assert_eq!(
            table.get("Root.Boost.Value").unwrap().unit.as_deref(),
            Some("deg/s"),
            "base unit from Qty before cfg"
        );

        let cfg = r#"<?xml version="1.0"?>
<Configuration>
  <Parameter Name="Boost.Value"><Cell Type="f32" Unit="rpm">0</Cell></Parameter>
</Configuration>"#;
        augment(&mut table, cfg).unwrap();

        assert_eq!(
            table.get("Root.Boost.Value").unwrap().unit.as_deref(),
            Some("deg/s"),
            "cfg display unit `rpm` must not override authoritative base unit `deg/s`"
        );
    }
}
