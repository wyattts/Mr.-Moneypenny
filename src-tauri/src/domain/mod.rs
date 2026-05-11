// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//! Domain types: the in-memory shape of expenses, categories, budgets,
//! and the period-pacing snapshot the LLM and dashboard share.

pub mod budget;
pub mod category;
pub mod expense;
pub mod period;
pub mod recurring;

pub use budget::{Budget, BudgetPeriod, NewBudget};
pub use category::{Category, CategoryKind, NewCategory};
pub use expense::{Expense, ExpenseSource, NewExpense};
pub use period::{compute_snapshot, current_month_bounds, PeriodSnapshot};
pub use recurring::{next_due, Frequency, NewRecurringRule, RecurringMode, RecurringRule};
