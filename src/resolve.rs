//! Name resolution: locals, absolute, group-relative, opaque-root passthrough.
use crate::project::Project;
use crate::types::ValueType;
use std::collections::HashMap;

pub struct Scope<'p> {
    pub locals: HashMap<String, ValueType>,
    pub group: Option<String>,
    pub project: Option<&'p Project>,
}

#[derive(Debug)]
pub enum Resolution<'p> {
    Local(ValueType),
    Symbol(&'p crate::symbols::Symbol),
    Opaque,     // resolves to a built-in/unknown root; type Unknown, never flagged
    Unresolved, // project-rooted path with no matching symbol
}

fn root_segment(path: &str) -> &str {
    match path.find('.') {
        Some(i) => &path[..i],
        None => path,
    }
}

pub fn resolve<'p>(path: &str, scope: &Scope<'p>) -> Resolution<'p> {
    // 1. Local (single segment).
    if !path.contains('.') {
        if let Some(&t) = scope.locals.get(path) {
            return Resolution::Local(t);
        }
    }
    let Some(project) = scope.project else {
        // project-less mode: locals only, everything else opaque
        return Resolution::Opaque;
    };
    let table = project.symbols();

    // 2. Absolute (as-is, then Root.-prefixed).
    if let Some(sym) = table.get(path) {
        return Resolution::Symbol(sym);
    }
    let rooted = format!("Root.{path}");
    if let Some(sym) = table.get(&rooted) {
        return Resolution::Symbol(sym);
    }

    // 3. Group-relative: walk the enclosing group up to Root.
    if let Some(group) = &scope.group {
        let mut prefix = Some(group.clone());
        while let Some(g) = prefix {
            let candidate = format!("{g}.{path}");
            if let Some(sym) = table.get(&candidate) {
                return Resolution::Symbol(sym);
            }
            prefix = g.rfind('.').map(|i| g[..i].to_string());
        }
    }

    // 4. Opaque vs unresolved: only flag when the root is a known PROJECT group.
    let root = root_segment(path);
    let root_is_project_group = table
        .get(&format!("Root.{root}"))
        .map(|s| matches!(s.kind, crate::symbols::SymbolKind::Group))
        .unwrap_or(false)
        || root == "Root";
    if root_is_project_group {
        Resolution::Unresolved
    } else {
        Resolution::Opaque
    }
}
