//! Per-category analyzer (opt-in tool, not auto-rendered).
//!
//! Replaces the v0.3.0 single-purpose Trend Analyzer with a richer
//! analysis surface. User picks a category + a time window and gets:
//!
//! - **Per-transaction stats** over individual non-refund rows
//!   (n, mean, median, σ, min, max). Tells the user what a typical
//!   *purchase* in this category looks like.
//! - **Per-bucket stats** over the auto-derived granularity (daily /
//!   weekly / bi-weekly / monthly depending on window length). Useful
//!   for budget planning at the cadence the user thinks about.
//! - **Refund summary** (count + total $) called out separately so
//!   it doesn't pollute the per-transaction "typical purchase" stats
//!   but the user still sees them.
//! - **Linear regression** on the bucket totals with slope reported
//!   as `$/mo per year` (always normalized to that unit regardless of
//!   bucket size, so headlines stay comparable across windows).
//! - **Headline** plain-English string.
//!
//! Granularity is auto-derived from the window: 2w→daily(14),
//! month→daily(30), quarter→weekly(13), half-year→bi-weekly(13),
//! year→monthly(12). See `bucket_size_for_window` for the rule.

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use time::{Date, Duration, Month, OffsetDateTime, Time};

use crate::insights::stats;
use crate::repository::expenses;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisWindow {
    /// 14 days back, daily buckets (14).
    TwoWeeks,
    /// ~30 days back, daily buckets (30).
    Month,
    /// ~91 days back, weekly buckets (~13).
    Quarter,
    /// ~182 days back, bi-weekly buckets (~13).
    HalfYear,
    /// ~365 days back, monthly buckets (~12).
    Year,
}

impl AnalysisWindow {
    pub fn days_back(&self) -> i64 {
        match self {
            AnalysisWindow::TwoWeeks => 14,
            AnalysisWindow::Month => 30,
            AnalysisWindow::Quarter => 91,
            AnalysisWindow::HalfYear => 182,
            AnalysisWindow::Year => 365,
        }
    }

    pub fn bucket_size_days(&self) -> i64 {
        match self {
            AnalysisWindow::TwoWeeks => 1,
            AnalysisWindow::Month => 1,
            AnalysisWindow::Quarter => 7,
            AnalysisWindow::HalfYear => 14,
            // Year uses calendar months, not fixed days. Reported here
            // as ~30 for the few callers that want a rough number.
            AnalysisWindow::Year => 30,
        }
    }

