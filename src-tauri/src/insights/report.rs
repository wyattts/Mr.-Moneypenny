// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//! Deterministic analysis engine for the AI Report Wizard (v0.4.0).
//!
//! Every figure in a generated report is computed here, in Rust, from
//! the local database — never by the language model. The LLM's only job
//! (Phase 2) is to narrate and advise *over* these numbers, so the
//! quantitative content of a report is reproducible and cannot be
//! hallucinated. This module performs no network I/O and holds no LLM
//! types.
//!
//! All amounts are integer cents. All bucketing is done in Rust against
//! the offset carried by the `now` passed in, identical to the rest of
//! `insights`, so behavior does not depend on the host timezone.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;
use time::{Date, Duration, Month, OffsetDateTime, Time};

use crate::domain::{Category, CategoryKind};
use crate::repository::{categories, expenses, settings};

/// Average days per calendar month (365.25 / 12). Used to prorate a
/// monthly budget target onto an arbitrary-length report window.
const AVG_DAYS_PER_MONTH: f64 = 30.4375;

/// Report window selection. The four presets resolve to the *previous
/// complete* calendar period relative to `now` (e.g. on 2026-05-15,
/// `LastMonth` is all of April 2026) — a finished period the user can
/// actually reason about, not a partial in-progress one.
#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportTimeframe {
    LastWeek,
    LastMonth,
    LastQuarter,
    LastYear,
    /// Inclusive: `to` is the last day included.
    Custom {
        from: Date,
        to: Date,
    },
}

impl ReportTimeframe {
    /// Resolve to a half-open `[start, end)` interval in `now`'s offset.
    pub fn resolve(&self, now: OffsetDateTime) -> (OffsetDateTime, OffsetDateTime) {
        let offset = now.offset();
        let today = now.date();
        let midnight = |d: Date| d.with_time(Time::MIDNIGHT).assume_offset(offset);
        match *self {
            ReportTimeframe::LastWeek => {
                let days_since_monday = today.weekday().number_days_from_monday() as i64;
                let this_monday = today - Duration::days(days_since_monday);
                let last_monday = this_monday - Duration::days(7);
                (midnight(last_monday), midnight(this_monday))
            }
            ReportTimeframe::LastMonth => {
                let this_first = Date::from_calendar_date(today.year(), today.month(), 1)
                    .expect("day 1 always valid");
                let prev_first = first_of_previous_month(this_first);
                (midnight(prev_first), midnight(this_first))
            }
            ReportTimeframe::LastQuarter => {
                let q_start_month = quarter_start_month(today.month());
                let this_q_start = Date::from_calendar_date(today.year(), q_start_month, 1)
                    .expect("first of quarter always valid");
                // Step back three months to the prior quarter's first day.
                let mut prev = this_q_start;
                for _ in 0..3 {
                    prev = first_of_previous_month(prev);
                }
                (midnight(prev), midnight(this_q_start))
            }
            ReportTimeframe::LastYear => {
                let start = Date::from_calendar_date(today.year() - 1, Month::January, 1).unwrap();
                let end = Date::from_calendar_date(today.year(), Month::January, 1).unwrap();
                (midnight(start), midnight(end))
            }
            ReportTimeframe::Custom { from, to } => {
                (midnight(from), midnight(to + Duration::days(1)))
            }
        }
    }

    /// Human label for the resolved window, used in the report header.
    pub fn label(&self, now: OffsetDateTime) -> String {
        let (start, end) = self.resolve(now);
        let last_day = (end - Duration::seconds(1)).date();
        format!("{} to {}", start.date(), last_day)
    }
}

fn quarter_start_month(m: Month) -> Month {
    match m {
        Month::January | Month::February | Month::March => Month::January,
        Month::April | Month::May | Month::June => Month::April,
        Month::July | Month::August | Month::September => Month::July,
        Month::October | Month::November | Month::December => Month::October,
    }
}

fn first_of_previous_month(first_of_month: Date) -> Date {
    let (y, m) = if first_of_month.month() == Month::January {
        (first_of_month.year() - 1, Month::December)
    } else {
        (first_of_month.year(), first_of_month.month().previous())
    };
    Date::from_calendar_date(y, m, 1).expect("first of month always valid")
}

