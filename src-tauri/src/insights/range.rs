//! Date range selection for the insights dashboard.
//!
//! All ranges resolve to a half-open interval `[start, end)` in the same
//! timezone offset as the `now` passed in. Bounds are stored in the DB
//! as ISO8601 strings; comparison there is lexicographic, which works
//! correctly because we always emit the same offset.

use serde::{Deserialize, Serialize};
use time::{Date, Duration, Month, OffsetDateTime, Time, Weekday};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DateRange {
    ThisWeek,
    ThisMonth,
    ThisQuarter,
    ThisYear,
    Ytd,
    /// Inclusive range; `to` is the last day to include.
    Custom {
        from: Date,
        to: Date,
    },
}

impl DateRange {
    pub fn resolve(&self, now: OffsetDateTime) -> (OffsetDateTime, OffsetDateTime) {
        let offset = now.offset();
        let today = now.date();
        match *self {
            DateRange::ThisWeek => {
                // ISO week: Monday is the first day.
                let days_since_monday = match today.weekday() {
                    Weekday::Monday => 0,
                    Weekday::Tuesday => 1,
                    Weekday::Wednesday => 2,
                    Weekday::Thursday => 3,
                    Weekday::Friday => 4,
                    Weekday::Saturday => 5,
                    Weekday::Sunday => 6,
                };
                let monday = today - Duration::days(days_since_monday as i64);
                let next_monday = monday + Duration::days(7);
                (
                    monday.with_time(Time::MIDNIGHT).assume_offset(offset),
                    next_monday.with_time(Time::MIDNIGHT).assume_offset(offset),
                )
            }
            DateRange::ThisMonth => {
                let start = Date::from_calendar_date(today.year(), today.month(), 1)
                    .expect("day 1 always valid");
                let next = if today.month() == Month::December {
                    Date::from_calendar_date(today.year() + 1, Month::January, 1)
                } else {
                    Date::from_calendar_date(today.year(), today.month().next(), 1)
                }
                .expect("next month day 1 always valid");
                (
                    start.with_time(Time::MIDNIGHT).assume_offset(offset),
                    next.with_time(Time::MIDNIGHT).assume_offset(offset),
                )
            }
            DateRange::ThisQuarter => {
                let q_start_month = match today.month() {
                    Month::January | Month::February | Month::March => Month::January,
                    Month::April | Month::May | Month::June => Month::April,
                    Month::July | Month::August | Month::September => Month::July,
                    Month::October | Month::November | Month::December => Month::October,
                };
                let q_end_month_after = match q_start_month {
                    Month::January => Month::April,
                    Month::April => Month::July,
                    Month::July => Month::October,
                    Month::October => Month::January,
                    _ => unreachable!(),
                };
                let start = Date::from_calendar_date(today.year(), q_start_month, 1).unwrap();
                let end_year = if q_end_month_after == Month::January {
                    today.year() + 1
                } else {
                    today.year()
                };
                let end = Date::from_calendar_date(end_year, q_end_month_after, 1).unwrap();
                (
                    start.with_time(Time::MIDNIGHT).assume_offset(offset),
                    end.with_time(Time::MIDNIGHT).assume_offset(offset),
                )
            }
            DateRange::ThisYear => {
                let start = Date::from_calendar_date(today.year(), Month::January, 1).unwrap();
                let next = Date::from_calendar_date(today.year() + 1, Month::January, 1).unwrap();
                (
                    start.with_time(Time::MIDNIGHT).assume_offset(offset),
                    next.with_time(Time::MIDNIGHT).assume_offset(offset),
                )
            }
            DateRange::Ytd => {
                // YTD = Jan 1 of this year through end of today (exclusive
                // start of tomorrow).
                let start = Date::from_calendar_date(today.year(), Month::January, 1).unwrap();
                let tomorrow = today + Duration::days(1);
                (
                    start.with_time(Time::MIDNIGHT).assume_offset(offset),
                    tomorrow.with_time(Time::MIDNIGHT).assume_offset(offset),
                )
            }
            DateRange::Custom { from, to } => {
                let next_after_to = to + Duration::days(1);
                (
                    from.with_time(Time::MIDNIGHT).assume_offset(offset),
                    next_after_to
                        .with_time(Time::MIDNIGHT)
                        .assume_offset(offset),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    #[test]
    fn this_week_starts_monday() {
        // April 28, 2026 is a Tuesday
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let (start, end) = DateRange::ThisWeek.resolve(now);
        assert_eq!(start, datetime!(2026-04-27 00:00:00 UTC)); // Monday
        assert_eq!(end, datetime!(2026-05-04 00:00:00 UTC)); // Next Monday
    }

    #[test]
    fn this_week_on_sunday() {
        // May 3, 2026 is a Sunday
        let now = datetime!(2026-05-03 23:59:00 UTC);
        let (start, end) = DateRange::ThisWeek.resolve(now);
        assert_eq!(start, datetime!(2026-04-27 00:00:00 UTC)); // Monday before
        assert_eq!(end, datetime!(2026-05-04 00:00:00 UTC));
    }

    #[test]
    fn this_month_april() {
        let now = datetime!(2026-04-15 09:00:00 UTC);
        let (start, end) = DateRange::ThisMonth.resolve(now);
        assert_eq!(start, datetime!(2026-04-01 00:00:00 UTC));
        assert_eq!(end, datetime!(2026-05-01 00:00:00 UTC));
    }

    #[test]
    fn this_month_december_wraps() {
        let now = datetime!(2026-12-25 09:00:00 UTC);
        let (start, end) = DateRange::ThisMonth.resolve(now);
        assert_eq!(start, datetime!(2026-12-01 00:00:00 UTC));
        assert_eq!(end, datetime!(2027-01-01 00:00:00 UTC));
    }

    #[test]
    fn quarter_q2() {
        let now = datetime!(2026-05-15 12:00:00 UTC);
        let (start, end) = DateRange::ThisQuarter.resolve(now);
        assert_eq!(start, datetime!(2026-04-01 00:00:00 UTC));
        assert_eq!(end, datetime!(2026-07-01 00:00:00 UTC));
    }

    #[test]
    fn quarter_q4_wraps_year() {
        let now = datetime!(2026-11-30 12:00:00 UTC);
        let (start, end) = DateRange::ThisQuarter.resolve(now);
        assert_eq!(start, datetime!(2026-10-01 00:00:00 UTC));
        assert_eq!(end, datetime!(2027-01-01 00:00:00 UTC));
    }

    #[test]
    fn ytd_includes_today() {
        let now = datetime!(2026-04-28 18:00:00 UTC);
        let (start, end) = DateRange::Ytd.resolve(now);
        assert_eq!(start, datetime!(2026-01-01 00:00:00 UTC));
        assert_eq!(end, datetime!(2026-04-29 00:00:00 UTC));
    }

    #[test]
    fn custom_includes_to_date() {
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let r = DateRange::Custom {
            from: date!(2026 - 03 - 01),
            to: date!(2026 - 03 - 31),
        };
        let (start, end) = r.resolve(now);
        assert_eq!(start, datetime!(2026-03-01 00:00:00 UTC));
        // March 31 is the last day to include → end is April 1
        assert_eq!(end, datetime!(2026-04-01 00:00:00 UTC));
    }
}
