//! Integration tests for the dashboard aggregation queries.

use moneypenny_lib::db;
use moneypenny_lib::domain::{CategoryKind, ExpenseSource, NewExpense};
use moneypenny_lib::insights::{dashboard, DateRange};
use moneypenny_lib::repository::{categories, expenses};
use rusqlite::Connection;
use time::macros::{date, datetime};
use time::OffsetDateTime;

fn fresh_db() -> Connection {
    let conn = db::open_in_memory().unwrap();
    db::migrate(&conn).unwrap();
    conn
}

/// Seed an authorized chat. Returns the chat_id.
fn seed_chat(conn: &Connection, chat_id: i64, name: &str, owner: bool) -> i64 {
    let role = if owner { "owner" } else { "member" };
    conn.execute(
        "INSERT INTO telegram_authorized_chats (chat_id, display_name, role) VALUES (?1, ?2, ?3)",
        rusqlite::params![chat_id, name, role],
    )
    .unwrap();
    chat_id
}

/// Look up the seed Coffee / Rent / Dining categories.
fn cat_id(conn: &Connection, name: &str) -> i64 {
    categories::get_by_name(conn, name).unwrap().unwrap().id
}

/// Set monthly_target_cents on a category for use as the budget figure
/// the period-pacing math uses.
fn set_target(conn: &Connection, name: &str, cents: i64) {
    let id = cat_id(conn, name);
    categories::set_monthly_target(conn, id, Some(cents)).unwrap();
}

fn add_expense(
    conn: &Connection,
    cents: i64,
    category: &str,
    occurred_at: OffsetDateTime,
    chat_id: Option<i64>,
) {
    let cid = cat_id(conn, category);
    expenses::insert(
        conn,
        &NewExpense {
            amount_cents: cents,
            currency: "USD".into(),
            category_id: Some(cid),
            description: Some(format!("test {category}")),
            occurred_at,
            source: ExpenseSource::Telegram,
            raw_message: None,
            llm_confidence: None,
            logged_by_chat_id: chat_id,
        },
    )
    .unwrap();
}

#[test]
fn empty_db_dashboard_is_zeroed_for_each_range() {
    let conn = fresh_db();
    let now = datetime!(2026-04-15 12:00:00 UTC);

    for range in [
        DateRange::ThisWeek,
        DateRange::ThisMonth,
        DateRange::ThisQuarter,
        DateRange::ThisYear,
        DateRange::Ytd,
    ] {
        let snap = dashboard(&conn, range, now).unwrap();
        assert_eq!(snap.kpi.total_spent_cents, 0, "range {range:?}");
        assert!(
            snap.category_totals.is_empty(),
            "no spend → no category rows"
        );
        assert!(snap.member_spend.is_empty());
        assert!(snap.top_expenses.is_empty());
    }
}

#[test]
fn rent_on_day_two_scenario_dashboard_says_on_pace() {
    let conn = fresh_db();
    set_target(&conn, "Rent / Mortgage", 150_000); // $1500
    set_target(&conn, "Coffee", 8_000); // $80
    set_target(&conn, "Groceries", 50_000); // $500
    set_target(&conn, "Dining Out", 22_000); // $220
    // Variable budget total = $80 + $500 + $220 = $800

    // Day 2 of April
    let now = datetime!(2026-04-02 12:00:00 UTC);
    add_expense(
        &conn,
        150_000,
        "Rent / Mortgage",
        datetime!(2026-04-01 09:00:00 UTC),
        None,
    );
    add_expense(
        &conn,
        2_000,
        "Coffee",
        datetime!(2026-04-02 08:00:00 UTC),
        None,
    );
    add_expense(
        &conn,
        1_000,
        "Coffee",
        datetime!(2026-04-02 11:30:00 UTC),
        None,
    );

    let snap = dashboard(&conn, DateRange::ThisMonth, now).unwrap();
    let period = snap.period.expect("ThisMonth always populates period");

    assert_eq!(period.fixed_actual_cents, 150_000);
    assert_eq!(period.variable_spent_cents, 3_000);
    assert!(
        period.on_pace,
        "$30 of variable spend on day 2 of $800 budget should be ON pace"
    );
    assert!(snap.kpi.on_pace, "KPI mirrors period.on_pace");
    assert_eq!(snap.kpi.variable_remaining_cents, 80_000 - 3_000);
}