// ---------------------------------------------------------------------------
// Output structures. Every field is a deterministic figure or a label;
// no advice text — that is the LLM's job in Phase 2. All are Serialize so
// the prompt builder can hand them to the model verbatim and the UI can
// render the same numbers.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RebalanceRow {
    pub category_id: i64,
    pub name: String,
    pub kind: CategoryKind,
    /// The category's configured monthly target, if any.
    pub monthly_target_cents: Option<i64>,
    /// `monthly_target_cents` scaled to the report window length.
    pub prorated_budget_cents: Option<i64>,
    pub actual_cents: i64,
    /// `actual − prorated_budget` (positive = overspent). None when the
    /// category has no target to compare against.
    pub delta_cents: Option<i64>,
    /// A naive suggested new monthly target: the window's actual spend
    /// normalized to a month and rounded to the nearest $5. Only emitted
    /// for fixed/variable categories that have a target and are off it by
    /// a material amount; the LLM decides whether to actually recommend
    /// it.
    pub suggested_monthly_target_cents: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeekdayBucket {
    /// 0 = Monday … 6 = Sunday.
    pub weekday: u8,
    pub total_cents: i64,
    /// How many of this weekday fell inside the window (so the caller can
    /// derive a per-occurrence average without re-querying).
    pub occurrences: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpendCycles {
    /// Variable (discretionary) spend only — fixed costs don't have a
    /// behavioral "cycle" worth narrating.
    pub by_weekday: Vec<WeekdayBucket>,
    /// Days 1–10 of the calendar month.
    pub early_month_cents: i64,
    /// Days 11–20.
    pub mid_month_cents: i64,
    /// Day 21 through end of month.
    pub late_month_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CutCandidate {
    pub category_id: i64,
    pub name: String,
    pub kind: CategoryKind,
    pub total_cents: i64,
    /// Share of total spend in the window, 0–100.
    pub pct_of_total: f64,
    /// Overage vs. the prorated target, if the category has one and is
    /// over it.
    pub over_budget_cents: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionHit {
    pub category_id: i64,
    pub category_name: String,
    /// The repeating charge amount.
    pub amount_cents: i64,
    pub occurrences: u32,
    pub total_cents: i64,
    /// "flagged" = the category is marked recurring; "detected" = a
    /// repeated identical amount was found heuristically.
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct SavingsSummary {
    pub monthly_income_cents: i64,
    pub months_in_window: f64,
    pub income_for_window_cents: i64,
    pub spent_cents: i64,
    pub saved_cents: i64,
    /// `saved / income * 100`. None if income for the window is zero.
    pub savings_rate_pct: Option<f64>,
    /// Saved amount normalized to one month.
    pub monthly_net_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryDelta {
    pub category_id: i64,
    pub name: String,
    pub kind: CategoryKind,
    pub current_cents: i64,
    pub prior_cents: i64,
    pub delta_cents: i64,
    /// Percent change vs. the prior equal-length window. None when prior
    /// spend was zero (an infinite increase — narrate as "new").
    pub delta_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WinRow {
    pub category_id: i64,
    pub name: String,
    pub kind: CategoryKind,
    /// "under_budget" or "improved" (vs. the prior window).
    pub reason: &'static str,
    pub detail_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MerchantSample {
    pub category_id: i64,
    pub category_name: String,
    /// Top descriptions by spend within the category. These are
    /// user-entered free text and are only populated when the caller
    /// opts in; Phase 2 wraps them in `<user_data>` before sending.
    pub merchants: Vec<MerchantRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MerchantRow {
    pub label: String,
    pub total_cents: i64,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportData {
    pub timeframe_label: String,
    #[serde(with = "time::serde::rfc3339")]
    pub start: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub end: OffsetDateTime,
    pub days: i64,
    pub months_in_window: f64,
    pub currency: String,
    pub total_spent_cents: i64,
    pub rebalance: Vec<RebalanceRow>,
    pub spend_cycles: SpendCycles,
    pub cut_candidates: Vec<CutCandidate>,
    pub subscriptions: Vec<SubscriptionHit>,
    pub savings: Option<SavingsSummary>,
    pub anomalies: Vec<CategoryDelta>,
    pub wins: Vec<WinRow>,
    pub merchant_samples: Vec<MerchantSample>,
}

fn round_to_nearest(value: i64, step: i64) -> i64 {
    if step <= 0 {
        return value;
    }
    ((value as f64 / step as f64).round() as i64) * step
}

/// Net signed contribution of an expense to a total (refunds subtract).
fn signed(amount_cents: i64, is_refund: bool) -> i64 {
    if is_refund {
        -amount_cents
    } else {
        amount_cents
    }
}

/// Build the full deterministic data bundle for a report. Reads only;
/// performs no network I/O. `monthly_income_cents` enables the savings
/// section; `include_merchant_samples` populates the (privacy-sensitive)
/// merchant rows.
pub fn build_report_data(
    conn: &Connection,
    timeframe: ReportTimeframe,
    now: OffsetDateTime,
    monthly_income_cents: Option<i64>,
    include_merchant_samples: bool,
) -> Result<ReportData> {
    let (start, end) = timeframe.resolve(now);
    let offset = now.offset();
    let span = end - start;
    let days = (span.whole_seconds() / 86_400).max(1);
    let months_in_window = span.whole_seconds() as f64 / (AVG_DAYS_PER_MONTH * 86_400.0);

    let currency = settings::get(conn, "currency")?
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "USD".to_string());

    let cats = categories::list(conn, true)?;
    let cat_by_id: HashMap<i64, &Category> = cats.iter().map(|c| (c.id, c)).collect();

    let exps = expenses::list_in_range(conn, start, end)?;
    let (prior_start, prior_end) = (start - span, start);
    let prior_exps = expenses::list_in_range(conn, prior_start, prior_end)?;

    // Per-category net totals for the window and the prior window.
    let mut by_cat: HashMap<i64, i64> = HashMap::new();
    let mut total_spent: i64 = 0;
    for e in &exps {
        let s = signed(e.amount_cents, e.is_refund);
        total_spent += s;
        if let Some(cid) = e.category_id {
            *by_cat.entry(cid).or_insert(0) += s;
        }
    }
    let mut prior_by_cat: HashMap<i64, i64> = HashMap::new();
    for e in &prior_exps {
        if let Some(cid) = e.category_id {
            *prior_by_cat.entry(cid).or_insert(0) += signed(e.amount_cents, e.is_refund);
        }
    }

    let rebalance = build_rebalance(&cats, &by_cat, months_in_window);
    let spend_cycles = build_spend_cycles(&exps, &cat_by_id, start, end, offset);
    let cut_candidates = build_cut_candidates(&cats, &by_cat, total_spent, months_in_window);
    let subscriptions = build_subscriptions(&exps, &cat_by_id);
    let savings = monthly_income_cents.map(|inc| {
        let income_for_window = (inc as f64 * months_in_window).round() as i64;
        let saved = income_for_window - total_spent;
        SavingsSummary {
            monthly_income_cents: inc,
            months_in_window,
            income_for_window_cents: income_for_window,
            spent_cents: total_spent,
            saved_cents: saved,
            savings_rate_pct: if income_for_window > 0 {
                Some((saved as f64 / income_for_window as f64) * 100.0)
            } else {
                None
            },
            monthly_net_cents: if months_in_window > 0.0 {
                (saved as f64 / months_in_window).round() as i64
            } else {
                0
            },
        }
    });
    let anomalies = build_anomalies(&cats, &by_cat, &prior_by_cat);
    let wins = build_wins(&cats, &by_cat, &prior_by_cat, months_in_window);
    let merchant_samples = if include_merchant_samples {
        build_merchant_samples(&exps, &cat_by_id, &cut_candidates, &anomalies)
    } else {
        Vec::new()
    };

    Ok(ReportData {
        timeframe_label: timeframe.label(now),
        start,
        end,
        days,
        months_in_window,
        currency,
        total_spent_cents: total_spent,
        rebalance,
        spend_cycles,
        cut_candidates,
        subscriptions,
        savings,
        anomalies,
        wins,
        merchant_samples,
    })
}

fn build_rebalance(
    cats: &[Category],
    by_cat: &HashMap<i64, i64>,
    months_in_window: f64,
) -> Vec<RebalanceRow> {
    let mut rows: Vec<RebalanceRow> = cats
        .iter()
        .filter(|c| c.is_active)
        .map(|c| {
            let actual = by_cat.get(&c.id).copied().unwrap_or(0);
            let prorated = c
                .monthly_target_cents
                .map(|t| (t as f64 * months_in_window).round() as i64);
            let delta = prorated.map(|p| actual - p);
            // Suggest a revised monthly target only for fixed/variable
            // categories that have a target and are materially off it
            // (>$10 and >10% of the prorated budget).
            let suggested = match (c.kind, c.monthly_target_cents, prorated, delta) {
                (CategoryKind::Investing, ..) => None,
                (_, Some(_), Some(p), Some(d))
                    if p > 0 && d.abs() > 1_000 && (d.abs() as f64 / p as f64) > 0.10 =>
                {
                    let monthly_actual =
                        (actual as f64 / months_in_window.max(1e-9)).round() as i64;
                    Some(round_to_nearest(monthly_actual.max(0), 500))
                }
                _ => None,
            };
            RebalanceRow {
                category_id: c.id,
                name: c.name.clone(),
                kind: c.kind,
                monthly_target_cents: c.monthly_target_cents,
                prorated_budget_cents: prorated,
                actual_cents: actual,
                delta_cents: delta,
                suggested_monthly_target_cents: suggested,
            }
        })
        .filter(|r| r.actual_cents != 0 || r.monthly_target_cents.is_some())
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.actual_cents));
    rows
}

fn build_spend_cycles(
    exps: &[crate::domain::Expense],
    cat_by_id: &HashMap<i64, &Category>,
    start: OffsetDateTime,
    end: OffsetDateTime,
    offset: time::UtcOffset,
) -> SpendCycles {
    let is_variable = |cid: Option<i64>| {
        cid.and_then(|id| cat_by_id.get(&id))
            .map(|c| c.kind == CategoryKind::Variable)
            .unwrap_or(false)
    };

    let mut weekday_totals = [0i64; 7];
    let (mut early, mut mid, mut late) = (0i64, 0i64, 0i64);
    for e in exps {
        if !is_variable(e.category_id) {
            continue;
        }
        let s = signed(e.amount_cents, e.is_refund);
        let local = e.occurred_at.to_offset(offset);
        let wd = local.weekday().number_days_from_monday() as usize;
        weekday_totals[wd] += s;
        match local.day() {
            1..=10 => early += s,
            11..=20 => mid += s,
            _ => late += s,
        }
    }

    // Count how many of each weekday fall in [start, end) so the UI/LLM
    // can normalize to a per-occurrence average.
    let mut occ = [0u32; 7];
    let mut day = start.to_offset(offset).date();
    let last = (end - Duration::seconds(1)).to_offset(offset).date();
    while day <= last {
        occ[day.weekday().number_days_from_monday() as usize] += 1;
        day += Duration::days(1);
    }

    SpendCycles {
        by_weekday: (0..7)
            .map(|i| WeekdayBucket {
                weekday: i as u8,
                total_cents: weekday_totals[i],
                occurrences: occ[i],
            })
            .collect(),
        early_month_cents: early,
        mid_month_cents: mid,
        late_month_cents: late,
    }
}

fn build_cut_candidates(
    cats: &[Category],
    by_cat: &HashMap<i64, i64>,
    total_spent: i64,
    months_in_window: f64,
) -> Vec<CutCandidate> {
    let mut rows: Vec<CutCandidate> = cats
        .iter()
        .filter(|c| c.is_active && c.kind == CategoryKind::Variable)
        .filter_map(|c| {
            let total = by_cat.get(&c.id).copied().unwrap_or(0);
            if total <= 0 {
                return None;
            }
            let over = c.monthly_target_cents.and_then(|t| {
                let prorated = (t as f64 * months_in_window).round() as i64;
                let diff = total - prorated;
                (diff > 0).then_some(diff)
            });
            Some(CutCandidate {
                category_id: c.id,
                name: c.name.clone(),
                kind: c.kind,
                total_cents: total,
                pct_of_total: if total_spent > 0 {
                    (total as f64 / total_spent as f64) * 100.0
                } else {
                    0.0
                },
                over_budget_cents: over,
            })
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.total_cents));
    rows.truncate(6);
    rows
}

fn build_subscriptions(
    exps: &[crate::domain::Expense],
    cat_by_id: &HashMap<i64, &Category>,
) -> Vec<SubscriptionHit> {
    // Group non-refund, non-investing charges by (category, exact amount).
    // A group is a subscription candidate when the amount repeats often
    // enough, or when the category is explicitly flagged recurring.
    let mut groups: HashMap<(i64, i64), u32> = HashMap::new();
    for e in exps {
        if e.is_refund {
            continue;
        }
        let Some(cid) = e.category_id else { continue };
        let Some(cat) = cat_by_id.get(&cid) else {
            continue;
        };
        if cat.kind == CategoryKind::Investing {
            continue;
        }
        *groups.entry((cid, e.amount_cents)).or_insert(0) += 1;
    }
    let mut hits: Vec<SubscriptionHit> = groups
        .into_iter()
        .filter_map(|((cid, amount), count)| {
            let cat = cat_by_id.get(&cid)?;
            let threshold = if cat.is_recurring { 2 } else { 3 };
            if count < threshold {
                return None;
            }
            Some(SubscriptionHit {
                category_id: cid,
                category_name: cat.name.clone(),
                amount_cents: amount,
                occurrences: count,
                total_cents: amount * count as i64,
                source: if cat.is_recurring {
                    "flagged"
                } else {
                    "detected"
                },
            })
        })
        .collect();
    hits.sort_by_key(|h| std::cmp::Reverse(h.total_cents));
    hits.truncate(12);
    hits
}

fn build_anomalies(
    cats: &[Category],
    by_cat: &HashMap<i64, i64>,
    prior_by_cat: &HashMap<i64, i64>,
) -> Vec<CategoryDelta> {
    let mut rows: Vec<CategoryDelta> = cats
        .iter()
        .filter(|c| c.is_active)
        .filter_map(|c| {
            let current = by_cat.get(&c.id).copied().unwrap_or(0);
            let prior = prior_by_cat.get(&c.id).copied().unwrap_or(0);
            let delta = current - prior;
            // Material change: ≥ $20 absolute, and either brand-new
            // spend or a ≥25% swing.
            let pct = if prior > 0 {
                Some((delta as f64 / prior as f64) * 100.0)
            } else {
                None
            };
            let material = delta.abs() >= 2_000
                && match pct {
                    Some(p) => p.abs() >= 25.0,
                    None => current >= 2_000,
                };
            material.then_some(CategoryDelta {
                category_id: c.id,
                name: c.name.clone(),
                kind: c.kind,
                current_cents: current,
                prior_cents: prior,
                delta_cents: delta,
                delta_pct: pct,
            })
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.delta_cents.abs()));
    rows.truncate(8);
    rows
}

fn build_wins(
    cats: &[Category],
    by_cat: &HashMap<i64, i64>,
    prior_by_cat: &HashMap<i64, i64>,
    months_in_window: f64,
) -> Vec<WinRow> {
    let mut rows: Vec<WinRow> = Vec::new();
    for c in cats.iter().filter(|c| c.is_active) {
        let actual = by_cat.get(&c.id).copied().unwrap_or(0);
        // Under-budget win: fixed/variable with a target, ≥10% under the
        // prorated budget.
        if c.kind != CategoryKind::Investing {
            if let Some(t) = c.monthly_target_cents {
                let prorated = (t as f64 * months_in_window).round() as i64;
                if prorated > 0 && actual >= 0 && actual <= (prorated as f64 * 0.9) as i64 {
                    rows.push(WinRow {
                        category_id: c.id,
                        name: c.name.clone(),
                        kind: c.kind,
                        reason: "under_budget",
                        detail_cents: prorated - actual,
                    });
                    continue;
                }
            }
        }
        // Improvement win: spend dropped ≥25% vs. the prior window, and
        // the prior window had meaningful spend.
        let prior = prior_by_cat.get(&c.id).copied().unwrap_or(0);
        if prior >= 2_000 && actual >= 0 {
            let drop = prior - actual;
            if drop > 0 && (drop as f64 / prior as f64) >= 0.25 {
                rows.push(WinRow {
                    category_id: c.id,
                    name: c.name.clone(),
                    kind: c.kind,
                    reason: "improved",
                    detail_cents: drop,
                });
            }
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.detail_cents));
    rows.truncate(6);
    rows
}

fn build_merchant_samples(
    exps: &[crate::domain::Expense],
    cat_by_id: &HashMap<i64, &Category>,
    cuts: &[CutCandidate],
    anomalies: &[CategoryDelta],
) -> Vec<MerchantSample> {
    use std::collections::BTreeSet;
    // Only sample merchants for categories the report will actually
    // discuss (cut candidates + anomalous categories).
    let mut wanted: BTreeSet<i64> = BTreeSet::new();
    wanted.extend(cuts.iter().map(|c| c.category_id));
    wanted.extend(anomalies.iter().map(|a| a.category_id));

    let mut out = Vec::new();
    for cid in wanted {
        let Some(cat) = cat_by_id.get(&cid) else {
            continue;
        };
        let mut agg: HashMap<String, (i64, u32)> = HashMap::new();
        for e in exps
            .iter()
            .filter(|e| e.category_id == Some(cid) && !e.is_refund)
        {
            let label = e
                .description
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("(no description)")
                .to_string();
            let entry = agg.entry(label).or_insert((0, 0));
            entry.0 += e.amount_cents;
            entry.1 += 1;
        }
        if agg.is_empty() {
            continue;
        }
        let mut merchants: Vec<MerchantRow> = agg
            .into_iter()
            .map(|(label, (total, count))| MerchantRow {
                label,
                total_cents: total,
                count,
            })
            .collect();
        merchants.sort_by_key(|m| std::cmp::Reverse(m.total_cents));
        merchants.truncate(5);
        out.push(MerchantSample {
            category_id: cid,
            category_name: cat.name.clone(),
            merchants,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::domain::{ExpenseSource, NewCategory, NewExpense};
    use crate::repository::{categories, expenses};
    use time::macros::datetime;

    fn fresh_conn() -> Connection {
        let conn = db::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        conn
    }

    fn mk_category(conn: &Connection, name: &str, kind: CategoryKind, target: Option<i64>) -> i64 {
        // Prefix to guarantee no collision with migration-seeded
        // default category names (UNIQUE on categories.name).
        categories::insert(
            conn,
            &NewCategory {
                name: format!("RPT_{name}"),
                kind,
                monthly_target_cents: target,
                is_recurring: false,
                recurrence_day_of_month: None,
            },
        )
        .unwrap()
    }

    fn add(
        conn: &Connection,
        cat: i64,
        cents: i64,
        when: OffsetDateTime,
        desc: Option<&str>,
        is_refund: bool,
    ) {
        expenses::insert(
            conn,
            &NewExpense {
                amount_cents: cents,
                currency: "USD".into(),
                category_id: Some(cat),
                description: desc.map(|s| s.to_string()),
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

    const NOW: OffsetDateTime = datetime!(2026-05-15 12:00:00 UTC);

    #[test]
    fn last_month_resolves_to_previous_calendar_month() {
        let (s, e) = ReportTimeframe::LastMonth.resolve(NOW);
        assert_eq!(s, datetime!(2026-04-01 00:00:00 UTC));
        assert_eq!(e, datetime!(2026-05-01 00:00:00 UTC));
    }

    #[test]
    fn last_week_is_previous_complete_iso_week() {
        // 2026-05-15 is a Friday; this week's Monday is 2026-05-11.
        let (s, e) = ReportTimeframe::LastWeek.resolve(NOW);
        assert_eq!(s, datetime!(2026-05-04 00:00:00 UTC));
        assert_eq!(e, datetime!(2026-05-11 00:00:00 UTC));
    }

    #[test]
    fn last_quarter_is_q1_when_now_in_q2() {
        let (s, e) = ReportTimeframe::LastQuarter.resolve(NOW);
        assert_eq!(s, datetime!(2026-01-01 00:00:00 UTC));
        assert_eq!(e, datetime!(2026-04-01 00:00:00 UTC));
    }

    #[test]
    fn last_year_is_prior_calendar_year() {
        let (s, e) = ReportTimeframe::LastYear.resolve(NOW);
        assert_eq!(s, datetime!(2025-01-01 00:00:00 UTC));
        assert_eq!(e, datetime!(2026-01-01 00:00:00 UTC));
    }

    #[test]
    fn custom_includes_the_to_date() {
        let tf = ReportTimeframe::Custom {
            from: time::macros::date!(2026 - 04 - 10),
            to: time::macros::date!(2026 - 04 - 20),
        };
        let (s, e) = tf.resolve(NOW);
        assert_eq!(s, datetime!(2026-04-10 00:00:00 UTC));
        assert_eq!(e, datetime!(2026-04-21 00:00:00 UTC));
    }

    #[test]
    fn rebalance_flags_chronic_underspend_with_a_suggestion() {
        let conn = fresh_conn();
        let gas = mk_category(&conn, "Gas", CategoryKind::Variable, Some(25_000));
        // April: spend ~$150 against a $250 target.
        add(
            &conn,
            gas,
            7_500,
            datetime!(2026-04-05 10:00 UTC),
            None,
            false,
        );
        add(
            &conn,
            gas,
            7_500,
            datetime!(2026-04-20 10:00 UTC),
            None,
            false,
        );

        let r = build_report_data(&conn, ReportTimeframe::LastMonth, NOW, None, false).unwrap();
        let row = r
            .rebalance
            .iter()
            .find(|x| x.category_id == gas)
            .expect("gas row present");
        assert_eq!(row.actual_cents, 15_000);
        assert_eq!(row.monthly_target_cents, Some(25_000));
        // Underspent by ~$100 → a lower suggested target, rounded to $5.
        let suggestion = row.suggested_monthly_target_cents.expect("suggestion");
        assert!(suggestion % 500 == 0, "rounded to $5");
        assert!(
            (14_000..=16_000).contains(&suggestion),
            "suggestion ~= $150, got {suggestion}"
        );
    }

    #[test]
    fn subscription_radar_detects_repeated_amount_and_flagged_recurring() {
        let conn = fresh_conn();
        let stream = mk_category(&conn, "Streaming", CategoryKind::Fixed, None);
        // Same $15.99 three times in the window → detected.
        for d in [2, 9, 16] {
            add(
                &conn,
                stream,
                1_599,
                datetime!(2026-04-01 00:00 UTC) + Duration::days(d),
                Some("Netflix"),
                false,
            );
        }
        let r = build_report_data(&conn, ReportTimeframe::LastMonth, NOW, None, false).unwrap();
        let hit = r
            .subscriptions
            .iter()
            .find(|s| s.category_id == stream)
            .expect("subscription detected");
        assert_eq!(hit.occurrences, 3);
        assert_eq!(hit.amount_cents, 1_599);
        assert_eq!(hit.source, "detected");
    }

    #[test]
    fn savings_section_only_present_with_income_and_rate_is_correct() {
        let conn = fresh_conn();
        let food = mk_category(&conn, "Food", CategoryKind::Variable, None);
        add(
            &conn,
            food,
            100_000,
            datetime!(2026-04-10 12:00 UTC),
            None,
            false,
        );

        let without =
            build_report_data(&conn, ReportTimeframe::LastMonth, NOW, None, false).unwrap();
        assert!(without.savings.is_none());

        // $4,000/mo income, ~1 month window, $1,000 spent → ~75% saved.
        let with = build_report_data(&conn, ReportTimeframe::LastMonth, NOW, Some(400_000), false)
            .unwrap();
        let s = with.savings.expect("savings present");
        assert_eq!(s.spent_cents, 100_000);
        let rate = s.savings_rate_pct.expect("rate");
        assert!((60.0..=85.0).contains(&rate), "rate ~75%, got {rate}");
    }

    #[test]
    fn merchant_samples_gated_by_opt_in() {
        let conn = fresh_conn();
        let dining = mk_category(&conn, "Dining", CategoryKind::Variable, Some(5_000));
        for _ in 0..3 {
            add(
                &conn,
                dining,
                4_000,
                datetime!(2026-04-12 19:00 UTC),
                Some("DoorDash"),
                false,
            );
        }
        let off = build_report_data(&conn, ReportTimeframe::LastMonth, NOW, None, false).unwrap();
        assert!(off.merchant_samples.is_empty());

        let on = build_report_data(&conn, ReportTimeframe::LastMonth, NOW, None, true).unwrap();
        let sample = on
            .merchant_samples
            .iter()
            .find(|m| m.category_id == dining)
            .expect("dining sampled (it's a cut candidate)");
        assert_eq!(sample.merchants[0].label, "DoorDash");
        assert_eq!(sample.merchants[0].count, 3);
    }

    #[test]
    fn anomalies_compare_against_prior_equal_window() {
        let conn = fresh_conn();
        let travel = mk_category(&conn, "Travel", CategoryKind::Variable, None);
        // Prior month (March): $50. Report month (April): $500 → +900%.
        add(
            &conn,
            travel,
            5_000,
            datetime!(2026-03-10 12:00 UTC),
            None,
            false,
        );
        add(
            &conn,
            travel,
            50_000,
            datetime!(2026-04-10 12:00 UTC),
            None,
            false,
        );
        let r = build_report_data(&conn, ReportTimeframe::LastMonth, NOW, None, false).unwrap();
        let a = r
            .anomalies
            .iter()
            .find(|a| a.category_id == travel)
            .expect("travel anomaly");
        assert_eq!(a.current_cents, 50_000);
        assert_eq!(a.prior_cents, 5_000);
        assert_eq!(a.delta_cents, 45_000);
        assert!(a.delta_pct.unwrap() > 100.0);
    }
}
