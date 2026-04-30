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
    AddRefund,
    DeleteExpense,
    QueryExpenses,
    SummarizePeriod,
    ListCategories,
    SetBudget,
    ListHouseholdMembers,
    AddRecurringRule,
    ListRecurringRules,
    DeleteRecurringRule,
    PauseRecurringRule,
}

impl ToolName {
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolName::AddExpense => "add_expense",
            ToolName::AddRefund => "add_refund",
            ToolName::DeleteExpense => "delete_expense",
            ToolName::QueryExpenses => "query_expenses",
            ToolName::SummarizePeriod => "summarize_period",
            ToolName::ListCategories => "list_categories",
            ToolName::SetBudget => "set_budget",
            ToolName::ListHouseholdMembers => "list_household_members",
            ToolName::AddRecurringRule => "add_recurring_rule",
            ToolName::ListRecurringRules => "list_recurring_rules",
            ToolName::DeleteRecurringRule => "delete_recurring_rule",
            ToolName::PauseRecurringRule => "pause_recurring_rule",
        }
    }
}

impl std::str::FromStr for ToolName {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "add_expense" => ToolName::AddExpense,
            "add_refund" => ToolName::AddRefund,
            "delete_expense" => ToolName::DeleteExpense,
            "query_expenses" => ToolName::QueryExpenses,
            "summarize_period" => ToolName::SummarizePeriod,
            "list_categories" => ToolName::ListCategories,
            "set_budget" => ToolName::SetBudget,
            "list_household_members" => ToolName::ListHouseholdMembers,
            "add_recurring_rule" => ToolName::AddRecurringRule,
            "list_recurring_rules" => ToolName::ListRecurringRules,
            "delete_recurring_rule" => ToolName::DeleteRecurringRule,
            "pause_recurring_rule" => ToolName::PauseRecurringRule,
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

/// Log a refund — money returned to the user (return, cancellation,
/// chargeback). Stored in the same `expenses` table with `is_refund = 1`,
/// so aggregations subtract it from the matching category's net spend.
#[derive(Debug, Clone, Deserialize)]
pub struct AddRefundInput {
    /// Amount refunded in major currency units. Positive number.
    pub amount: f64,
    /// Three-letter ISO currency code. Defaults to user's configured currency.
    #[serde(default)]
    pub currency: Option<String>,
    /// Category to credit the refund against. Must match an existing
    /// category exactly (case-insensitive). Use the same category as the
    /// original purchase if known.
    pub category: String,
    /// Free-text note. Should reflect the user's original message
    /// (e.g., "returned the blender to Amazon").
    #[serde(default)]
    pub description: Option<String>,
    /// When the refund occurred. YYYY-MM-DD or RFC3339. Defaults to now.
    #[serde(default)]
    pub occurred_at: Option<String>,
    /// Optional ID of the original expense being reversed. Use only when
    /// you can identify it from a recent `query_expenses` result.
    #[serde(default)]
    pub refund_for_expense_id: Option<i64>,
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
    /// Monthly budget amount in major currency units. The bot only
    /// supports monthly budgets internally; the LLM is responsible for
    /// converting "$X per week" or "$Y per year" before calling.
    pub amount: f64,
}

// ListHouseholdMembers takes no input.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ListHouseholdMembersInput {}

/// Add a recurring expense rule (e.g., "Netflix $15.49 monthly on the
/// 7th"). The scheduler fires it on its due date and either confirms
/// with the user via DM (default) or logs silently (`mode: "auto"`).
#[derive(Debug, Clone, Deserialize)]
pub struct AddRecurringRuleInput {
    /// Short label shown in the confirmation DM ("Netflix", "Rent").
    pub label: String,
    /// Amount in major currency units (positive number).
    pub amount: f64,
    /// Three-letter ISO currency code. Defaults to user's currency.
    #[serde(default)]
    pub currency: Option<String>,
    /// Category name. Must match an existing active category.
    pub category: String,
    /// "monthly" | "weekly" | "yearly".
    pub frequency: String,
    /// Day to fire on:
    ///   - monthly: day-of-month 1..31 (clamped to last day for short months).
    ///   - weekly:  ISO weekday 1..7 (Mon=1, Sun=7).
    ///   - yearly:  day-of-year 1..366.
    pub anchor_day: u16,
    /// "confirm" (default) → bot asks yes/no/skip when due.
    /// "auto" → log silently (use only for true auto-pay items).
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ListRecurringRulesInput {
    /// Include disabled / paused rules. Default false.
    #[serde(default)]
    pub include_disabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteRecurringRuleInput {
    pub rule_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PauseRecurringRuleInput {
    pub rule_id: i64,
    /// `false` → pause, `true` → resume.
    pub enabled: bool,
}

// ---------------------------------------------------------------------
// Spec assembly.
// ---------------------------------------------------------------------

/// Build the canonical list of tools to expose to the LLM.
pub fn all_tools() -> Vec<ToolSpec> {
    vec![
        add_expense_spec(),
        add_refund_spec(),
        delete_expense_spec(),
        query_expenses_spec(),
        summarize_period_spec(),
        list_categories_spec(),
        set_budget_spec(),
        list_household_members_spec(),
        add_recurring_rule_spec(),
        list_recurring_rules_spec(),
        delete_recurring_rule_spec(),
        pause_recurring_rule_spec(),
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

fn add_refund_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::AddRefund.as_str().into(),
        description: "Log a refund — money returned to the user (return, \
                      cancellation, refund, chargeback). Use this whenever \
                      the user says they got money BACK, not when they spent \
                      money. Examples: \"got $20 refund from Amazon\", \
                      \"returned the shirt, $35 back\", \"cancelled the gym \
                      subscription, refunded $40\". The amount is the \
                      positive amount returned. Aggregations will subtract \
                      it from the category's net spend automatically. \
                      If you can identify the original expense from a \
                      recent `query_expenses` result, pass its ID as \
                      `refund_for_expense_id`; otherwise omit."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "amount": {
                    "type": "number",
                    "description": "Positive amount refunded in major currency units (e.g., 20.00 for $20)."
                },
                "currency": {
                    "type": "string",
                    "description": "Three-letter ISO 4217 currency code. Defaults to the user's configured currency."
                },
                "category": {
                    "type": "string",
                    "description": "Category to credit the refund against. Use the same category as the original purchase when known."
                },
                "description": {
                    "type": "string",
                    "description": "Short note (≤ 200 chars). Should reflect the user's original message."
                },
                "occurred_at": {
                    "type": "string",
                    "description": "When the refund occurred. YYYY-MM-DD or RFC3339. Defaults to now."
                },
                "refund_for_expense_id": {
                    "type": "integer",
                    "description": "Optional ID of the original expense being reversed (from a prior query_expenses result)."
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
        description: "Set a category's MONTHLY budget. Confirm with the \
                      user before calling — budgets are user-config, not \
                      transactions. The bot only supports monthly budgets; \
                      if the user says \"$X per week\", multiply by 4.345 \
                      to get the monthly equivalent before calling."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "category": { "type": "string", "description": "Exact category name." },
                "amount": { "type": "number", "description": "Monthly budget amount in major currency units." }
            },
            "required": ["category", "amount"]
        }),
    }
}

fn add_recurring_rule_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::AddRecurringRule.as_str().into(),
        description: "Create a recurring expense rule. Use when the user \
                      describes a regular bill or subscription they want \
                      to track on autopilot — e.g., \"add Netflix $15.49 \
                      monthly on the 7th\", \"track my $1500 rent on the \
                      1st of every month\". The bot will DM the user on \
                      the due date asking yes/no/skip before logging \
                      (mode = confirm, the default). Use mode = auto ONLY \
                      when the user explicitly says it's auto-pay or \
                      always-the-same — otherwise default to confirm."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "label": { "type": "string", "description": "Short name shown in confirmations (e.g., 'Netflix', 'Rent')." },
                "amount": { "type": "number", "description": "Amount in major currency units (e.g., 15.49)." },
                "currency": { "type": "string", "description": "Three-letter ISO 4217 code. Defaults to user's currency." },
                "category": { "type": "string", "description": "Category name. Must match an existing active category." },
                "frequency": {
                    "type": "string",
                    "enum": ["monthly", "weekly", "yearly"],
                    "description": "How often the rule fires."
                },
                "anchor_day": {
                    "type": "integer",
                    "description": "Day of fire. monthly: 1-31 (auto-clamped for short months). weekly: 1=Mon..7=Sun. yearly: 1-366 day-of-year."
                },
                "mode": {
                    "type": "string",
                    "enum": ["confirm", "auto"],
                    "description": "confirm (default): bot asks yes/no/skip on due date. auto: log silently."
                }
            },
            "required": ["label", "amount", "category", "frequency", "anchor_day"]
        }),
    }
}

