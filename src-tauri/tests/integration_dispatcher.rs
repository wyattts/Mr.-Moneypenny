//! Integration tests for the LLM tool dispatcher — the safety boundary
//! between the LLM and SQLite.

use moneypenny_lib::db;
use moneypenny_lib::llm::dispatcher::{execute, CallContext, ToolOutput};
use rusqlite::Connection;
use serde_json::{json, Value};
use time::macros::datetime;

fn fresh_db() -> Connection {
    let conn = db::open_in_memory().unwrap();
    db::migrate(&conn).unwrap();
    // These tests exercise dispatcher behavior across the full seed set
    // (e.g. Coffee is the canonical example for add_expense). Migration
    // 0003 ships only 14 default-active seeds; the remainder are flipped
    // off. Reactivate everything seeded so tests aren't coupled to the
    // curated default-active list.
    conn.execute("UPDATE categories SET is_active = 1 WHERE is_seed = 1", [])
        .unwrap();
    conn
}

fn ctx_with_chat(name: &str) -> CallContext {
    CallContext {
        now: datetime!(2026-04-15 12:00:00 UTC),
        authorized_chat_id: Some(111),
        authorized_chat_name: Some(name.into()),
        default_currency: "USD".into(),
    }
}

fn ctx_solo() -> CallContext {
    CallContext {
        now: datetime!(2026-04-15 12:00:00 UTC),
        authorized_chat_id: None,
        authorized_chat_name: None,
        default_currency: "USD".into(),
    }
}

fn parse_ok(out: ToolOutput) -> Value {
    assert!(!out.is_error, "expected ok, got error: {}", out.content);
    serde_json::from_str(&out.content).expect("ok content is JSON")
}

fn assert_err_contains(out: ToolOutput, needle: &str) {
    assert!(out.is_error, "expected error, got ok: {}", out.content);
    assert!(
        out.content.contains(needle),
        "error did not contain {needle:?}: {}",
        out.content
    );
}

// ---- add_expense ----

#[test]
fn add_expense_happy_path() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({ "amount": 5.0, "category": "Coffee" }),
    );
    let v = parse_ok(out);
    assert_eq!(v["ok"], true);
    assert_eq!(v["amount_cents"], 500);
    assert_eq!(v["category"], "Coffee");
    assert_eq!(v["category_kind"], "variable");
    assert_eq!(v["currency"], "USD");
}

#[test]
fn add_expense_decimal_amount_rounds_to_cents() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({ "amount": 7.99, "category": "Coffee" }),
    ));
    assert_eq!(v["amount_cents"], 799);
}

#[test]
fn add_expense_unknown_category_lists_alternatives() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({ "amount": 5, "category": "Espresso" }),
    );
    assert_err_contains(out, "no category named 'Espresso'");
}

#[test]
fn add_expense_case_insensitive_category() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({ "amount": 5, "category": "coffee" }),
    ));
    assert_eq!(v["category"], "Coffee"); // canonical name returned
}

#[test]
fn add_expense_with_iso_date_only() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({
            "amount": 5,
            "category": "Coffee",
            "occurred_at": "2026-04-10"
        }),
    ));
    let occ = v["occurred_at"].as_str().unwrap();
    assert!(occ.starts_with("2026-04-10"), "got {occ}");
}

#[test]
fn add_expense_with_rfc3339_datetime() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({
            "amount": 5,
            "category": "Coffee",
            "occurred_at": "2026-04-10T15:30:00Z"
        }),
    ));
    let occ = v["occurred_at"].as_str().unwrap();
    assert!(occ.starts_with("2026-04-10"), "got {occ}");
}

#[test]
fn add_expense_zero_amount_rejected() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({ "amount": 0, "category": "Coffee" }),
    );
    assert_err_contains(out, "rounds to zero");
}

#[test]
fn add_expense_negative_amount_rejected() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({ "amount": -5, "category": "Coffee" }),
    );
    assert_err_contains(out, "non-negative");
}

#[test]
fn add_expense_attribution_records_chat_id() {
    let conn = fresh_db();
    // Seed a chat
    conn.execute(
        "INSERT INTO telegram_authorized_chats (chat_id, display_name, role) VALUES (111, 'Wyatt', 'owner')",
        [],
    )
    .unwrap();
    let ctx = ctx_with_chat("Wyatt");
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({ "amount": 5, "category": "Coffee" }),
    ));
    let exp_id = v["expense_id"].as_i64().unwrap();
    let chat_id: Option<i64> = conn
        .query_row(
            "SELECT logged_by_chat_id FROM expenses WHERE id = ?1",
            [exp_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(chat_id, Some(111));
}

#[test]
fn add_expense_uses_default_currency() {
    let conn = fresh_db();
    let mut ctx = ctx_solo();
    ctx.default_currency = "EUR".into();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({ "amount": 5, "category": "Coffee" }),
    ));
    assert_eq!(v["currency"], "EUR");
}

