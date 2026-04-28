//! Domain types: the in-memory shape of expenses, categories, budgets,
//! and the period-pacing snapshot the LLM and dashboard share.

pub mod budget;
pub mod category;
pub mod expense;
pub mod period;

pub use budget::{Budget, BudgetPeriod, NewBudget};
pub use category::{Category, CategoryKind, NewCategory};
pub use expense::{Expense, ExpenseSource, NewExpense};
pub use period::{compute_snapshot, current_month_bounds, PeriodSnapshot};
