//! Tool specifications and typed inputs.
//!
//! Each tool has:
//!   1. A `ToolSpec` (name + description + JSON Schema input shape) sent
//!      to the LLM provider.
//!   2. A typed `Input` struct that the dispatcher uses to deserialize
//!      the LLM's tool-call payload strictly. Anything that doesn't
//!      match the typed shape is rejected before touching the database.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::Date;

/// Names of all tools we expose. The dispatcher matches on these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolName {
    AddExpense,
    DeleteExpense,
    QueryExpenses,
    SummarizePeriod,
    ListCategories,
    SetBudget,
    ListHouseholdMembers,
}

impl ToolName {
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolName::AddExpense => "add_expense",
            ToolName::DeleteExpense => "delete_expense",
            ToolName::QueryExpenses => "query_expenses",
            ToolName::SummarizePeriod => "summarize_period",
            ToolName::ListCategories => "list_categories",
            ToolName::SetBudget => "set_budget",
            ToolName::ListHouseholdMembers => "list_household_members",
        }
    }
}

impl std::str::FromStr for ToolName {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "add_expense" => ToolName::AddExpense,
            "delete_expense" => ToolName::DeleteExpense,
            "query_expenses" => ToolName::QueryExpenses,
            "summarize_period" => ToolName::SummarizePeriod,
            "list_categories" => ToolName::ListCategories,
            "set_budget" => ToolName::SetBudget,
            "list_household_members" => ToolName::ListHouseholdMembers,
            other => anyhow::bail!("unknown tool name: {other}"),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

// ---------------------------------------------------------------------
// Typed input structs.
// ---------------------------------------------------------------------

/// Add a new expense to the local database.
#[derive(Debug, Clone, Deserialize)]
pub struct AddExpenseInput {
    /// Amount in major currency units (dollars). Will be rounded to the
    /// nearest cent on insert.
    pub amount: f64,
    /// Three-letter ISO currency code. Defaults to the user's configured
    /// currency if omitted by the LLM.
    #[serde(default)]
    pub currency: Option<String>,
    /// Category name. Must match an existing active category exactly
    /// (case-insensitive). If unsure, call `list_categories` first.
    pub category: String,
    /// Free-text note. Should reflect the user's original message
    /// (e.g., "morning latte at Blue Bottle").
    #[serde(default)]
    pub description: Option<String>,
    /// Optional override of when the expense actually occurred. Accepts
    /// `YYYY-MM-DD` (midnight in the user's offset) or RFC3339.
    /// Defaults to "now" if omitted.
    #[serde(default)]
    pub occurred_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteExpenseInput {
    pub expense_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueryExpensesInput {
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub start_date: Option<Date>,
    #[serde(default)]
    pub end_date: Option<Date>,
    #[serde(default)]
    pub min_amount: Option<f64>,
    #[serde(default)]
    pub max_amount: Option<f64>,
    #[serde(default = "default_query_limit")]
    pub limit: u32,
}

fn default_query_limit() -> u32 {
    50
}

#[derive(Debug, Clone, Deserialize)]
pub struct SummarizePeriodInput {
    /// One of: this_week | this_month | this_quarter | this_year | ytd | custom
    pub period: String,
    /// Required when `period == "custom"`.
    #[serde(default)]
    pub from: Option<Date>,
    /// Required when `period == "custom"`.
    #[serde(default)]
    pub to: Option<Date>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListCategoriesInput {
    /// Include deactivated categories. Default false.
    #[serde(default)]
    pub include_inactive: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetBudgetInput {
    pub category: String,
    pub amount: f64,
    /// "weekly" | "monthly" | "yearly". Defaults to "monthly".
    #[serde(default = "default_budget_period")]
    pub period: String,
}

fn default_budget_period() -> String {
    "monthly".into()
}

// ListHouseholdMembers takes no input.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ListHouseholdMembersInput {}

// ---------------------------------------------------------------------
// Spec assembly.
// ---------------------------------------------------------------------

/// Build the canonical list of tools to expose to the LLM.
pub fn all_tools() -> Vec<ToolSpec> {
    vec![
        add_expense_spec(),
        delete_expense_spec(),
        query_expenses_spec(),
        summarize_period_spec(),
        list_categories_spec(),
        set_budget_spec(),
        list_household_members_spec(),
    ]
}

fn add_expense_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::AddExpense.as_str().into(),
        description: "Log a new expense. Use this whenever the user describes \
                      having spent money (e.g., \"$5 coffee\", \"paid rent $1500\"). \
                      Convert the amount to a decimal number in the user's \
                      currency. The category must exactly match an existing \
                      active category — call `list_categories` first if unsure."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "amount": {
                    "type": "number",
                    "description": "Amount in major currency units (e.g., 5.00 for $5, 7.99 for $7.99). Rounded to the nearest cent on insert."
                },
                "currency": {
                    "type": "string",
                    "description": "Three-letter ISO 4217 currency code (e.g., USD, EUR). Defaults to the user's configured currency."
                },
                "category": {
                    "type": "string",
                    "description": "Exact name of an existing active category."
                },
                "description": {
                    "type": "string",
                    "description": "Short note about the expense (≤ 200 chars). Should reflect the user's original message."
                },
                "occurred_at": {
                    "type": "string",
                    "description": "When the expense occurred. Accepts YYYY-MM-DD (midnight) or RFC3339 datetime. Defaults to now."
                }
            },
            "required": ["amount", "category"]
        }),
    }
}