#[test]
fn add_expense_explicit_currency_overrides_default() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({ "amount": 5, "category": "Coffee", "currency": "GBP" }),
    ));
    assert_eq!(v["currency"], "GBP");
}

// ---- delete_expense ----

#[test]
fn delete_expense_happy_path() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let added = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({ "amount": 5, "category": "Coffee" }),
    ));
    let id = added["expense_id"].as_i64().unwrap();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_2",
        "delete_expense",
        &json!({ "expense_id": id }),
    ));
    assert_eq!(v["ok"], true);
    assert_eq!(v["deleted_id"], id);
}

#[test]
fn delete_expense_missing_id() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "delete_expense",
        &json!({ "expense_id": 999_999 }),
    );
    assert_err_contains(out, "no expense with id 999999");
}

// ---- query_expenses ----

#[test]
fn query_expenses_filters_by_category_and_date() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({ "amount": 5, "category": "Coffee", "occurred_at": "2026-04-10" }),
    ));
    parse_ok(execute(
        &conn,
        &ctx,
        "tu_2",
        "add_expense",
        &json!({ "amount": 3, "category": "Coffee", "occurred_at": "2026-04-12" }),
    ));
    parse_ok(execute(
        &conn,
        &ctx,
        "tu_3",
        "add_expense",
        &json!({ "amount": 50, "category": "Groceries", "occurred_at": "2026-04-11" }),
    ));

    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_q",
        "query_expenses",
        &json!({
            "category": "Coffee",
            "start_date": "2026-04-10",
            "end_date": "2026-04-12"
        }),
    ));
    assert_eq!(v["count"], 2);
    assert_eq!(v["total_cents"], 800);
}

// ---- summarize_period ----

#[test]
fn summarize_period_this_month_returns_pacing() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "summarize_period",
        &json!({ "period": "this_month" }),
    ));
    // Period block populated with on-pace status
    assert!(v["period"].is_object());
    assert_eq!(v["period"]["day_of_month"], 15);
    assert_eq!(v["period"]["days_in_period"], 30);
    assert!(v["mom_comparison"].is_object());
}

#[test]
fn summarize_period_custom_requires_from_to() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "summarize_period",
        &json!({ "period": "custom" }),
    );
    assert_err_contains(out, "requires 'from'");
}

#[test]
fn summarize_period_unknown_period_rejected() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "summarize_period",
        &json!({ "period": "fortnight" }),
    );
    assert_err_contains(out, "unknown period 'fortnight'");
}

// ---- list_categories ----

#[test]
fn list_categories_returns_seed_set() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let v = parse_ok(execute(&conn, &ctx, "tu_1", "list_categories", &json!({})));
    let cats = v["categories"].as_array().unwrap();
    assert!(cats.len() >= 25);
    assert!(cats
        .iter()
        .any(|c| c["name"] == "Coffee" && c["kind"] == "variable"));
}

// ---- set_budget ----

#[test]
fn set_budget_updates_category_monthly_target() {
    use moneypenny_lib::repository::categories;
    let conn = fresh_db();
    let ctx = ctx_solo();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "set_budget",
        &json!({ "category": "Dining Out", "amount": 250.0 }),
    ));
    assert_eq!(v["monthly_target_cents"], 25_000);
    assert_eq!(v["category"], "Dining Out");

    // Persisted on the category itself — the same field the dashboard,
    // summarize_period, and over-budget detection all read.
    let dining = categories::get_by_name(&conn, "Dining Out")
        .unwrap()
        .unwrap();
    assert_eq!(dining.monthly_target_cents, Some(25_000));
}

#[test]
fn set_budget_negative_rejected() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "set_budget",
        &json!({ "category": "Coffee", "amount": -50 }),
    );
    assert_err_contains(out, "non-negative");
}

// ---- list_household_members ----

