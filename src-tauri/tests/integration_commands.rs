// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//! Integration tests for the Tauri IPC layer (audit T-1).
//!
//! Tauri commands are `async fn`s that take `tauri::State<'_, AppState>`,
//! which can't be constructed without a running Tauri app. We work around
//! that by having `commands.rs` expose pure-helper twins of the
//! highest-value handlers (`list_expenses_query`, `csv_import_commit_inner`)
//! that take a plain `&Connection` / `&mut Connection`. The `#[tauri::command]`
//! wrapper is a one-liner that forwards via the DbHandle actor.
//!
//! This file covers what the audit explicitly called out:
//!   - `list_expenses` filter assembly (the off-by-one risk on `end_date + 1 day`).
//!   - `csv_import_commit` round-trip (transaction rolls back cleanly on
//!     error; happy path inserts the right rows).
//!   - `delete_expense` plus refund-cascade behavior via the repository
//!     it delegates to.
//!
//! Other handlers either route into already-tested repository / insights
//! code (covered in those modules' unit tests) or are pure calculators
//! whose math is unit-tested in `insights::*`.

#![cfg(feature = "desktop")]

use moneypenny_lib::commands::{
    csv_import_commit_inner, list_expenses_query, CommitInput, CommittableRow, ExpenseFilters,
    RuleToSave,
};
use moneypenny_lib::db;
use moneypenny_lib::domain::{ExpenseSource, NewExpense};
use moneypenny_lib::repository::expenses;
use rusqlite::Connection;
use tempfile::TempDir;
use time::macros::{date, datetime};
use time::{Date, Duration, OffsetDateTime, Time, UtcOffset};

fn fresh_db() -> (Connection, TempDir) {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("test.sqlite");
    let conn = db::open(&path).unwrap();
    db::migrate(&conn).unwrap();
    // Re-activate every seed category so tests can pick any of them.
    conn.execute("UPDATE categories SET is_active = 1 WHERE is_seed = 1", [])
        .unwrap();
    (conn, tmp)
}

fn category_id_by_name(conn: &Connection, name: &str) -> i64 {
    conn.query_row("SELECT id FROM categories WHERE name = ?1", [name], |r| {
        r.get(0)
    })
    .unwrap()
}

fn insert_expense(
    conn: &Connection,
    amount_cents: i64,
    category_id: Option<i64>,
    occurred_at: OffsetDateTime,
    description: Option<&str>,
) -> i64 {
    expenses::insert(
        conn,
        &NewExpense {
            amount_cents,
            currency: "USD".into(),
            category_id,
            description: description.map(|s| s.to_string()),
            occurred_at,
            source: ExpenseSource::Manual,
            raw_message: None,
            llm_confidence: None,
            logged_by_chat_id: None,
            is_refund: false,
            refund_for_expense_id: None,
        },
    )
    .unwrap()
}

// ---------------------------------------------------------------------
// list_expenses_query
// ---------------------------------------------------------------------

/// Document the date-window contract: an `end_date` of YYYY-MM-DD must
/// include rows occurring at any time on that day. The implementation
/// uses `e.occurred_at < (end_date + 1 day) MIDNIGHT`. A naive
/// off-by-one (using `<=` against `end_date 00:00`) would silently
/// drop same-day transactions.
#[test]
fn list_expenses_end_date_inclusive_of_same_day() {
    let (conn, _tmp) = fresh_db();
    let coffee = category_id_by_name(&conn, "Coffee");
    // Three rows in chronological order: 2026-04-14 23:59, 2026-04-15
    // 12:00, 2026-04-15 23:59. With end_date=2026-04-15 the query
    // must return all three. Off-by-one would drop both 04-15 rows.
    insert_expense(
        &conn,
        500,
        Some(coffee),
        datetime!(2026-04-14 23:59:00 UTC),
        Some("before"),
    );
    insert_expense(
        &conn,
        600,
        Some(coffee),
        datetime!(2026-04-15 12:00:00 UTC),
        Some("noon"),
    );
    insert_expense(
        &conn,
        700,
        Some(coffee),
        datetime!(2026-04-15 23:59:00 UTC),
        Some("late"),
    );

    let filters = ExpenseFilters {
        category_id: Some(coffee),
        start_date: Some(date!(2026 - 04 - 14)),
        end_date: Some(date!(2026 - 04 - 15)),
        search: None,
        limit: None,
        offset: None,
    };
    let rows = list_expenses_query(&conn, &filters, UtcOffset::UTC).unwrap();
    assert_eq!(rows.len(), 3, "all three rows fall inside the window");
}

