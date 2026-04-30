//! Recurring expense rules.
//!
//! Each rule is a user spec like "Netflix $15.49 monthly on the 7th" or
//! "trash $20 weekly on Thursday". The scheduler fires it on its due
//! date; mode determines whether the rule logs silently (`Auto`) or
//! asks the bot user to confirm (`Confirm`).

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Serialize};
use time::{Date, Duration, Month, OffsetDateTime, Time};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Frequency {
    Monthly,
    Weekly,
    Yearly,
}

impl Frequency {
    pub fn as_str(&self) -> &'static str {
        match self {
            Frequency::Monthly => "monthly",
            Frequency::Weekly => "weekly",
            Frequency::Yearly => "yearly",
        }
    }
}

impl std::str::FromStr for Frequency {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "monthly" => Frequency::Monthly,
            "weekly" => Frequency::Weekly,
            "yearly" => Frequency::Yearly,
            other => anyhow::bail!("unknown frequency: {other}"),
        })
    }
}

impl ToSql for Frequency {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for Frequency {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e: anyhow::Error| FromSqlError::Other(e.into()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecurringMode {
    /// Bot DMs "yes / no / skip" before logging.
    Confirm,
    /// Logs immediately without confirmation.
    Auto,
}

impl RecurringMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecurringMode::Confirm => "confirm",
            RecurringMode::Auto => "auto",
        }
    }
}

impl std::str::FromStr for RecurringMode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "confirm" => RecurringMode::Confirm,
            "auto" => RecurringMode::Auto,
            other => anyhow::bail!("unknown recurring mode: {other}"),
        })
    }
}

