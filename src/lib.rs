//! m1-typecheck: symbol model + name resolution + basic type checking for .m1scr.
pub mod types;
pub mod symbols;
pub mod project;
pub mod resolve;
pub mod typer;
pub mod diagnostics;
pub mod rules;

// Re-exports are added as the underlying types come online in later tasks.
// pub use diagnostics::{CheckResult, TypeCode, TypeDiagnostic};
// pub use project::Project;
// pub use types::ValueType;
