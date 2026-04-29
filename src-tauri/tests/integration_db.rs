//! Integration tests for the database layer: migrations, seed data, CRUD,
//! and foreign-key enforcement.

use moneypenny_lib::db;
use moneypenny_lib::domain::{
    BudgetPeriod, CategoryKind, ExpenseSource, NewBudget, NewCategory, NewExpense,
};
use moneypenny_lib::repository::{budgets, categories, expenses};
use rusqlite::params;
use time::macros::datetime;

fn fresh_db() -> rusqlite::Connection {
    let conn = db::open_in_memory().expect("open in-memory db");
    db::migrate(&conn).expect("apply migrations");
    conn
}

#[test]
fn migrations_are_idempotent() {
    let conn = fresh_db();
    // Apply twice — should be a no-op the second time.
    db::migrate(&conn).expect("second migrate run");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 4, "after migrations, user_version should be 4");
}

#[test]
fn seed_categories_loaded_and_marked() {
    let conn = fresh_db();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM categories WHERE is_seed = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        count >= 25,
        "expected at least 25 seed categories, got {count}"
    );

    // Both kinds present
    let fixed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM categories WHERE kind = 'fixed' AND is_seed = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let variable: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM categories WHERE kind = 'variable' AND is_seed = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(fixed > 0);
    assert!(variable > 0);
}

#[test]
fn category_kind_check_constraint_blocks_invalid_value() {
    let conn = fresh_db();
    let err = conn.execute(
        "INSERT INTO categories (name, kind) VALUES ('Bogus', 'gaseous')",
        [],
    );
    assert!(err.is_err(), "CHECK should block invalid kind");
}

#[test]
fn insert_and_retrieve_expense_round_trips() {
    let conn = fresh_db();
    let coffee = categories::get_by_name(&conn, "Coffee")
        .unwrap()
        .expect("seed Coffee category exists");

    let new_id = expenses::insert(
        &conn,
        &NewExpense {
            amount_cents: 500,
            currency: "USD".into(),
            category_id: Some(coffee.id),
            description: Some("morning latte".into()),
            occurred_at: datetime!(2026-04-15 09:00:00 UTC),
            source: ExpenseSource::Telegram,
            raw_message: Some("$5 coffee".into()),
            llm_confidence: Some(0.95),
            logged_by_chat_id: None,
        },
    )
    .unwrap();

    let got = expenses::get(&conn, new_id).unwrap().unwrap();
    assert_eq!(got.amount_cents, 500);
    assert_eq!(got.description.as_deref(), Some("morning latte"));
    assert_eq!(got.source, ExpenseSource::Telegram);
    assert_eq!(got.category_id, Some(coffee.id));
}

#[test]
fn delete_expense_returns_false_when_missing() {
    let conn = fresh_db();
    assert!(!expenses::delete(&conn, 999_999).unwrap());
}

#[test]
fn list_in_range_excludes_outside_dates() {
    let conn = fresh_db();
    let coffee = categories::get_by_name(&conn, "Coffee").unwrap().unwrap();
    for occ in [
        datetime!(2026-03-31 23:59:00 UTC), // just before April
        datetime!(2026-04-01 00:00:00 UTC), // April start
        datetime!(2026-04-15 12:00:00 UTC), // mid-April
        datetime!(2026-05-01 00:00:00 UTC), // May start (excluded)
    ] {
        expenses::insert(
            &conn,
            &NewExpense {
                amount_cents: 100,
                currency: "USD".into(),
                category_id: Some(coffee.id),
                description: None,
                occurred_at: occ,
                source: ExpenseSource::Manual,
                raw_message: None,
                llm_confidence: None,
                logged_by_chat_id: None,
            },
        )
        .unwrap();
    }
    let april = expenses::list_in_range(
        &conn,
        datetime!(2026-04-01 00:00:00 UTC),
        datetime!(2026-05-01 00:00:00 UTC),
    )
    .unwrap();
    assert_eq!(april.len(), 2, "exactly the two April rows fall in range");
}

#[test]
fn category_target_can_be_updated() {
    let conn = fresh_db();
    let groceries = categories::get_by_name(&conn, "Groceries")
        .unwrap()
        .unwrap();
    categories::set_monthly_target(&conn, groceries.id, Some(40_000)).unwrap();
    let after = categories::get(&conn, groceries.id).unwrap().unwrap();
    assert_eq!(after.monthly_target_cents, Some(40_000));
}

#[test]
fn cannot_hard_delete_seed_category() {
    let conn = fresh_db();
    let coffee = categories::get_by_name(&conn, "Coffee").unwrap().unwrap();
    let res = categories::delete(&conn, coffee.id);
    assert!(res.is_err(), "seed categories must not be hard-deletable");
}

#[test]
fn budget_round_trips() {
    let conn = fresh_db();
    let dining = categories::get_by_name(&conn, "Dining Out")
        .unwrap()
        .unwrap();
    let id = budgets::insert(
        &conn,
        &NewBudget {
            category_id: dining.id,
            amount_cents: 30_000,
            period: BudgetPeriod::Monthly,
            effective_from: datetime!(2026-04-01 00:00:00 UTC),
            effective_to: None,
        },
    )
    .unwrap();

    let active = budgets::effective_at(&conn, dining.id, datetime!(2026-04-15 12:00:00 UTC))
        .unwrap()
        .unwrap();
    assert_eq!(active.id, id);
    assert_eq!(active.amount_cents, 30_000);
}

#[test]
fn deleting_category_cascades_to_budgets_and_nulls_expenses() {
    let conn = fresh_db();
    // User-created (non-seed) category
    let user_cat_id = categories::insert(
        &conn,
        &NewCategory {
            name: "Toy Tax".into(),
            kind: CategoryKind::Variable,
            monthly_target_cents: None,
            is_recurring: false,
            recurrence_day_of_month: None,
        },
    )
    .unwrap();

    let exp_id = expenses::insert(
        &conn,
        &NewExpense {
            amount_cents: 100,
            currency: "USD".into(),
            category_id: Some(user_cat_id),
            description: None,
            occurred_at: datetime!(2026-04-15 09:00:00 UTC),
            source: ExpenseSource::Manual,
            raw_message: None,
            llm_confidence: None,
            logged_by_chat_id: None,
        },
    )
    .unwrap();

    let budget_id = budgets::insert(
        &conn,
        &NewBudget {
            category_id: user_cat_id,
            amount_cents: 10_000,
            period: BudgetPeriod::Monthly,
            effective_from: datetime!(2026-04-01 00:00:00 UTC),
            effective_to: None,
        },
    )
    .unwrap();

    assert!(categories::delete(&conn, user_cat_id).unwrap());

    // Expense's category_id should be NULL (ON DELETE SET NULL)
    let after = expenses::get(&conn, exp_id).unwrap().unwrap();
    assert_eq!(after.category_id, None);

    // Budget should be gone (ON DELETE CASCADE)
    let still_there: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM budgets WHERE id = ?1",
            params![budget_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(still_there, 0);
}