#[test]
fn list_household_members_returns_authorized_chats() {
    let conn = fresh_db();
    conn.execute(
        "INSERT INTO telegram_authorized_chats (chat_id, display_name, role) VALUES (111, 'Wyatt', 'owner'), (222, 'Spouse', 'member')",
        [],
    )
    .unwrap();
    let ctx = ctx_solo();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "list_household_members",
        &json!({}),
    ));
    let members = v["members"].as_array().unwrap();
    assert_eq!(members.len(), 2);
}

// ---- generic safety ----

#[test]
fn unknown_tool_rejected() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(&conn, &ctx, "tu_1", "drop_table", &json!({}));
    assert_err_contains(out, "unknown tool: drop_table");
}

#[test]
fn malformed_arguments_rejected() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    // amount is required and must be a number; pass a string
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "add_expense",
        &json!({ "amount": "five dollars", "category": "Coffee" }),
    );
    assert_err_contains(out, "invalid arguments");
}

#[test]
fn add_expense_then_query_round_trips() {
    // The acceptance scenario from the plan: log "$5 coffee", read it back.
    let conn = fresh_db();
    let ctx = ctx_solo();
    let added = parse_ok(execute(
        &conn,
        &ctx,
        "tu_log",
        "add_expense",
        &json!({
            "amount": 5,
            "category": "Coffee",
            "description": "morning latte"
        }),
    ));
    let id = added["expense_id"].as_i64().unwrap();
    assert!(id > 0);

    let queried = parse_ok(execute(
        &conn,
        &ctx,
        "tu_query",
        "query_expenses",
        &json!({ "category": "Coffee" }),
    ));
    let exps = queried["expenses"].as_array().unwrap();
    assert_eq!(exps.len(), 1);
    assert_eq!(exps[0]["amount_cents"], 500);
    assert_eq!(exps[0]["description"], "morning latte");
}

// ---- add_refund ----

#[test]
fn add_refund_happy_path() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_refund",
        &json!({ "amount": 20.0, "category": "Groceries", "description": "returned the milk" }),
    ));
    assert_eq!(v["ok"], true);
    assert_eq!(v["amount_cents"], 2_000);
    assert_eq!(v["category"], "Groceries");
    assert!(
        v.get("refund_id").is_some(),
        "response includes refund_id: {v}"
    );
    // refund_for_expense_id null when not supplied.
    assert!(
        v["refund_for_expense_id"].is_null(),
        "no parent FK when not supplied"
    );
}

#[test]
fn add_refund_with_parent_links_correctly() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let parent = parse_ok(execute(
        &conn,
        &ctx,
        "tu_p",
        "add_expense",
        &json!({ "amount": 50.0, "category": "Groceries" }),
    ));
    let parent_id = parent["expense_id"].as_i64().unwrap();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_r",
        "add_refund",
        &json!({
            "amount": 12.5,
            "category": "Groceries",
            "refund_for_expense_id": parent_id,
        }),
    ));
    assert_eq!(v["amount_cents"], 1_250);
    assert_eq!(v["refund_for_expense_id"].as_i64(), Some(parent_id));
}

#[test]
fn add_refund_with_nonexistent_parent_rejected() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "add_refund",
        &json!({
            "amount": 10.0,
            "category": "Groceries",
            "refund_for_expense_id": 999_999,
        }),
    );
    assert_err_contains(out, "does not exist");
}

#[test]
fn add_refund_pointing_at_another_refund_rejected() {
    // Refund-of-a-refund is almost certainly LLM error; reject.
    let conn = fresh_db();
    let ctx = ctx_solo();
    let purchase = parse_ok(execute(
        &conn,
        &ctx,
        "tu_p",
        "add_expense",
        &json!({ "amount": 50.0, "category": "Groceries" }),
    ));
    let purchase_id = purchase["expense_id"].as_i64().unwrap();
    let refund = parse_ok(execute(
        &conn,
        &ctx,
        "tu_r1",
        "add_refund",
        &json!({
            "amount": 10.0,
            "category": "Groceries",
            "refund_for_expense_id": purchase_id,
        }),
    ));
    let refund_id = refund["refund_id"].as_i64().unwrap();
    let out = execute(
        &conn,
        &ctx,
        "tu_r2",
        "add_refund",
        &json!({
            "amount": 5.0,
            "category": "Groceries",
            "refund_for_expense_id": refund_id,
        }),
    );
    assert_err_contains(out, "is itself a refund");
}

#[test]
fn add_refund_zero_amount_rejected() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "add_refund",
        &json!({ "amount": 0, "category": "Groceries" }),
    );
    assert_err_contains(out, "rounds to zero");
}

// ---- recurring rules ----

