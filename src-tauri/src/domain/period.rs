//! Period pacing math — the "don't say terrible on the 2nd" logic.
//!
//! This module is shared between the LLM `summarize_period` tool and the
//! GUI insights dashboard so they cannot disagree about how the user is
//! doing this month.
//!
//! The core insight: do NOT pace total spend against total budget. On day
//! 2 with rent posted, the naive ratio looks awful — but rent was always
//! going to be paid. Pace **variable** spending against the **variable**
//! budget; treat fixed expenses as inevitable.

use serde::Serialize;
use time::{Date, Month, OffsetDateTime, Time};

/// A point-in-time snapshot of where the user is in the current month.
/// Caller passes `now` in their preferred timezone; bounds are computed
/// in that same offset (so "the 1st" is the user's local 1st).
#[derive(Debug, Clone, Serialize)]
pub struct PeriodSnapshot {
    /// Fraction of the current calendar month elapsed, 0.0 to 1.0.
    pub progress: f64,
    /// 1-indexed calendar day of month at `now`.
    pub day_of_month: u8,
    /// 28..=31 depending on the month.
    pub days_in_period: u8,
    /// Days remaining including today (so on the 30th of a 30-day month, == 1).
    pub days_remaining: u8,

    pub fixed_budget_cents: i64,
    pub fixed_actual_cents: i64,
    /// Fixed spend not yet posted this month. `(budget - actual).max(0)`.
    pub fixed_pending_cents: i64,

    pub variable_budget_cents: i64,
    pub variable_spent_cents: i64,
    pub variable_remaining_cents: i64,
    /// What the user "should" have spent by `now` if pacing linearly.
    pub variable_pace_expected_cents: i64,
    /// True if variable spend is within 10% of expected pace.
    pub on_pace: bool,
    /// Variable budget left, divided evenly over remaining days (incl. today).
    pub daily_variable_allowance_cents: i64,
}

/// Inclusive start (00:00 of the 1st), exclusive end (00:00 of next month).
/// Bounds use `now.offset()`, so the caller controls the timezone of the
/// month boundary.
pub fn current_month_bounds(now: OffsetDateTime) -> (OffsetDateTime, OffsetDateTime) {
    let date = now.date();
    let start_date = Date::from_calendar_date(date.year(), date.month(), 1)
        .expect("day 1 of any month is always valid");
    let next_month_date = if date.month() == Month::December {
        Date::from_calendar_date(date.year() + 1, Month::January, 1)
    } else {
        Date::from_calendar_date(date.year(), date.month().next(), 1)
    }
    .expect("day 1 of next month is always valid");
    let start = start_date.with_time(Time::MIDNIGHT).assume_offset(now.offset());
    let end = next_month_date
        .with_time(Time::MIDNIGHT)
        .assume_offset(now.offset());
    (start, end)
}

