// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//! Dashboard aggregation queries.
//!
//! All amounts are integer cents. Bucketing of expenses into daily
//! buckets is done in Rust against the user's offset, not via SQLite's
//! timezone-aware functions, so behavior is identical regardless of
//! the host machine's system timezone.

pub mod category_analyzer;
pub mod debt;
pub mod forecast;
pub mod monte_carlo;
pub mod range;
pub mod report;
pub mod simulator;
pub mod stats;

use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use time::{Date, Duration, Month, OffsetDateTime, Time};

use crate::domain::{
    compute_snapshot, current_month_bounds, CategoryKind, Expense, PeriodSnapshot,
};
use crate::repository::expenses::{self, SIGNED_AMOUNT_SQL};

pub use range::DateRange;

#[derive(Debug, Serialize)]
pub struct DashboardSnapshot {
    pub range: DateRange,
    #[serde(with = "time::serde::rfc3339")]
    pub start: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub end: OffsetDateTime,
    /// Only populated when `range == ThisMonth`. The bot reuses this same
    /// computation so dashboard and bot answers cannot diverge.
    pub period: Option<PeriodSnapshot>,
    pub kpi: KpiCard,
    pub category_totals: Vec<CategoryTotal>,
    pub daily_trend: Vec<DailyTrendPoint>,
    pub fixed_vs_variable: FixedVariableBreakdown,
    pub member_spend: Vec<MemberSpend>,
    pub top_expenses: Vec<Expense>,
    pub over_budget: Vec<OverBudgetCategory>,
    pub upcoming_fixed: Vec<UpcomingFixed>,
    pub mom_comparison: Option<MoMComparison>,
}

#[derive(Debug, Serialize)]
pub struct KpiCard {
    pub variable_remaining_cents: i64,
    pub daily_variable_allowance_cents: i64,
    /// Spend across active fixed + variable categories. Investing spend is
    /// excluded so this stays the spend-side mirror of `total_budget_cents`
    /// (which also excludes investing); `total_remaining_cents` is their
    /// difference and so can never dip below `variable_remaining_cents`
    /// unless fixed spending itself exceeds the fixed budget.
    pub total_spent_cents: i64,
    pub days_remaining: u8,
    pub on_pace: bool,
    /// Sum of `monthly_target_cents` across active fixed + variable
    /// categories. Investing targets are excluded — they're savings
    /// goals, not a spending allowance. Zero when range is not
    /// monthly-shaped.
    pub total_budget_cents: i64,
    /// `total_budget_cents - total_spent_cents`. Can go negative when
    /// the user is over total budget. Zero when range is not monthly.
    pub total_remaining_cents: i64,
    /// Sum of `monthly_target_cents` across active *variable*
    /// categories. Used by the dashboard's variable-trajectory chart
    /// to draw the budget cap line. Zero when range is not monthly.
    pub variable_budget_cents: i64,
    /// Sum of `monthly_target_cents` across active *fixed* categories.
    /// Provided for completeness alongside `variable_budget_cents`.
    pub fixed_budget_cents: i64,
}