fn list_recurring_rules_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::ListRecurringRules.as_str().into(),
        description: "List existing recurring expense rules. Useful when \
                      the user asks \"what subscriptions do I track\" or \
                      before deleting / pausing one (you'll need the \
                      rule_id from this list)."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "include_disabled": {
                    "type": "boolean",
                    "description": "Include paused rules in the result. Default false."
                }
            },
            "required": []
        }),
    }
}

fn delete_recurring_rule_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::DeleteRecurringRule.as_str().into(),
        description: "Permanently delete a recurring rule. ONLY call after \
                      the user has explicitly confirmed deletion. Use \
                      pause_recurring_rule for a temporary stop instead \
                      of delete when the user says \"pause\" or \"stop \
                      for now\"."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "rule_id": { "type": "integer", "description": "ID from list_recurring_rules." }
            },
            "required": ["rule_id"]
        }),
    }
}

fn pause_recurring_rule_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::PauseRecurringRule.as_str().into(),
        description: "Pause or resume a recurring rule without deleting \
                      it. Use when the user says \"pause my gym for the \
                      summer\" or \"resume the cable subscription\". Pass \
                      enabled=false to pause, enabled=true to resume."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "rule_id": { "type": "integer", "description": "ID from list_recurring_rules." },
                "enabled": { "type": "boolean", "description": "true = resume, false = pause." }
            },
            "required": ["rule_id", "enabled"]
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
