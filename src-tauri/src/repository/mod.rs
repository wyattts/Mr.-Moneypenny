//! Parameterized SQLite CRUD for domain entities. The LLM never sees or
//! generates SQL; every database mutation is bound through these
//! parameterized statements.

pub mod budgets;
pub mod categories;
pub mod csv_import_profiles;
pub mod expenses;
pub mod llm_usage;
pub mod merchant_rules;
pub mod recurring_rules;
pub mod settings;
