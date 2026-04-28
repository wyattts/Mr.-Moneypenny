//! Tool dispatcher — the safety boundary between the LLM and SQLite.
//!
//! Every tool call from the LLM is:
//!   1. Looked up by name (unknown tools rejected).
//!   2. Strictly deserialized into a typed input struct (rejects malformed
//!      arguments before any DB access).
//!   3. Executed through the parameterized repository.
//!   4. Returned as a string `ToolOutput` the LLM can interpret in its
//!      next turn.
//!
//! The dispatcher never builds SQL strings from LLM input.

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{Date, OffsetDateTime, Time};

use crate::domain::{
    BudgetPeriod, Category, CategoryKind, Expense, ExpenseSource, NewBudget, NewExpense,
};
use crate::insights::{dashboard, range::DateRange};
use crate::repository::{budgets, categories, expenses};

use super::tools::{
    AddExpenseInput, DeleteExpenseInput, ListCategoriesInput, ListHouseholdMembersInput,
    QueryExpensesInput, SetBudgetInput, SummarizePeriodInput, ToolName,
};

/// Per-call context that the LLM doesn't supply but the dispatcher needs:
/// who's talking, what currency to default to, what time it is.
#[derive(Debug, Clone)]
pub struct CallContext {
    pub now: OffsetDateTime,
    pub authorized_chat_id: Option<i64>,
    pub authorized_chat_name: Option<String>,
    pub default_currency: String,
}

/// What gets sent back to the LLM as the next-turn `tool_result`.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

/// Execute a single tool call.
pub fn execute(
    conn: &Connection,
    ctx: &CallContext,
    tool_use_id: &str,
    tool_name: &str,
    input: &Value,
) -> ToolOutput {
    let make_err = |msg: String| ToolOutput {
        tool_use_id: tool_use_id.to_string(),
        content: msg,
        is_error: true,
    };
    let make_ok = |body: Value| ToolOutput {
        tool_use_id: tool_use_id.to_string(),
        content: body.to_string(),
        is_error: false,
    };

    let name: ToolName = match tool_name.parse() {
        Ok(n) => n,
        Err(_) => return make_err(format!("unknown tool: {tool_name}")),
    };

    let result: Result<Value> = match name {
        ToolName::AddExpense => exec_add_expense(conn, ctx, input),
        ToolName::DeleteExpense => exec_delete_expense(conn, input),
        ToolName::QueryExpenses => exec_query_expenses(conn, ctx, input),
        ToolName::SummarizePeriod => exec_summarize_period(conn, ctx, input),
        ToolName::ListCategories => exec_list_categories(conn, input),
        ToolName::SetBudget => exec_set_budget(conn, ctx, input),
        ToolName::ListHouseholdMembers => exec_list_household_members(conn, input),
    };

    match result {
        Ok(v) => make_ok(v),
        Err(e) => make_err(format!("{e:#}")),
    }
}

// ---------------------------------------------------------------------
// Tool handlers.
// ---------------------------------------------------------------------

fn exec_add_expense(conn: &Connection, ctx: &CallContext, input: &Value) -> Result<Value> {
    let parsed: AddExpenseInput =
        serde_json::from_value(input.clone()).context("add_expense: invalid arguments")?;

    if !(parsed.amount.is_finite() && parsed.amount >= 0.0) {
        return Err(anyhow!(
            "add_expense: amount must be a non-negative finite number"
        ));
    }
    let amount_cents = (parsed.amount * 100.0).round() as i64;
    if amount_cents <= 0 {
        return Err(anyhow!(
            "add_expense: amount rounds to zero cents — too small"
        ));
    }

    let cat = resolve_category(conn, &parsed.category)?;

    let occurred_at = match parsed.occurred_at {
        None => ctx.now,
        Some(s) => parse_datetime_or_date(&s, ctx.now.offset())?,
    };

    let currency = parsed
        .currency
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| ctx.default_currency.clone());

    let id = expenses::insert(
        conn,
        &NewExpense {
            amount_cents,
            currency: currency.clone(),
            category_id: Some(cat.id),
            description: parsed.description,
            occurred_at,
            source: ExpenseSource::Telegram,
            raw_message: None,
            llm_confidence: None,
            logged_by_chat_id: ctx.authorized_chat_id,
        },
    )?;

    Ok(json!({
        "ok": true,
        "expense_id": id,
        "amount_cents": amount_cents,
        "currency": currency,
        "category": cat.name,
        "category_kind": cat.kind.as_str(),
        "occurred_at": occurred_at.format(&Rfc3339).unwrap_or_default(),
        "logged_by": ctx.authorized_chat_name.clone(),
    }))
}