#[test]
fn over_budget_detected() {
    let conn = fresh_db();
    set_target(&conn, "Coffee", 5_000); // $50/month coffee budget

    let now = datetime!(2026-04-15 12:00:00 UTC);
    // Spend $80 on coffee — over the $50 target
    add_expense(
        &conn,
        8_000,
        "Coffee",
        datetime!(2026-04-10 09:00:00 UTC),
        None,
    );

    let snap = dashboard(&conn, DateRange::ThisMonth, now).unwrap();
    let coffee_overage = snap
        .over_budget
        .iter()
        .find(|c| c.name == "Coffee")
        .expect("coffee should be over budget");
    assert_eq!(coffee_overage.spent_cents, 8_000);
    assert_eq!(coffee_overage.target_cents, 5_000);
    assert_eq!(coffee_overage.overage_cents, 3_000);
}

#[test]
fn per_member_attribution() {
    let conn = fresh_db();
    let wyatt = seed_chat(&conn, 111, "Wyatt", true);
    let spouse = seed_chat(&conn, 222, "Spouse", false);

    let now = datetime!(2026-04-15 12:00:00 UTC);
    add_expense(
        &conn,
        2_000,
        "Coffee",
        datetime!(2026-04-05 09:00:00 UTC),
        Some(wyatt),
    );
    add_expense(
        &conn,
        15_000,
        "Groceries",
        datetime!(2026-04-08 18:00:00 UTC),
        Some(spouse),
    );
    add_expense(
        &conn,
        4_000,
        "Coffee",
        datetime!(2026-04-12 09:00:00 UTC),
        Some(spouse),
    );

    let snap = dashboard(&conn, DateRange::ThisMonth, now).unwrap();
    assert_eq!(snap.member_spend.len(), 2);
    let by_name: std::collections::HashMap<_, _> = snap
        .member_spend
        .iter()
        .map(|m| (m.display_name.as_str(), m.total_cents))
        .collect();
    assert_eq!(by_name.get("Wyatt"), Some(&2_000));
    assert_eq!(by_name.get("Spouse"), Some(&19_000));
    // Sorted by total descending
    assert_eq!(snap.member_spend[0].display_name, "Spouse");
}

#[test]
fn daily_trend_buckets_separately_for_fixed_and_variable() {
    let conn = fresh_db();
    let now = datetime!(2026-04-15 12:00:00 UTC);

    add_expense(
        &conn,
        150_000,
        "Rent / Mortgage",
        datetime!(2026-04-01 09:00:00 UTC),
        None,
    );
    add_expense(
        &conn,
        2_000,
        "Coffee",
        datetime!(2026-04-01 14:00:00 UTC),
        None,
    );
    add_expense(
        &conn,
        3_000,
        "Coffee",
        datetime!(2026-04-05 14:00:00 UTC),
        None,
    );

    let snap = dashboard(&conn, DateRange::ThisMonth, now).unwrap();
    // April has 30 days → 30 daily buckets, all populated (zero-filled gaps).
    assert_eq!(snap.daily_trend.len(), 30);

    let day1 = snap
        .daily_trend
        .iter()
        .find(|p| p.date == date!(2026 - 04 - 01))
        .unwrap();
    assert_eq!(day1.fixed_cents, 150_000);
    assert_eq!(day1.variable_cents, 2_000);

    let day5 = snap
        .daily_trend
        .iter()
        .find(|p| p.date == date!(2026 - 04 - 05))
        .unwrap();
    assert_eq!(day5.fixed_cents, 0);
    assert_eq!(day5.variable_cents, 3_000);
}

#[test]
fn upcoming_fixed_excludes_already_paid_categories() {
    let conn = fresh_db();
    // Set rent recurrence to day 1; already paid this month
    let rent_id = cat_id(&conn, "Rent / Mortgage");
    conn.execute(
        "UPDATE categories SET recurrence_day_of_month = 1 WHERE id = ?1",
        rusqlite::params![rent_id],
    )
    .unwrap();

    // Set Internet recurrence to day 25; NOT yet paid
    let net_id = cat_id(&conn, "Internet");
    conn.execute(
        "UPDATE categories SET recurrence_day_of_month = 25 WHERE id = ?1",
        rusqlite::params![net_id],
    )
    .unwrap();

    let now = datetime!(2026-04-10 12:00:00 UTC);
    add_expense(
        &conn,
        150_000,
        "Rent / Mortgage",
        datetime!(2026-04-01 09:00:00 UTC),
        None,
    );

    let snap = dashboard(&conn, DateRange::ThisMonth, now).unwrap();
    let names: Vec<_> = snap.upcoming_fixed.iter().map(|u| u.name.as_str()).collect();
    assert!(
        !names.contains(&"Rent / Mortgage"),
        "rent paid → should not be in upcoming"
    );
    assert!(
        names.contains(&"Internet"),
        "internet not yet paid → should appear"
    );
}