    pub fn bucket_label(&self) -> &'static str {
        match self {
            AnalysisWindow::TwoWeeks | AnalysisWindow::Month => "daily",
            AnalysisWindow::Quarter => "weekly",
            AnalysisWindow::HalfYear => "bi-weekly",
            AnalysisWindow::Year => "monthly",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PerTransactionStats {
    pub n: usize,
    pub mean_cents: i64,
    pub median_cents: i64,
    pub stddev_cents: i64,
    pub min_cents: i64,
    pub max_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerBucketStats {
    pub n_buckets: usize,
    pub mean_cents: i64,
    pub median_cents: i64,
    pub stddev_cents: i64,
    pub min_cents: i64,
    pub max_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BucketPoint {
    pub bucket_index: u32,
    pub label: String,
    pub total_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RefundSummary {
    pub count: usize,
    pub total_cents: i64,
    /// Net spent (charges − refunds) over the window. Provided here so
    /// the UI doesn't have to recompute.
    pub net_spent_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryAnalysis {
    pub window: AnalysisWindow,
    pub bucket_label: &'static str,
    pub buckets: Vec<BucketPoint>,
    pub per_transaction: Option<PerTransactionStats>,
    pub per_bucket: Option<PerBucketStats>,
    pub refunds: RefundSummary,
    /// Cents per month of monthly delta (i.e., $/mo per year /12).
    /// UI multiplies by 12 for the headline.
    pub slope_cents_per_month_per_year: f64,
    pub r_squared: f64,
    pub direction: &'static str,
    pub headline: String,
}

pub fn analyze(
    conn: &Connection,
    category_id: i64,
    window: AnalysisWindow,
    now: OffsetDateTime,
) -> Result<CategoryAnalysis> {
    let start = now - Duration::days(window.days_back());
    let rows = expenses::list_in_range_by_category(conn, category_id, start, now)?;

    let charges: Vec<i64> = rows
        .iter()
        .filter(|e| !e.is_refund)
        .map(|e| e.amount_cents)
        .collect();
    let refund_amounts: Vec<i64> = rows
        .iter()
        .filter(|e| e.is_refund)
        .map(|e| e.amount_cents)
        .collect();

    let per_tx = stats::describe(&charges).map(|d| PerTransactionStats {
        n: d.n,
        mean_cents: d.mean_cents,
        median_cents: d.median_cents,
        stddev_cents: d.stddev_cents,
        min_cents: d.min_cents,
        max_cents: d.max_cents,
    });

    let buckets = bucketize(&rows, window, start, now);
    let totals: Vec<i64> = buckets.iter().map(|b| b.total_cents).collect();
    let per_bucket = stats::describe(&totals).map(|d| PerBucketStats {
        n_buckets: d.n,
        mean_cents: d.mean_cents,
        median_cents: d.median_cents,
        stddev_cents: d.stddev_cents,
        min_cents: d.min_cents,
        max_cents: d.max_cents,
    });

    let refunds = RefundSummary {
        count: refund_amounts.len(),
        total_cents: refund_amounts.iter().sum(),
        net_spent_cents: charges.iter().sum::<i64>() - refund_amounts.iter().sum::<i64>(),
    };

    // Linear regression on bucket totals. Slope unit is cents per
    // bucket. Convert to "cents per month per year" by:
    //   cents_per_bucket / bucket_size_months * 12_months_per_year
    // We define `bucket_size_months` such that:
    //   daily   = 1/30
    //   weekly  = 7/30
    //   bi-weekly = 14/30
    //   monthly = 1
    // The slope is per-bucket-step, so per-month is slope / months_per_bucket,
    // and per-year is that × 12.
    let buckets_per_year = match window {
        AnalysisWindow::TwoWeeks | AnalysisWindow::Month => 365.0,
        AnalysisWindow::Quarter => 52.0,
        AnalysisWindow::HalfYear => 26.0,
        AnalysisWindow::Year => 12.0,
    };
    let buckets_per_month = buckets_per_year / 12.0;
    let (slope_per_bucket, _intercept, r_sq) = if totals.len() >= 2 {
        least_squares(&totals)
    } else {
        (0.0, 0.0, 0.0)
    };
    let slope_cents_per_year = slope_per_bucket * buckets_per_year;
    let slope_cents_per_month = slope_per_bucket * buckets_per_month;
    let direction = classify_direction(slope_cents_per_year);
    let headline = build_headline(slope_cents_per_year, r_sq, direction, per_tx.is_some());

    Ok(CategoryAnalysis {
        window,
        bucket_label: window.bucket_label(),
        buckets,
        per_transaction: per_tx,
        per_bucket,
        refunds,
        slope_cents_per_month_per_year: slope_cents_per_month,
        r_squared: r_sq,
        direction,
        headline,
    })
}

/// Bucket rows into time slices. Refunds contribute negatively
/// (signed-sum convention).
fn bucketize(
    rows: &[crate::domain::Expense],
    window: AnalysisWindow,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> Vec<BucketPoint> {
    let bucket_days = window.bucket_size_days();
    let buckets: Vec<(OffsetDateTime, OffsetDateTime, String)> = match window {
        AnalysisWindow::Year => calendar_month_buckets(start, end),
        _ => fixed_day_buckets(start, end, bucket_days),
    };

    let mut totals = vec![0i64; buckets.len()];
    for r in rows {
        for (i, (lo, hi, _)) in buckets.iter().enumerate() {
            if r.occurred_at >= *lo && r.occurred_at < *hi {
                let signed = if r.is_refund {
                    -r.amount_cents
                } else {
                    r.amount_cents
                };
                totals[i] += signed;
                break;
            }
        }
    }

    buckets
        .into_iter()
        .enumerate()
        .map(|(i, (_, _, label))| BucketPoint {
            bucket_index: i as u32,
            label,
            total_cents: totals[i],
        })
        .collect()
}

fn fixed_day_buckets(
    start: OffsetDateTime,
    end: OffsetDateTime,
    bucket_days: i64,
) -> Vec<(OffsetDateTime, OffsetDateTime, String)> {
    let mut out = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let next = std::cmp::min(cursor + Duration::days(bucket_days), end);
        let label = cursor.date().to_string();
        out.push((cursor, next, label));
        cursor = next;
    }
    out
}

fn calendar_month_buckets(
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> Vec<(OffsetDateTime, OffsetDateTime, String)> {
    let mut out = Vec::new();
    let offset = end.offset();
    let mut y = start.year();
    let mut m = start.month() as u8;
    loop {
        let bucket_start = Date::from_calendar_date(y, Month::try_from(m).expect("valid month"), 1)
            .expect("valid date")
            .with_time(Time::MIDNIGHT)
            .assume_offset(offset);
        let (ny, nm) = if m == 12 { (y + 1, 1u8) } else { (y, m + 1) };
        let next_start = Date::from_calendar_date(ny, Month::try_from(nm).expect("valid month"), 1)
            .expect("valid date")
            .with_time(Time::MIDNIGHT)
            .assume_offset(offset);
        if bucket_start >= end {
            break;
        }
        let lo = std::cmp::max(bucket_start, start);
        let hi = std::cmp::min(next_start, end);
        let label = format!("{y}-{:02}", m);
        out.push((lo, hi, label));
        y = ny;
        m = nm;
        // Cap at 14 buckets to keep the chart readable.
        if out.len() >= 14 {
            break;
        }
    }
    out
}

/// Closed-form least-squares: fit `y = slope*x + intercept` where `x`
/// is the bucket index 0..n-1 and `y` is the bucket total in cents.
/// Also returns R².
fn least_squares(y: &[i64]) -> (f64, f64, f64) {
    let n = y.len() as f64;
    if n < 2.0 {
        return (0.0, 0.0, 0.0);
    }
    let mean_x = (n - 1.0) / 2.0;
    let mean_y: f64 = y.iter().map(|v| *v as f64).sum::<f64>() / n;
    let mut num = 0.0;
    let mut den_x = 0.0;
    for (i, v) in y.iter().enumerate() {
        let dx = i as f64 - mean_x;
        let dy = *v as f64 - mean_y;
        num += dx * dy;
        den_x += dx * dx;
    }
    if den_x.abs() < 1e-12 {
        return (0.0, mean_y, 0.0);
    }
    let slope = num / den_x;
    let intercept = mean_y - slope * mean_x;
    let mut ss_res = 0.0;
    let mut ss_tot = 0.0;
    for (i, v) in y.iter().enumerate() {
        let pred = slope * i as f64 + intercept;
        ss_res += (*v as f64 - pred).powi(2);
        ss_tot += (*v as f64 - mean_y).powi(2);
    }
    let r_sq = if ss_tot < 1e-12 {
        0.0
    } else {
        1.0 - ss_res / ss_tot
    };
    (slope, intercept, r_sq)
}

fn classify_direction(slope_per_year_cents: f64) -> &'static str {
    if slope_per_year_cents.abs() < 1000.0 {
        "flat"
    } else if slope_per_year_cents > 0.0 {
        "rising"
    } else {
        "falling"
    }
}

fn build_headline(
    slope_per_year_cents: f64,
    r_sq: f64,
    direction: &str,
    have_data: bool,
) -> String {
    if !have_data {
        return "Not enough history yet — log a few more purchases in this category.".into();
    }
    match direction {
        "flat" => "Spending in this category is roughly flat over the period.".into(),
        _ => {
            let dollars_per_mo_per_yr = (slope_per_year_cents.abs() / 100.0).round() as i64;
            let strength = if r_sq >= 0.7 {
                "strong"
            } else if r_sq >= 0.4 {
                "moderate"
            } else {
                "weak"
            };
            format!(
                "Spending is {direction} at ${dollars_per_mo_per_yr}/mo per year — {strength} trend (R²={:.2}).",
                r_sq
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::domain::{ExpenseSource, NewExpense};
    use crate::repository::{categories, expenses};

    fn fresh_conn() -> Connection {
        let conn = db::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        conn
    }

    fn first_active_category(conn: &Connection) -> i64 {
        categories::list(conn, false)
            .unwrap()
            .into_iter()
            .find(|c| c.is_active)
            .unwrap()
            .id
    }

    fn insert(conn: &Connection, cat: i64, amount: i64, when: OffsetDateTime, is_refund: bool) {
        expenses::insert(
            conn,
            &NewExpense {
                amount_cents: amount,
                currency: "USD".into(),
                category_id: Some(cat),
                description: None,
                occurred_at: when,
                source: ExpenseSource::Manual,
                raw_message: None,
                llm_confidence: None,
                logged_by_chat_id: None,
                is_refund,
                refund_for_expense_id: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn no_data_returns_empty_stats() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let r = analyze(
            &conn,
            cat,
            AnalysisWindow::Quarter,
            OffsetDateTime::now_utc(),
        )
        .unwrap();
        assert!(r.per_transaction.is_none());
        assert!(r.headline.contains("Not enough"));
    }

    #[test]
    fn per_transaction_stats_excludes_refunds() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let now = OffsetDateTime::now_utc();
        // 3 charges + 1 refund within the last 14 days.
        for (i, amt) in [500, 600, 700].iter().enumerate() {
            insert(&conn, cat, *amt, now - Duration::days(i as i64 + 1), false);
        }
        insert(&conn, cat, 200, now - Duration::days(2), true);
        let r = analyze(&conn, cat, AnalysisWindow::TwoWeeks, now).unwrap();
        let stats = r.per_transaction.unwrap();
        assert_eq!(stats.n, 3);
        assert_eq!(stats.min_cents, 500);
        assert_eq!(stats.max_cents, 700);
        assert_eq!(stats.median_cents, 600);
        // Refund should be in the refund summary.
        assert_eq!(r.refunds.count, 1);
        assert_eq!(r.refunds.total_cents, 200);
    }

    #[test]
    fn net_spent_subtracts_refunds() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let now = OffsetDateTime::now_utc();
        insert(&conn, cat, 1000, now - Duration::days(1), false);
        insert(&conn, cat, 1000, now - Duration::days(2), false);
        insert(&conn, cat, 300, now - Duration::days(3), true);
        let r = analyze(&conn, cat, AnalysisWindow::TwoWeeks, now).unwrap();
        assert_eq!(r.refunds.net_spent_cents, 1700);
    }

    #[test]
    fn rising_trend_classifier_flags_increase() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let now = OffsetDateTime::now_utc();
        // 8 weeks of rising weekly spend.
        for week in 0i64..8 {
            let amt = 1000 + (week * 500);
            insert(&conn, cat, amt, now - Duration::days((week + 1) * 7), false);
        }
        let r = analyze(&conn, cat, AnalysisWindow::Quarter, now).unwrap();
        assert_eq!(r.direction, "rising");
        assert!(r.slope_cents_per_month_per_year > 0.0);
    }

    #[test]
    fn flat_trend_classifier_returns_flat() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let now = OffsetDateTime::now_utc();
        // One $100 charge per day across the full 91-day window. With
        // weekly buckets that's $700 in every bucket — perfectly flat.
        for day in 1..=91 {
            insert(&conn, cat, 10_000, now - Duration::days(day), false);
        }
        let r = analyze(&conn, cat, AnalysisWindow::Quarter, now).unwrap();
        assert_eq!(r.direction, "flat", "headline: {}", r.headline);
        assert!(r.headline.contains("flat"));
    }

    #[test]
    fn window_to_bucket_label_is_correct() {
        assert_eq!(AnalysisWindow::TwoWeeks.bucket_label(), "daily");
        assert_eq!(AnalysisWindow::Month.bucket_label(), "daily");
        assert_eq!(AnalysisWindow::Quarter.bucket_label(), "weekly");
        assert_eq!(AnalysisWindow::HalfYear.bucket_label(), "bi-weekly");
        assert_eq!(AnalysisWindow::Year.bucket_label(), "monthly");
    }
}
