//! Symbol model loaded from .m1prj / .m1cfg.
pub mod enums;
pub mod m1prj;
pub mod m1cfg;

use crate::types::ValueType;
pub use enums::{EnumId, EnumType};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Channel,
    Parameter,
    Constant,
    Function,
    Method,
    Table,
    Group,
    Reference,
    Other,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub path: String,
    pub kind: SymbolKind,
    pub value_type: ValueType,
    pub unit: Option<String>,
    pub filename: Option<String>,
}

#[derive(Debug, Default)]
pub struct SymbolTable {
    symbols: Vec<Symbol>,
    by_path: HashMap<String, usize>,
    enums: Vec<EnumType>,
}

impl SymbolTable {
    pub fn push(&mut self, sym: Symbol) {
        let idx = self.symbols.len();
        self.by_path.insert(sym.path.clone(), idx);
        self.symbols.push(sym);
    }
    pub fn get(&self, path: &str) -> Option<&Symbol> {
        self.by_path.get(path).map(|&i| &self.symbols[i])
    }
    pub fn get_mut(&mut self, path: &str) -> Option<&mut Symbol> {
        self.by_path.get(path).map(|&i| &mut self.symbols[i])
    }
    pub fn len(&self) -> usize {
        self.symbols.len()
    }
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.iter()
    }
    pub fn add_enum(&mut self, e: EnumType) -> EnumId {
        self.enums.push(e);
        self.enums.len() - 1
    }
    pub fn enums(&self) -> &[EnumType] {
        &self.enums
    }
}