#[derive(Debug, Serialize)]
pub struct CategoryTotal {
    pub category_id: i64,
    pub name: String,
    pub kind: CategoryKind,
    pub total_cents: i64,
    /// Monthly target on the category, if any. The dashboard's per-category
    /// bar chart uses this to switch colors: fixed/variable turn orange
    /// when `total > target`, investing turns deep green when met/exceeded.
    pub monthly_target_cents: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct DailyTrendPoint {
    pub date: Date,
    pub fixed_cents: i64,
    pub variable_cents: i64,
}

#[derive(Debug, Serialize)]
pub struct FixedVariableBreakdown {
    pub fixed_committed_cents: i64,
    pub variable_spent_cents: i64,
    pub variable_remaining_cents: i64,
}

#[derive(Debug, Serialize)]
pub struct MemberSpend {
    pub chat_id: i64,
    pub display_name: String,
    pub total_cents: i64,
}

#[derive(Debug, Serialize)]
pub struct OverBudgetCategory {
    pub category_id: i64,
    pub name: String,
    pub spent_cents: i64,
    pub target_cents: i64,
    pub overage_cents: i64,
}

#[derive(Debug, Serialize)]
pub struct UpcomingFixed {
    pub category_id: i64,
    pub name: String,
    pub recurrence_day_of_month: u8,
    pub expected_amount_cents: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct MoMComparison {
    pub variable_spent_this_period_cents: i64,
    pub variable_spent_same_point_last_month_cents: i64,
    pub delta_cents: i64,
    /// Percentage change vs. last month (positive = spending more).
    /// `None` when last-month figure is zero.
    pub delta_pct: Option<f64>,
}

/// Compose the full dashboard snapshot for the given range.
pub fn dashboard(
    conn: &Connection,
    range: DateRange,
    now: OffsetDateTime,
) -> Result<DashboardSnapshot> {
    let (start, end) = range.resolve(now);

    let category_totals = query_category_totals(conn, start, end)?;
    let daily_trend = compute_daily_trend(conn, start, end)?;
    let fixed_total: i64 = category_totals
        .iter()
        .filter(|c| c.kind == CategoryKind::Fixed)
        .map(|c| c.total_cents)
        .sum();
    let variable_total: i64 = category_totals
        .iter()
        .filter(|c| c.kind == CategoryKind::Variable)
        .map(|c| c.total_cents)
        .sum();
    // Mirror total_budget_cents, which is fixed + variable targets only.
    // Investing is a savings goal, not a spending allowance, and is excluded
    // from the budget — so its spend must not be subtracted from
    // total_remaining either. Otherwise a savings contribution drags
    // total_remaining below variable_remaining once fixed + savings outflow
    // tops the fixed budget. (category_totals still carries investing for the
    // per-category breakdown and the savings-goal note.)
    let total_spent_cents: i64 = fixed_total + variable_total;

    // Period pacing math is only meaningful when the selected range is
    // the *current* calendar month — variable_remaining and the daily
    // allowance both depend on "today" being inside the period. For a
    // prior-month view, surface the static totals (budget / remaining /
    // spent) without the pacing fields.
    let is_current = range.is_current_month(now);
    let is_monthly = range.is_monthly();
    let (variable_budget_cents, fixed_budget_cents) = if is_monthly {
        query_active_targets(conn)?
    } else {
        (0, 0)
    };
    let total_budget_cents = if is_monthly {
        fixed_budget_cents + variable_budget_cents
    } else {
        0
    };
    let total_remaining_cents = if is_monthly {
        total_budget_cents - total_spent_cents
    } else {
        0
    };
    let (period, kpi) = if is_current {
        let snap = compute_snapshot(
            now,
            fixed_budget_cents,
            fixed_total,
            variable_budget_cents,
            variable_total,
        );
        let kpi = KpiCard {
            variable_remaining_cents: snap.variable_remaining_cents,
            daily_variable_allowance_cents: snap.daily_variable_allowance_cents,
            total_spent_cents,
            days_remaining: snap.days_remaining,
            on_pace: snap.on_pace,
            total_budget_cents,
            total_remaining_cents,
            variable_budget_cents,
            fixed_budget_cents,
        };
        (Some(snap), kpi)
    } else {
        let kpi = KpiCard {
            variable_remaining_cents: 0,
            daily_variable_allowance_cents: 0,
            total_spent_cents,
            days_remaining: 0,
            on_pace: true,
            total_budget_cents,
            total_remaining_cents,
            variable_budget_cents,
            fixed_budget_cents,
        };
        (None, kpi)
    };

    let fixed_vs_variable = FixedVariableBreakdown {
        fixed_committed_cents: fixed_total,
        variable_spent_cents: variable_total,
        variable_remaining_cents: period
            .as_ref()
            .map(|p| p.variable_remaining_cents)
            .unwrap_or(0),
    };

    let member_spend = query_member_spend(conn, start, end)?;
    let top_expenses = query_top_expenses(conn, start, end, 5)?;
    let over_budget = if is_monthly {
        query_over_budget(conn, start, end)?
    } else {
        Vec::new()
    };
    let upcoming_fixed = if is_current {
        query_upcoming_fixed(conn, now)?
    } else {
        Vec::new()
    };
    let mom_comparison = if is_current {
        Some(compute_mom(conn, now)?)
    } else {
        None
    };

    Ok(DashboardSnapshot {
        range,
        start,
        end,
        period,
        kpi,
        category_totals,
        daily_trend,
        fixed_vs_variable,
        member_spend,
        top_expenses,
        over_budget,
        upcoming_fixed,
        mom_comparison,
    })
}

fn query_category_totals(
    conn: &Connection,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> Result<Vec<CategoryTotal>> {
    let sql = format!(
        // is_active = 1 keeps this spend set equal to the budget set in
        // query_active_targets: a deactivated category contributes neither
        // budget nor paced spend, so it can't inflate the total budget and
        // can't orphan its spend (the "bot can't see my budget" bug).
        "SELECT c.id, c.name, c.kind, c.monthly_target_cents,
                COALESCE(SUM({SIGNED_AMOUNT_SQL}), 0) AS total
         FROM categories c
         LEFT JOIN expenses e ON e.category_id = c.id
             AND e.occurred_at >= ?1 AND e.occurred_at < ?2
         WHERE c.is_active = 1
         GROUP BY c.id, c.name, c.kind, c.monthly_target_cents
         HAVING total > 0
         ORDER BY total DESC"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(params![start, end], |r| {
            Ok(CategoryTotal {
                category_id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                monthly_target_cents: r.get(3)?,
                total_cents: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Bucket expenses into daily totals (split into fixed and variable) using
/// the offset of the range start. Days with no spend are still emitted as
/// zeroes so the line chart doesn't show gaps.
fn compute_daily_trend(
    conn: &Connection,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> Result<Vec<DailyTrendPoint>> {
    let exps = expenses::list_in_range(conn, start, end)?;
    let kinds = category_kind_lookup(conn)?;

    let offset = start.offset();
    let mut buckets: BTreeMap<Date, (i64, i64)> = BTreeMap::new();
    // Initialize every day in the range with zero so the chart is dense.
    let mut day = start.to_offset(offset).date();
    let last = (end.to_offset(offset) - Duration::seconds(1)).date();
    while day <= last {
        buckets.insert(day, (0, 0));
        day += Duration::days(1);
    }

    for e in &exps {
        let local = e.occurred_at.to_offset(offset).date();
        let bucket = buckets.entry(local).or_insert((0, 0));
        let signed = if e.is_refund {
            -e.amount_cents
        } else {
            e.amount_cents
        };
        match e.category_id.and_then(|id| kinds.get(&id).copied()) {
            Some(CategoryKind::Fixed) => bucket.0 += signed,
            Some(CategoryKind::Variable) => bucket.1 += signed,
            // Investing contributions don't show on the daily fixed-vs-
            // variable line chart — they're outflows but not "spend" in
            // the budget-pacing sense. They appear in the per-category
            // bar chart instead.
            Some(CategoryKind::Investing) => {}
            None => {} // uncategorized expenses don't pace either bucket
        }
    }

    Ok(buckets
        .into_iter()
        .map(|(date, (f, v))| DailyTrendPoint {
            date,
            fixed_cents: f,
            variable_cents: v,
        })
        .collect())
}

fn category_kind_lookup(conn: &Connection) -> Result<std::collections::HashMap<i64, CategoryKind>> {
    let mut stmt = conn.prepare_cached("SELECT id, kind FROM categories")?;
    let rows = stmt.query_map([], |r| {
        let id: i64 = r.get(0)?;
        let kind: CategoryKind = r.get(1)?;
        Ok((id, kind))
    })?;
    let mut map = std::collections::HashMap::new();
    for r in rows {
        let (id, kind) = r?;
        map.insert(id, kind);
    }
    Ok(map)
}

/// Sum of `monthly_target_cents` across **active** variable / fixed
/// categories that have a target set. Returns
/// `(variable_target_total, fixed_target_total)`.
///
/// Filtered by `is_active = 1`, in lockstep with `query_category_totals`
/// (which feeds `variable_spent`) and `query_over_budget` — all three scope
/// the dashboard to the active categories. Keeping the budget and spend
/// queries on the *same* set is what makes it coherent. A deactivated category
/// contributes no target, so it can't inflate the budget (the "Insights total
/// budget too high" bug); and it contributes no paced spend, so its spend can't
/// count against the user while its allowance silently vanishes (the "bot can't
/// see my budget" bug). Reactivating a category restores both together.
fn query_active_targets(conn: &Connection) -> Result<(i64, i64)> {
    let mut stmt = conn.prepare_cached(
        "SELECT kind, COALESCE(SUM(monthly_target_cents), 0)
         FROM categories
         WHERE monthly_target_cents IS NOT NULL AND is_active = 1
         GROUP BY kind",
    )?;
    let mut variable = 0i64;
    let mut fixed = 0i64;
    let rows = stmt.query_map([], |r| {
        let kind: CategoryKind = r.get(0)?;
        let total: i64 = r.get(1)?;
        Ok((kind, total))
    })?;
    for r in rows {
        let (kind, total) = r?;
        match kind {
            CategoryKind::Fixed => fixed = total,
            CategoryKind::Variable => variable = total,
            // Investing targets are tracked separately and don't feed
            // the variable/fixed pacing math.
            CategoryKind::Investing => {}
        }
    }
    Ok((variable, fixed))
}

fn query_member_spend(
    conn: &Connection,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> Result<Vec<MemberSpend>> {
    let sql = format!(
        "SELECT t.chat_id, t.display_name, COALESCE(SUM({SIGNED_AMOUNT_SQL}), 0) AS total
         FROM telegram_authorized_chats t
         LEFT JOIN expenses e ON e.logged_by_chat_id = t.chat_id
             AND e.occurred_at >= ?1 AND e.occurred_at < ?2
         GROUP BY t.chat_id, t.display_name
         HAVING total > 0
         ORDER BY total DESC"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(params![start, end], |r| {
            Ok(MemberSpend {
                chat_id: r.get(0)?,
                display_name: r.get(1)?,
                total_cents: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn query_top_expenses(
    conn: &Connection,
    start: OffsetDateTime,
    end: OffsetDateTime,
    limit: u32,
) -> Result<Vec<Expense>> {
    // Refunds are excluded — "top expenses" means biggest outflows, not
    // biggest credits. Refunds still appear in lists/queries that surface
    // them explicitly.
    let mut stmt = conn.prepare_cached(
        "SELECT id, amount_cents, currency, category_id, description, occurred_at, created_at,
                source, raw_message, llm_confidence, logged_by_chat_id,
                is_refund, refund_for_expense_id
         FROM expenses
         WHERE occurred_at >= ?1 AND occurred_at < ?2 AND is_refund = 0
         ORDER BY amount_cents DESC, id DESC
         LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![start, end, limit], |r| {
            Ok(Expense {
                id: r.get(0)?,
                amount_cents: r.get(1)?,
                currency: r.get(2)?,
                category_id: r.get(3)?,
                description: r.get(4)?,
                occurred_at: r.get(5)?,
                created_at: r.get(6)?,
                source: r.get(7)?,
                raw_message: r.get(8)?,
                llm_confidence: r.get(9)?,
                logged_by_chat_id: r.get(10)?,
                is_refund: r.get::<_, i64>(11)? != 0,
                refund_for_expense_id: r.get(12)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn query_over_budget(
    conn: &Connection,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> Result<Vec<OverBudgetCategory>> {
    let sql = format!(
        "SELECT c.id, c.name, c.monthly_target_cents,
                COALESCE(SUM({SIGNED_AMOUNT_SQL}), 0) AS spent
         FROM categories c
         LEFT JOIN expenses e ON e.category_id = c.id
             AND e.occurred_at >= ?1 AND e.occurred_at < ?2
         WHERE c.monthly_target_cents IS NOT NULL AND c.is_active = 1
         GROUP BY c.id, c.name, c.monthly_target_cents
         HAVING spent > c.monthly_target_cents
         ORDER BY (spent - c.monthly_target_cents) DESC"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(params![start, end], |r| {
            let id: i64 = r.get(0)?;
            let name: String = r.get(1)?;
            let target: i64 = r.get(2)?;
            let spent: i64 = r.get(3)?;
            Ok(OverBudgetCategory {
                category_id: id,
                name,
                spent_cents: spent,
                target_cents: target,
                overage_cents: spent - target,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn query_upcoming_fixed(conn: &Connection, now: OffsetDateTime) -> Result<Vec<UpcomingFixed>> {
    let (month_start, month_end) = current_month_bounds(now);
    let today_dom = now.day();
    let mut stmt = conn.prepare_cached(
        "SELECT c.id, c.name, c.recurrence_day_of_month, c.monthly_target_cents
         FROM categories c
         WHERE c.is_recurring = 1
           AND c.is_active = 1
           AND c.recurrence_day_of_month IS NOT NULL
           AND c.recurrence_day_of_month >= ?1
           AND c.id NOT IN (
               SELECT DISTINCT category_id FROM expenses
               WHERE category_id IS NOT NULL
                 AND occurred_at >= ?2 AND occurred_at < ?3
                 AND is_refund = 0
           )
         ORDER BY c.recurrence_day_of_month ASC, c.name ASC",
    )?;
    let rows = stmt
        .query_map(params![today_dom as i64, month_start, month_end], |r| {
            Ok(UpcomingFixed {
                category_id: r.get(0)?,
                name: r.get(1)?,
                recurrence_day_of_month: r.get::<_, i64>(2)? as u8,
                expected_amount_cents: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Variable spend in the current MTD window vs. the same number of
/// elapsed days in the previous calendar month.
fn compute_mom(conn: &Connection, now: OffsetDateTime) -> Result<MoMComparison> {
    let offset = now.offset();
    let (this_start, _) = current_month_bounds(now);

    // "Same point last month" = first N days of last month, where N = current day_of_month.
    let day_of_month = now.day() as i64;
    let prev_month_first = previous_month_first(now);
    let prev_month_days = days_in_month(prev_month_first.date());
    let cap = day_of_month.min(prev_month_days as i64);
    let prev_period_end = prev_month_first.date() + Duration::days(cap);
    let prev_period_end_dt = prev_period_end
        .with_time(Time::MIDNIGHT)
        .assume_offset(offset);

    let this_period =
        expenses::sum_in_range_by_kind(conn, this_start, now, CategoryKind::Variable)?;
    let last_period = expenses::sum_in_range_by_kind(
        conn,
        prev_month_first,
        prev_period_end_dt,
        CategoryKind::Variable,
    )?;

    let delta = this_period - last_period;
    let delta_pct = if last_period > 0 {
        Some((delta as f64 / last_period as f64) * 100.0)
    } else {
        None
    };

    Ok(MoMComparison {
        variable_spent_this_period_cents: this_period,
        variable_spent_same_point_last_month_cents: last_period,
        delta_cents: delta,
        delta_pct,
    })
}

fn previous_month_first(now: OffsetDateTime) -> OffsetDateTime {
    let d = now.date();
    let (year, month) = if d.month() == Month::January {
        (d.year() - 1, Month::December)
    } else {
        (d.year(), d.month().previous())
    };
    Date::from_calendar_date(year, month, 1)
        .expect("day 1 always valid")
        .with_time(Time::MIDNIGHT)
        .assume_offset(now.offset())
}

fn days_in_month(d: Date) -> u8 {
    // time crate gives this directly via `d.month().length(d.year())`.
    d.month().length(d.year())
}

#[cfg(test)]
mod target_set_tests {
    use super::*;
    use crate::db;

    use time::macros::datetime;

    // April 2026, the window the spend-side test queries.
    const START: OffsetDateTime = datetime!(2026-04-01 00:00:00 UTC);
    const END: OffsetDateTime = datetime!(2026-05-01 00:00:00 UTC);

    fn seed_active_and_inactive(conn: &Connection) {
        // $500 active variable category, $200 inactive variable category,
        // both with a saved target.
        conn.execute(
            "INSERT INTO categories (id, name, kind, monthly_target_cents, is_active, is_seed)
             VALUES (101, 'TestGroceries', 'variable', 50000, 1, 0),
                    (102, 'TestEntertainment', 'variable', 20000, 0, 0)",
            [],
        )
        .unwrap();
    }

    // The "Insights total budget too high" bug: a deactivated category's target
    // must NOT be summed into the budget, even if it has spend this period.
    #[test]
    fn inactive_category_target_is_excluded_from_budget() {
        let conn = db::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        seed_active_and_inactive(&conn);
        // Spend in the deactivated category this period — still must not pull
        // its $200 target into the budget.
        conn.execute(
            "INSERT INTO expenses (amount_cents, category_id, occurred_at, source)
             VALUES (3000, 102, '2026-04-15T12:00:00Z', 'manual')",
            [],
        )
        .unwrap();

        let (variable, _fixed) = query_active_targets(&conn).unwrap();
        assert_eq!(
            variable, 50000,
            "deactivated category ($200 target) must be excluded; only the \
             active $500 should count, got {variable}"
        );
    }

    // The symmetric half that keeps the "bot can't see my budget" bug dead:
    // because the budget query drops inactive categories, the *spend* query
    // must drop them too — otherwise that spend would be paced with no matching
    // allowance. query_category_totals filters is_active for exactly this.
    #[test]
    fn inactive_category_spend_is_excluded_from_pacing() {
        let conn = db::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        seed_active_and_inactive(&conn);
        conn.execute(
            "INSERT INTO expenses (amount_cents, category_id, occurred_at, source)
             VALUES (4000, 101, '2026-04-10T12:00:00Z', 'manual'),
                    (3000, 102, '2026-04-15T12:00:00Z', 'manual')",
            [],
        )
        .unwrap();

        let totals = query_category_totals(&conn, START, END).unwrap();
        let spent: i64 = totals.iter().map(|c| c.total_cents).sum();
        assert_eq!(
            spent, 4000,
            "only the active category's $40 should be paced; the inactive \
             category's $30 must be excluded, got {spent}"
        );
        assert!(
            totals.iter().all(|c| c.category_id != 102),
            "deactivated category must not appear in the spend breakdown"
        );
    }
}

#[cfg(test)]
mod total_remaining_tests {
    use super::*;
    use crate::db;

    use time::macros::datetime;

    // Regression: a savings (investing) contribution must not be subtracted
    // from total_remaining. total_budget is fixed + variable only, so
    // total_spent must be too — otherwise total_remaining dips below
    // variable_remaining purely because the user saved money.
    #[test]
    fn investing_spend_does_not_pull_total_remaining_below_variable() {
        let conn = db::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        // Fixed budget $10 (spent $8, under budget), variable budget $5
        // (spent $1), savings goal with a $4 contribution.
        conn.execute(
            "INSERT INTO categories (id, name, kind, monthly_target_cents, is_active, is_seed)
             VALUES (201, 'TestRent', 'fixed', 1000, 1, 0),
                    (202, 'TestDining', 'variable', 500, 1, 0),
                    (203, 'TestSavings', 'investing', 400, 1, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO expenses (amount_cents, category_id, occurred_at, source)
             VALUES (800, 201, '2026-04-10T12:00:00Z', 'manual'),
                    (100, 202, '2026-04-11T12:00:00Z', 'manual'),
                    (400, 203, '2026-04-12T12:00:00Z', 'manual')",
            [],
        )
        .unwrap();

        let now = datetime!(2026-04-15 12:00:00 UTC);
        let snap = dashboard(&conn, DateRange::ThisMonth, now).unwrap();
        let kpi = &snap.kpi;

        // total_budget = 1000 + 500 = 1500 (investing excluded).
        assert_eq!(kpi.total_budget_cents, 1500);
        // total_spent = 800 + 100 = 900 — the $400 savings is NOT counted.
        assert_eq!(
            kpi.total_spent_cents, 900,
            "investing spend must be excluded from total_spent; got {}",
            kpi.total_spent_cents
        );
        // total_remaining = 1500 - 900 = 600, well above variable_remaining.
        assert_eq!(kpi.total_remaining_cents, 600);
        assert_eq!(kpi.variable_remaining_cents, 400); // 500 - 100
        assert!(
            kpi.total_remaining_cents >= kpi.variable_remaining_cents,
            "total_remaining ({}) must not be below variable_remaining ({})",
            kpi.total_remaining_cents,
            kpi.variable_remaining_cents,
        );
    }
}