#[test]
fn add_recurring_rule_creates_rule_and_schedules_job() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_recurring_rule",
        &json!({
            "label": "Netflix",
            "amount": 15.49,
            "category": "Entertainment",
            "frequency": "monthly",
            "anchor_day": 7,
        }),
    ));
    assert_eq!(v["ok"], true);
    let rule_id = v["rule_id"].as_i64().unwrap();
    assert_eq!(v["mode"], "confirm");
    assert_eq!(v["amount_cents"], 1_549);

    // Scheduled job must exist with this rule_id in its payload.
    let job_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM scheduled_jobs WHERE kind = 'recurring_expense' \
             AND json_extract(payload, '$.rule_id') = ?1",
            [rule_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(job_count, 1);
}

#[test]
fn add_recurring_rule_invalid_anchor_for_weekly_rejected() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "add_recurring_rule",
        &json!({
            "label": "Trash",
            "amount": 20.0,
            "category": "Household",
            "frequency": "weekly",
            "anchor_day": 12,
        }),
    );
    assert_err_contains(out, "out of range");
}

#[test]
fn add_recurring_rule_unknown_mode_rejected() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let out = execute(
        &conn,
        &ctx,
        "tu_1",
        "add_recurring_rule",
        &json!({
            "label": "Bad",
            "amount": 5.0,
            "category": "Misc",
            "frequency": "monthly",
            "anchor_day": 1,
            "mode": "yolo",
        }),
    );
    assert_err_contains(out, "unknown mode");
}

#[test]
fn list_recurring_rules_returns_added_rule() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_recurring_rule",
        &json!({
            "label": "Spotify",
            "amount": 9.99,
            "category": "Entertainment",
            "frequency": "monthly",
            "anchor_day": 12,
            "mode": "auto",
        }),
    ));
    let v = parse_ok(execute(
        &conn,
        &ctx,
        "tu_2",
        "list_recurring_rules",
        &json!({}),
    ));
    let rules = v["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["label"], "Spotify");
    assert_eq!(rules[0]["mode"], "auto");
    assert_eq!(rules[0]["enabled"], true);
}

#[test]
fn delete_recurring_rule_cascades_to_scheduled_jobs() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let added = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_recurring_rule",
        &json!({
            "label": "Gym",
            "amount": 30.0,
            "category": "Personal Care",
            "frequency": "monthly",
            "anchor_day": 1,
        }),
    ));
    let rule_id = added["rule_id"].as_i64().unwrap();
    parse_ok(execute(
        &conn,
        &ctx,
        "tu_2",
        "delete_recurring_rule",
        &json!({ "rule_id": rule_id }),
    ));
    let job_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM scheduled_jobs WHERE kind = 'recurring_expense' \
             AND json_extract(payload, '$.rule_id') = ?1",
            [rule_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        job_count, 0,
        "delete_recurring_rule must drop the scheduler queue rows too"
    );
}

#[test]
fn pause_recurring_rule_flips_enabled_flag() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    let added = parse_ok(execute(
        &conn,
        &ctx,
        "tu_1",
        "add_recurring_rule",
        &json!({
            "label": "Gym",
            "amount": 30.0,
            "category": "Personal Care",
            "frequency": "monthly",
            "anchor_day": 1,
        }),
    ));
    let rule_id = added["rule_id"].as_i64().unwrap();
    parse_ok(execute(
        &conn,
        &ctx,
        "tu_2",
        "pause_recurring_rule",
        &json!({ "rule_id": rule_id, "enabled": false }),
    ));
    let enabled: i64 = conn
        .query_row(
            "SELECT enabled FROM recurring_rules WHERE id = ?1",
            [rule_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(enabled, 0);
}

#[test]
fn query_expenses_total_subtracts_refunds() {
    let conn = fresh_db();
    let ctx = ctx_solo();
    parse_ok(execute(
        &conn,
        &ctx,
        "tu_e",
        "add_expense",
        &json!({ "amount": 100.0, "category": "Groceries" }),
    ));
    parse_ok(execute(
        &conn,
        &ctx,
        "tu_r",
        "add_refund",
        &json!({ "amount": 25.0, "category": "Groceries" }),
    ));
    let q = parse_ok(execute(
        &conn,
        &ctx,
        "tu_q",
        "query_expenses",
        &json!({ "category": "Groceries" }),
    ));
    assert_eq!(q["count"], 2, "both rows visible to query");
    assert_eq!(q["total_cents"], 7_500, "net total subtracts the refund");
}
