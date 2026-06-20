//! m1-typecheck: symbol model + name resolution + basic type checking for .m1scr.
pub mod ambiguous;
pub mod audit;
pub mod classname;
pub mod cross_script;
pub mod diagnostics;
pub mod expand;
pub mod filter;
pub mod flow;
pub mod in_out_io;
pub mod intrinsics;
pub mod invalid_value;
pub mod parsed;
pub mod project;
pub mod resolve;
pub mod rules;
pub mod schedule;
pub mod symbols;
pub mod typer;
pub mod types;
pub mod unit_table;
pub mod units;

pub use diagnostics::{CheckResult, TypeCode, TypeDiagnostic};
pub use project::Project;
pub use types::ValueType;