fn exec_delete_expense(conn: &Connection, input: &Value) -> Result<Value> {
    let parsed: DeleteExpenseInput =
        serde_json::from_value(input.clone()).context("delete_expense: invalid arguments")?;
    let removed = expenses::delete(conn, parsed.expense_id)?;
    if !removed {
        return Err(anyhow!(
            "delete_expense: no expense with id {}",
            parsed.expense_id
        ));
    }
    Ok(json!({ "ok": true, "deleted_id": parsed.expense_id }))
}

fn exec_query_expenses(conn: &Connection, ctx: &CallContext, input: &Value) -> Result<Value> {
    let parsed: QueryExpensesInput =
        serde_json::from_value(input.clone()).context("query_expenses: invalid arguments")?;

    let limit = parsed.limit.min(500);
    let offset = ctx.now.offset();

    let mut sql =
        "SELECT e.id, e.amount_cents, e.currency, e.category_id, c.name, e.description, e.occurred_at, e.created_at, e.source, e.raw_message, e.llm_confidence, e.logged_by_chat_id \
         FROM expenses e LEFT JOIN categories c ON c.id = e.category_id WHERE 1=1"
            .to_string();

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(cat_name) = parsed.category.as_ref() {
        sql.push_str(" AND LOWER(c.name) = LOWER(?)");
        params.push(Box::new(cat_name.clone()));
    }
    if let Some(d) = parsed.start_date {
        sql.push_str(" AND e.occurred_at >= ?");
        params.push(Box::new(d.with_time(Time::MIDNIGHT).assume_offset(offset)));
    }
    if let Some(d) = parsed.end_date {
        // Inclusive end_date → use start of next day
        let next = d + time::Duration::days(1);
        sql.push_str(" AND e.occurred_at < ?");
        params.push(Box::new(
            next.with_time(Time::MIDNIGHT).assume_offset(offset),
        ));
    }
    if let Some(min) = parsed.min_amount {
        if !min.is_finite() {
            return Err(anyhow!("query_expenses: min_amount must be finite"));
        }
        sql.push_str(" AND e.amount_cents >= ?");
        params.push(Box::new((min * 100.0).round() as i64));
    }
    if let Some(max) = parsed.max_amount {
        if !max.is_finite() {
            return Err(anyhow!("query_expenses: max_amount must be finite"));
        }
        sql.push_str(" AND e.amount_cents <= ?");
        params.push(Box::new((max * 100.0).round() as i64));
    }
    sql.push_str(" ORDER BY e.occurred_at DESC, e.id DESC LIMIT ?");
    params.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok(QueryRow {
                id: r.get(0)?,
                amount_cents: r.get(1)?,
                currency: r.get(2)?,
                category_id: r.get(3)?,
                category_name: r.get(4)?,
                description: r.get(5)?,
                occurred_at: r.get(6)?,
                _created_at: r.get::<_, OffsetDateTime>(7)?,
                _source: r.get::<_, ExpenseSource>(8)?,
                _raw_message: r.get::<_, Option<String>>(9)?,
                _llm_confidence: r.get::<_, Option<f64>>(10)?,
                logged_by_chat_id: r.get(11)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let total_cents: i64 = rows.iter().map(|r| r.amount_cents).sum();
    Ok(json!({
        "ok": true,
        "count": rows.len(),
        "total_cents": total_cents,
        "expenses": rows,
    }))
}

#[derive(Debug, Serialize)]
struct QueryRow {
    id: i64,
    amount_cents: i64,
    currency: String,
    category_id: Option<i64>,
    category_name: Option<String>,
    description: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    occurred_at: OffsetDateTime,
    #[serde(skip)]
    _created_at: OffsetDateTime,
    #[serde(skip)]
    _source: ExpenseSource,
    #[serde(skip)]
    _raw_message: Option<String>,
    #[serde(skip)]
    _llm_confidence: Option<f64>,
    logged_by_chat_id: Option<i64>,
}

fn exec_summarize_period(conn: &Connection, ctx: &CallContext, input: &Value) -> Result<Value> {
    let parsed: SummarizePeriodInput =
        serde_json::from_value(input.clone()).context("summarize_period: invalid arguments")?;
    let range = match parsed.period.as_str() {
        "this_week" => DateRange::ThisWeek,
        "this_month" => DateRange::ThisMonth,
        "this_quarter" => DateRange::ThisQuarter,
        "this_year" => DateRange::ThisYear,
        "ytd" => DateRange::Ytd,
        "custom" => {
            let from = parsed
                .from
                .ok_or_else(|| anyhow!("summarize_period: 'custom' requires 'from'"))?;
            let to = parsed
                .to
                .ok_or_else(|| anyhow!("summarize_period: 'custom' requires 'to'"))?;
            DateRange::Custom { from, to }
        }
        other => {
            return Err(anyhow!(
                "summarize_period: unknown period '{other}'; \
                 must be one of this_week|this_month|this_quarter|this_year|ytd|custom"
            ));
        }
    };
    let snap = dashboard(conn, range, ctx.now)?;
    Ok(serde_json::to_value(snap)?)
}

fn exec_list_categories(conn: &Connection, input: &Value) -> Result<Value> {
    let parsed: ListCategoriesInput =
        serde_json::from_value(input.clone()).context("list_categories: invalid arguments")?;
    let cats = categories::list(conn, parsed.include_inactive)?;
    let slim: Vec<_> = cats
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "kind": c.kind.as_str(),
                "monthly_target_cents": c.monthly_target_cents,
                "is_recurring": c.is_recurring,
                "recurrence_day_of_month": c.recurrence_day_of_month,
                "is_active": c.is_active,
            })
        })
        .collect();
    Ok(json!({ "ok": true, "categories": slim }))
}