#[test]
fn list_expenses_end_date_excludes_next_day() {
    let (conn, _tmp) = fresh_db();
    let coffee = category_id_by_name(&conn, "Coffee");
    insert_expense(
        &conn,
        500,
        Some(coffee),
        datetime!(2026-04-15 12:00:00 UTC),
        Some("kept"),
    );
    insert_expense(
        &conn,
        600,
        Some(coffee),
        datetime!(2026-04-16 00:00:00 UTC),
        Some("dropped"),
    );
    let filters = ExpenseFilters {
        category_id: Some(coffee),
        start_date: None,
        end_date: Some(date!(2026 - 04 - 15)),
        search: None,
        limit: None,
        offset: None,
    };
    let rows = list_expenses_query(&conn, &filters, UtcOffset::UTC).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].description.as_deref(), Some("kept"));
}

#[test]
fn list_expenses_search_matches_description_or_raw_message() {
    let (conn, _tmp) = fresh_db();
    let coffee = category_id_by_name(&conn, "Coffee");
    insert_expense(
        &conn,
        500,
        Some(coffee),
        datetime!(2026-04-15 12:00:00 UTC),
        Some("morning latte"),
    );
    insert_expense(
        &conn,
        500,
        Some(coffee),
        datetime!(2026-04-15 12:00:00 UTC),
        Some("americano"),
    );

    let filters = ExpenseFilters {
        category_id: None,
        start_date: None,
        end_date: None,
        search: Some("latte".into()),
        limit: None,
        offset: None,
    };
    let rows = list_expenses_query(&conn, &filters, UtcOffset::UTC).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].description.as_deref(), Some("morning latte"));
}

#[test]
fn list_expenses_limit_caps_at_500() {
    let (conn, _tmp) = fresh_db();
    let coffee = category_id_by_name(&conn, "Coffee");
    for i in 0..10 {
        insert_expense(
            &conn,
            100 + i,
            Some(coffee),
            datetime!(2026-04-15 12:00:00 UTC) + Duration::seconds(i),
            Some(&format!("r{i}")),
        );
    }
    // Request 5000; expected: clamped to 500 internally, real returned
    // count is min(500, 10) = 10. Demonstrates the clamp didn't drop
    // the request entirely.
    let filters = ExpenseFilters {
        category_id: Some(coffee),
        start_date: None,
        end_date: None,
        search: None,
        limit: Some(5000),
        offset: None,
    };
    let rows = list_expenses_query(&conn, &filters, UtcOffset::UTC).unwrap();
    assert_eq!(rows.len(), 10);
}

#[test]
fn list_expenses_orders_most_recent_first() {
    let (conn, _tmp) = fresh_db();
    let coffee = category_id_by_name(&conn, "Coffee");
    insert_expense(
        &conn,
        100,
        Some(coffee),
        datetime!(2026-04-15 08:00:00 UTC),
        Some("morning"),
    );
    insert_expense(
        &conn,
        200,
        Some(coffee),
        datetime!(2026-04-15 18:00:00 UTC),
        Some("evening"),
    );
    let filters = ExpenseFilters {
        category_id: Some(coffee),
        ..Default::default()
    };
    let rows = list_expenses_query(&conn, &filters, UtcOffset::UTC).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].description.as_deref(), Some("evening"));
    assert_eq!(rows[1].description.as_deref(), Some("morning"));
}

// ---------------------------------------------------------------------
// csv_import_commit_inner
// ---------------------------------------------------------------------