fn delete_expense_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::DeleteExpense.as_str().into(),
        description: "Delete an expense. ONLY call this after the user has \
                      explicitly confirmed deletion (e.g., they replied \"yes\" \
                      or \"confirm\" to a confirmation message you sent). \
                      Never delete on speculation."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "expense_id": {
                    "type": "integer",
                    "description": "The ID of the expense to delete (from a prior query result)."
                }
            },
            "required": ["expense_id"]
        }),
    }
}

fn query_expenses_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::QueryExpenses.as_str().into(),
        description: "Look up past expenses with optional filters. Use this \
                      to answer questions like \"how much have I spent on \
                      coffee this month\" or \"what was my last grocery run\"."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "category": { "type": "string", "description": "Filter by category name (exact, case-insensitive)." },
                "start_date": { "type": "string", "description": "ISO date YYYY-MM-DD; inclusive." },
                "end_date": { "type": "string", "description": "ISO date YYYY-MM-DD; inclusive." },
                "min_amount": { "type": "number", "description": "Lower bound, in major currency units." },
                "max_amount": { "type": "number", "description": "Upper bound, in major currency units." },
                "limit": { "type": "integer", "description": "Max rows to return (default 50, hard cap 500)." }
            },
            "required": []
        }),
    }
}

fn summarize_period_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::SummarizePeriod.as_str().into(),
        description: "Get a structured budget summary. The dashboard and you \
                      use the same math, so call this whenever the user asks \
                      \"how am I doing\". Pace VARIABLE spending against \
                      the variable budget — fixed expenses (rent, insurance, \
                      etc.) are inevitable and must NOT make the user look \
                      \"over\". Only count discretionary overspend."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "period": {
                    "type": "string",
                    "enum": ["this_week", "this_month", "this_quarter", "this_year", "ytd", "custom"],
                    "description": "Reporting window. `this_month` is the default for casual queries."
                },
                "from": { "type": "string", "description": "ISO date YYYY-MM-DD, required when period='custom'." },
                "to": { "type": "string", "description": "ISO date YYYY-MM-DD, required when period='custom'." }
            },
            "required": ["period"]
        }),
    }
}

fn list_categories_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::ListCategories.as_str().into(),
        description: "Return the list of expense categories with their kind \
                      (fixed | variable) and current monthly target. Call \
                      this when you don't know the exact category name the \
                      user means."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "include_inactive": {
                    "type": "boolean",
                    "description": "Include categories the user has deactivated."
                }
            },
            "required": []
        }),
    }
}

fn set_budget_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::SetBudget.as_str().into(),
        description: "Set a category's budget. Confirm with the user before \
                      calling — budgets are user-config, not transactions."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "category": { "type": "string", "description": "Exact category name." },
                "amount": { "type": "number", "description": "Budget amount in major currency units." },
                "period": {
                    "type": "string",
                    "enum": ["weekly", "monthly", "yearly"],
                    "description": "Budget period. Default monthly."
                }
            },
            "required": ["category", "amount"]
        }),
    }
}

fn list_household_members_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::ListHouseholdMembers.as_str().into(),
        description: "Return the authorized chats (household members) and \
                      their roles. Useful for per-person spending questions \
                      (\"how much did Spouse spend on dining\")."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    }
}