pub fn compute_snapshot(
    now: OffsetDateTime,
    fixed_budget_cents: i64,
    fixed_actual_cents: i64,
    variable_budget_cents: i64,
    variable_spent_cents: i64,
) -> PeriodSnapshot {
    let (start, end) = current_month_bounds(now);
    let total_secs = (end - start).whole_seconds().max(1) as f64;
    let elapsed_secs = ((now - start).whole_seconds().max(0)) as f64;
    let progress = (elapsed_secs / total_secs).clamp(0.0, 1.0);

    let days_in_period = (end - start).whole_days() as u8;
    let day_of_month = now.day();
    let days_remaining = (days_in_period as i32 - day_of_month as i32 + 1).max(0) as u8;

    let fixed_pending_cents = (fixed_budget_cents - fixed_actual_cents).max(0);
    let variable_remaining_cents = (variable_budget_cents - variable_spent_cents).max(0);
    let variable_pace_expected_cents = (variable_budget_cents as f64 * progress).round() as i64;
    // 10% grace before declaring off-pace.
    let pace_threshold = ((variable_pace_expected_cents as f64) * 1.1).ceil() as i64;
    let on_pace = variable_spent_cents <= pace_threshold;
    let daily_variable_allowance_cents = if days_remaining > 0 {
        variable_remaining_cents / days_remaining as i64
    } else {
        0
    };

    PeriodSnapshot {
        progress,
        day_of_month,
        days_in_period,
        days_remaining,
        fixed_budget_cents,
        fixed_actual_cents,
        fixed_pending_cents,
        variable_budget_cents,
        variable_spent_cents,
        variable_remaining_cents,
        variable_pace_expected_cents,
        on_pace,
        daily_variable_allowance_cents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn bounds_for_april() {
        // April 15, 2026 at noon UTC
        let now = datetime!(2026-04-15 12:00:00 UTC);
        let (start, end) = current_month_bounds(now);
        assert_eq!(start, datetime!(2026-04-01 00:00:00 UTC));
        assert_eq!(end, datetime!(2026-05-01 00:00:00 UTC));
    }

    #[test]
    fn bounds_wrap_year_in_december() {
        let now = datetime!(2026-12-31 23:59:59 UTC);
        let (start, end) = current_month_bounds(now);
        assert_eq!(start, datetime!(2026-12-01 00:00:00 UTC));
        assert_eq!(end, datetime!(2027-01-01 00:00:00 UTC));
    }

    #[test]
    fn rent_posted_on_day_two_is_not_terrible() {
        // Day 2 of April (30 days). Rent ($1500) just posted; $30 of
        // variable spent against an $800 variable budget. The user
        // should be ON PACE — rent was always going to happen.
        let now = datetime!(2026-04-02 12:00:00 UTC);
        let snap = compute_snapshot(now, 150_000, 150_000, 80_000, 3_000);

        assert_eq!(snap.day_of_month, 2);
        assert_eq!(snap.days_in_period, 30);
        assert_eq!(snap.days_remaining, 29);
        assert_eq!(snap.fixed_pending_cents, 0); // rent fully posted
        assert_eq!(snap.variable_remaining_cents, 77_000);
        assert!(
            snap.on_pace,
            "$30 spent on day 2 of $800/month variable should be ON pace; \
             snap: {snap:#?}",
        );
        // ~5% of month elapsed → expected pace ~$40. $30 < $44 (10% grace) → on pace.
        assert!(snap.variable_pace_expected_cents <= 5_000);
    }

    #[test]
    fn very_overspent_is_off_pace() {
        // Day 5 of a 30-day month. $700 spent of an $800 variable budget.
        // Expected pace at day 5 ≈ $133. $700 >> $147 → off pace.
        let now = datetime!(2026-04-05 12:00:00 UTC);
        let snap = compute_snapshot(now, 150_000, 0, 80_000, 70_000);
        assert!(!snap.on_pace);
        assert_eq!(snap.variable_remaining_cents, 10_000);
    }

    #[test]
    fn last_day_of_month() {
        let now = datetime!(2026-04-30 23:00:00 UTC);
        let snap = compute_snapshot(now, 0, 0, 80_000, 75_000);
        assert_eq!(snap.day_of_month, 30);
        assert_eq!(snap.days_remaining, 1); // today is the only day left
        assert_eq!(snap.daily_variable_allowance_cents, 5_000);
    }

    #[test]
    fn zero_variable_budget_does_not_panic() {
        let now = datetime!(2026-04-15 12:00:00 UTC);
        let snap = compute_snapshot(now, 0, 0, 0, 0);
        assert_eq!(snap.daily_variable_allowance_cents, 0);
        assert_eq!(snap.variable_pace_expected_cents, 0);
        assert!(snap.on_pace);
    }

    #[test]
    fn progress_at_month_start_is_zero() {
        let now = datetime!(2026-04-01 00:00:00 UTC);
        let snap = compute_snapshot(now, 0, 0, 0, 0);
        assert_eq!(snap.progress, 0.0);
    }

    #[test]
    fn progress_at_month_midpoint() {
        // April 15 midnight = exactly 14 days into a 30-day month
        let now = datetime!(2026-04-15 00:00:00 UTC);
        let snap = compute_snapshot(now, 0, 0, 0, 0);
        assert!((snap.progress - 14.0 / 30.0).abs() < 1e-9);
    }
}