#[test]
fn mom_comparison_computes_delta() {
    let conn = fresh_db();

    // Today: April 15 — 15 days into April
    let now = datetime!(2026-04-15 12:00:00 UTC);

    // April spend (variable) so far: $100
    add_expense(
        &conn,
        10_000,
        "Coffee",
        datetime!(2026-04-10 09:00:00 UTC),
        None,
    );

    // Last month (March), first 15 days: $50
    add_expense(
        &conn,
        5_000,
        "Coffee",
        datetime!(2026-03-08 09:00:00 UTC),
        None,
    );
    // March 20 expense should NOT count (past day-15 cap)
    add_expense(
        &conn,
        20_000,
        "Coffee",
        datetime!(2026-03-20 09:00:00 UTC),
        None,
    );

    let snap = dashboard(&conn, DateRange::ThisMonth, now).unwrap();
    let mom = snap.mom_comparison.expect("ThisMonth produces MoM");
    assert_eq!(mom.variable_spent_this_period_cents, 10_000);
    assert_eq!(mom.variable_spent_same_point_last_month_cents, 5_000);
    assert_eq!(mom.delta_cents, 5_000);
    assert_eq!(mom.delta_pct, Some(100.0)); // doubled
}

#[test]
fn this_week_only_includes_within_week() {
    let conn = fresh_db();
    let now = datetime!(2026-04-28 12:00:00 UTC); // Tuesday

    // Last week (week of April 20-26): $50
    add_expense(
        &conn,
        5_000,
        "Coffee",
        datetime!(2026-04-23 09:00:00 UTC),
        None,
    );
    // This week (Mon Apr 27 onward): $20
    add_expense(
        &conn,
        2_000,
        "Coffee",
        datetime!(2026-04-27 09:00:00 UTC),
        None,
    );

    let snap = dashboard(&conn, DateRange::ThisWeek, now).unwrap();
    assert_eq!(snap.kpi.total_spent_cents, 2_000);
    // No period snapshot for non-month ranges
    assert!(snap.period.is_none());
    // No MoM for non-month ranges
    assert!(snap.mom_comparison.is_none());
}

#[test]
fn this_quarter_aggregates_three_months() {
    let conn = fresh_db();
    let now = datetime!(2026-05-15 12:00:00 UTC); // Q2

    add_expense(
        &conn,
        1_000,
        "Coffee",
        datetime!(2026-04-01 09:00:00 UTC),
        None,
    );
    add_expense(
        &conn,
        2_000,
        "Coffee",
        datetime!(2026-05-01 09:00:00 UTC),
        None,
    );
    // June not yet (now is May 15) but if it were it'd count
    add_expense(
        &conn,
        4_000,
        "Coffee",
        datetime!(2026-03-31 23:00:00 UTC), // Q1 — should be excluded
        None,
    );

    let snap = dashboard(&conn, DateRange::ThisQuarter, now).unwrap();
    assert_eq!(snap.kpi.total_spent_cents, 3_000); // April + May only
}

#[test]
fn custom_range_inclusive_endpoints() {
    let conn = fresh_db();
    let now = datetime!(2026-04-15 12:00:00 UTC);

    add_expense(
        &conn,
        1_000,
        "Coffee",
        datetime!(2026-03-01 09:00:00 UTC),
        None,
    ); // first day in
    add_expense(
        &conn,
        2_000,
        "Coffee",
        datetime!(2026-03-31 23:00:00 UTC),
        None,
    ); // last day in
    add_expense(
        &conn,
        4_000,
        "Coffee",
        datetime!(2026-04-01 00:00:00 UTC),
        None,
    ); // April — out

    let range = DateRange::Custom {
        from: date!(2026 - 03 - 01),
        to: date!(2026 - 03 - 31),
    };
    let snap = dashboard(&conn, range, now).unwrap();
    assert_eq!(snap.kpi.total_spent_cents, 3_000);
}

#[test]
fn top_expenses_ordered_by_amount_descending() {
    let conn = fresh_db();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    for (cents, day) in [(2_000, 3), (15_000, 5), (500, 7), (8_000, 11)] {
        let occ = OffsetDateTime::from_unix_timestamp(
            datetime!(2026-04-01 00:00:00 UTC).unix_timestamp() + day * 86_400,
        )
        .unwrap();
        add_expense(&conn, cents, "Coffee", occ, None);
    }
    let snap = dashboard(&conn, DateRange::ThisMonth, now).unwrap();
    let amounts: Vec<i64> = snap.top_expenses.iter().map(|e| e.amount_cents).collect();
    assert_eq!(amounts, vec![15_000, 8_000, 2_000, 500]);
}