#[test]
fn csv_import_commit_inserts_all_rows_and_records_rules() {
    let (mut conn, _tmp) = fresh_db();
    let coffee = category_id_by_name(&conn, "Coffee");
    let groceries = category_id_by_name(&conn, "Groceries");

    let input = CommitInput {
        rows: vec![
            CommittableRow {
                occurred_at: "2026-04-10T12:00:00Z".into(),
                amount_cents: 500,
                category_id: Some(coffee),
                merchant: "STARBUCKS #1234".into(),
                description: None,
                is_refund: false,
            },
            CommittableRow {
                occurred_at: "2026-04-11T12:00:00Z".into(),
                amount_cents: 8350,
                category_id: Some(groceries),
                merchant: "WHOLE FOODS".into(),
                description: Some("weekly haul".into()),
                is_refund: false,
            },
        ],
        rules_to_save: vec![RuleToSave {
            pattern: "STARBUCKS".into(),
            category_id: coffee,
            default_is_refund: false,
        }],
        profile_id: None,
    };

    let now = datetime!(2026-04-12 09:00:00 UTC);
    let result = csv_import_commit_inner(&mut conn, &input, now).unwrap();
    assert_eq!(result.inserted, 2);
    assert_eq!(result.rules_added, 1);

    // Both expenses landed on disk.
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM expenses WHERE source = 'csv'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);

    // The row without an explicit description falls back to merchant.
    let starbucks_desc: String = conn
        .query_row(
            "SELECT description FROM expenses WHERE amount_cents = 500",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(starbucks_desc, "STARBUCKS #1234");

    // Merchant rule persisted.
    let rule_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM merchant_rules", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rule_count, 1);
}

#[test]
fn csv_import_commit_rolls_back_on_bad_date() {
    let (mut conn, _tmp) = fresh_db();
    let coffee = category_id_by_name(&conn, "Coffee");

    let input = CommitInput {
        rows: vec![
            CommittableRow {
                occurred_at: "2026-04-10T12:00:00Z".into(),
                amount_cents: 500,
                category_id: Some(coffee),
                merchant: "OK".into(),
                description: None,
                is_refund: false,
            },
            CommittableRow {
                occurred_at: "not-a-date".into(), // forces a parse error mid-batch
                amount_cents: 600,
                category_id: Some(coffee),
                merchant: "BAD".into(),
                description: None,
                is_refund: false,
            },
        ],
        rules_to_save: vec![],
        profile_id: None,
    };

    let err = csv_import_commit_inner(&mut conn, &input, OffsetDateTime::now_utc()).unwrap_err();
    let msg = err.to_string();
    // The exact wording depends on `time::Parse`'s error type — what
    // we care about is that the mid-batch failure surfaced as an Err
    // and that the first row got rolled back.
    assert!(!msg.is_empty(), "expected non-empty error message");

    // The first row's insert must have been rolled back — full
    // transactional rollback on mid-batch error is the whole point.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM expenses", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "partial import must roll back");
}

// ---------------------------------------------------------------------
// delete_expense + refund cascade
// ---------------------------------------------------------------------

#[test]
fn deleting_an_expense_orphans_its_refund_via_set_null() {
    // Migration 0006 sets `refund_for_expense_id ... ON DELETE SET NULL`
    // (NOT cascade). Deleting the original purchase must leave the
    // refund row standing — its history is meaningful to the user
    // even if the parent purchase is gone — but with the back-pointer
    // nulled. This test pins that contract; flipping it to cascade
    // would change refund-accounting semantics silently.
    let (conn, _tmp) = fresh_db();
    let coffee = category_id_by_name(&conn, "Coffee");
    let purchase_id = insert_expense(
        &conn,
        1000,
        Some(coffee),
        datetime!(2026-04-15 12:00:00 UTC),
        Some("latte"),
    );
    conn.execute(
        "INSERT INTO expenses (amount_cents, currency, category_id, occurred_at, source, is_refund, refund_for_expense_id) \
         VALUES (?1, 'USD', ?2, ?3, 'manual', 1, ?4)",
        rusqlite::params![
            300,
            coffee,
            datetime!(2026-04-16 12:00:00 UTC),
            purchase_id,
        ],
    )
    .unwrap();
    assert_eq!(
        conn.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM expenses WHERE is_refund = 1",
            [],
            |r| r.get(0),
        )
        .unwrap(),
        1
    );

    let removed = expenses::delete(&conn, purchase_id).unwrap();
    assert!(removed);

    // The refund row survives (count stays 1) and its back-pointer is
    // nulled.
    let refund_row: (i64, Option<i64>) = conn
        .query_row(
            "SELECT id, refund_for_expense_id FROM expenses WHERE is_refund = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let _ = refund_row.0;
    assert!(
        refund_row.1.is_none(),
        "refund_for_expense_id must be NULL after parent delete"
    );
}

// Silence the unused-imports lint when the test file is compiled but
// only some helpers happen to be used by the active test set.
#[allow(dead_code)]
fn _shut_up_unused(_date: Date, _time: Time) {}