fn exec_set_budget(conn: &Connection, ctx: &CallContext, input: &Value) -> Result<Value> {
    let parsed: SetBudgetInput =
        serde_json::from_value(input.clone()).context("set_budget: invalid arguments")?;
    if !(parsed.amount.is_finite() && parsed.amount >= 0.0) {
        return Err(anyhow!(
            "set_budget: amount must be a non-negative finite number"
        ));
    }
    let cat = resolve_category(conn, &parsed.category)?;
    let period: BudgetPeriod = parsed.period.parse()?;
    let id = budgets::insert(
        conn,
        &NewBudget {
            category_id: cat.id,
            amount_cents: (parsed.amount * 100.0).round() as i64,
            period,
            effective_from: ctx.now,
            effective_to: None,
        },
    )?;
    Ok(json!({
        "ok": true,
        "budget_id": id,
        "category": cat.name,
        "amount_cents": (parsed.amount * 100.0).round() as i64,
        "period": period.as_str(),
    }))
}

fn exec_list_household_members(conn: &Connection, input: &Value) -> Result<Value> {
    let _parsed: ListHouseholdMembersInput = serde_json::from_value(input.clone())
        .context("list_household_members: invalid arguments")?;
    let mut stmt = conn.prepare_cached(
        "SELECT chat_id, display_name, role FROM telegram_authorized_chats ORDER BY role DESC, display_name ASC",
    )?;
    let rows: Vec<Value> = stmt
        .query_map([], |r| {
            let chat_id: i64 = r.get(0)?;
            let display_name: String = r.get(1)?;
            let role: String = r.get(2)?;
            Ok(json!({
                "chat_id": chat_id,
                "display_name": display_name,
                "role": role,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({ "ok": true, "members": rows }))
}

// ---------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------

/// Resolve a category by name, case-insensitive. Returns helpful error
/// (with the list of available active categories) if nothing matches.
fn resolve_category(conn: &Connection, name: &str) -> Result<Category> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("category name is empty"));
    }

    // Case-insensitive exact match
    let mut stmt = conn.prepare_cached(
        "SELECT id, name, kind, monthly_target_cents, is_recurring, recurrence_day_of_month, is_active, is_seed
         FROM categories WHERE LOWER(name) = LOWER(?1) LIMIT 1",
    )?;
    let cat: Option<Category> = stmt
        .query_row(params![trimmed], |r| {
            Ok(Category {
                id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                monthly_target_cents: r.get(3)?,
                is_recurring: r.get::<_, i64>(4)? != 0,
                recurrence_day_of_month: r.get::<_, Option<i64>>(5)?.map(|d| d as u8),
                is_active: r.get::<_, i64>(6)? != 0,
                is_seed: r.get::<_, i64>(7)? != 0,
            })
        })
        .ok();

    if let Some(c) = cat {
        if !c.is_active {
            return Err(anyhow!(
                "category '{}' is deactivated; ask the user to reactivate it or pick another",
                c.name
            ));
        }
        return Ok(c);
    }

    // No exact match — surface the active list so the LLM can correct itself.
    let active = categories::list(conn, false)?;
    let names: Vec<&str> = active.iter().map(|c| c.name.as_str()).collect();
    Err(anyhow!(
        "no category named '{trimmed}' (active categories: {})",
        names.join(", ")
    ))
}

/// Accept either RFC3339 datetime or YYYY-MM-DD; return as `OffsetDateTime`
/// in the requested offset.
fn parse_datetime_or_date(s: &str, offset: time::UtcOffset) -> Result<OffsetDateTime> {
    if let Ok(dt) = OffsetDateTime::parse(s, &Rfc3339) {
        return Ok(dt);
    }
    let date_fmt = format_description!("[year]-[month]-[day]");
    if let Ok(d) = Date::parse(s, &date_fmt) {
        return Ok(d.with_time(Time::MIDNIGHT).assume_offset(offset));
    }
    Err(anyhow!(
        "could not parse '{s}' as date (YYYY-MM-DD) or RFC3339 datetime"
    ))
}

#[allow(dead_code)] // re-exported for tests / future use
pub fn category_kind_to_str(k: CategoryKind) -> &'static str {
    k.as_str()
}

#[allow(dead_code)]
fn _expense_used(_e: Expense) {}