impl ToSql for RecurringMode {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for RecurringMode {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e: anyhow::Error| FromSqlError::Other(e.into()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecurringRule {
    pub id: i64,
    pub label: String,
    pub amount_cents: i64,
    pub currency: String,
    pub category_id: i64,
    pub frequency: Frequency,
    pub anchor_day: u16,
    pub mode: RecurringMode,
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewRecurringRule {
    pub label: String,
    pub amount_cents: i64,
    pub currency: String,
    pub category_id: i64,
    pub frequency: Frequency,
    pub anchor_day: u16,
    pub mode: RecurringMode,
}

/// Compute the next firing time strictly *after* `now`, given the rule's
/// frequency and anchor. Result is at midnight (00:00) in the supplied
/// offset on the firing date.
///
/// Monthly: anchor is day-of-month, clamped to the last day if the
/// target month is shorter (e.g., anchor=31 in February → 28th).
/// Weekly: anchor is ISO weekday (1=Mon..7=Sun).
/// Yearly: anchor is day-of-year (1..=366), clamped to 365/366.
pub fn next_due(frequency: Frequency, anchor_day: u16, now: OffsetDateTime) -> OffsetDateTime {
    let offset = now.offset();
    let today = now.date();
    match frequency {
        Frequency::Monthly => {
            let candidate_this_month = clamp_to_last_day(today.year(), today.month(), anchor_day);
            let dt = candidate_this_month
                .with_time(Time::MIDNIGHT)
                .assume_offset(offset);
            if dt > now {
                return dt;
            }
            // Roll to next month.
            let (next_year, next_month) = if today.month() == Month::December {
                (today.year() + 1, Month::January)
            } else {
                (today.year(), today.month().next())
            };
            let next_date = clamp_to_last_day(next_year, next_month, anchor_day);
            next_date.with_time(Time::MIDNIGHT).assume_offset(offset)
        }
        Frequency::Weekly => {
            // anchor: 1=Mon..7=Sun. time crate: Date::weekday gives Weekday.
            let target = anchor_day.clamp(1, 7) as i64;
            let cur = today.weekday().number_from_monday() as i64;
            let mut delta = target - cur;
            if delta < 0 {
                delta += 7;
            }
            let candidate = today + Duration::days(delta);
            let dt = candidate.with_time(Time::MIDNIGHT).assume_offset(offset);
            if dt > now {
                dt
            } else {
                (candidate + Duration::days(7))
                    .with_time(Time::MIDNIGHT)
                    .assume_offset(offset)
            }
        }
        Frequency::Yearly => {
            let max_day = if is_leap(today.year()) { 366 } else { 365 };
            let target = anchor_day.min(max_day) as i64;
            let candidate = Date::from_ordinal_date(today.year(), target as u16)
                .expect("clamped target is valid");
            let dt = candidate.with_time(Time::MIDNIGHT).assume_offset(offset);
            if dt > now {
                return dt;
            }
            let next_year = today.year() + 1;
            let next_max = if is_leap(next_year) { 366 } else { 365 };
            let next_target = anchor_day.min(next_max);
            Date::from_ordinal_date(next_year, next_target)
                .expect("ordinal day always valid")
                .with_time(Time::MIDNIGHT)
                .assume_offset(offset)
        }
    }
}

fn clamp_to_last_day(year: i32, month: Month, anchor_day: u16) -> Date {
    let last = month.length(year);
    let day = (anchor_day as u8).min(last);
    Date::from_calendar_date(year, month, day).expect("clamped day in range")
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn monthly_anchor_inside_month_returns_this_month() {
        let now = datetime!(2026-04-05 12:00:00 UTC);
        let due = next_due(Frequency::Monthly, 15, now);
        assert_eq!(due, datetime!(2026-04-15 00:00:00 UTC));
    }

    #[test]
    fn monthly_anchor_already_passed_rolls_forward() {
        let now = datetime!(2026-04-20 12:00:00 UTC);
        let due = next_due(Frequency::Monthly, 15, now);
        assert_eq!(due, datetime!(2026-05-15 00:00:00 UTC));
    }

    #[test]
    fn monthly_anchor_31_clamps_to_february_28() {
        let now = datetime!(2027-01-31 12:00:00 UTC); // 2027 is not a leap year
        let due = next_due(Frequency::Monthly, 31, now);
        assert_eq!(due, datetime!(2027-02-28 00:00:00 UTC));
    }

    #[test]
    fn monthly_anchor_31_in_leap_year_february_29() {
        let now = datetime!(2028-01-31 12:00:00 UTC); // 2028 IS a leap year
        let due = next_due(Frequency::Monthly, 31, now);
        assert_eq!(due, datetime!(2028-02-29 00:00:00 UTC));
    }

    #[test]
    fn weekly_anchor_advances_within_week() {
        // Wed Apr 15, 2026; anchor = Friday (5) → Fri Apr 17.
        let now = datetime!(2026-04-15 12:00:00 UTC);
        let due = next_due(Frequency::Weekly, 5, now);
        assert_eq!(due, datetime!(2026-04-17 00:00:00 UTC));
    }

    #[test]
    fn weekly_anchor_already_passed_rolls_to_next_week() {
        // Fri Apr 17 noon; anchor = Wednesday (3) → next Wed Apr 22.
        let now = datetime!(2026-04-17 12:00:00 UTC);
        let due = next_due(Frequency::Weekly, 3, now);
        assert_eq!(due, datetime!(2026-04-22 00:00:00 UTC));
    }

    #[test]
    fn yearly_anchor_inside_year_returns_this_year() {
        let now = datetime!(2026-03-15 12:00:00 UTC);
        let due = next_due(Frequency::Yearly, 200, now); // day 200 = Jul 19
        assert_eq!(due, datetime!(2026-07-19 00:00:00 UTC));
    }

    #[test]
    fn yearly_anchor_passed_rolls_to_next_year() {
        let now = datetime!(2026-08-01 12:00:00 UTC);
        let due = next_due(Frequency::Yearly, 200, now);
        assert_eq!(due, datetime!(2027-07-19 00:00:00 UTC));
    }
}
