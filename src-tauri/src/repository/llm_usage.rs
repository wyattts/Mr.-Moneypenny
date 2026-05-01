//! LLM API usage log: insert + summary aggregations.
//!
//! Each successful chat() response gets one row. The router calls
//! `log()` after every response — Anthropic rows carry a real
//! `cost_micros`; Ollama rows carry zero (free local inference) but
//! still contribute to call counts.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use time::{Date, Duration, Month, OffsetDateTime, Time};

use crate::llm::pricing;
use crate::llm::Usage;

/// Persist one usage row. Best-effort — callers ignore errors so a
/// transient DB blip doesn't fail an otherwise-successful chat turn.
pub fn log(
    conn: &Connection,
    provider: &str,
    model: &str,
    usage: &Usage,
    occurred_at: OffsetDateTime,
) -> Result<()> {
    let cost_micros = pricing::compute_cost_micros(model, usage).unwrap_or(0);
    conn.execute(
        "INSERT INTO llm_usage
            (provider, model, input_tokens, output_tokens,
             cache_read_tokens, cache_creation_tokens,
             cost_micros, occurred_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            provider,
            model,
            usage.input_tokens as i64,
            usage.output_tokens as i64,
            usage.cache_read_input_tokens as i64,
            usage.cache_creation_input_tokens as i64,
            cost_micros,
            occurred_at,
        ],
    )?;
    Ok(())
}

/// Summary buckets for the Settings → API usage panel.
#[derive(Debug, Clone, Serialize)]
pub struct UsageSummary {
    pub today_micros: i64,
    pub this_month_micros: i64,
    pub lifetime_micros: i64,
    pub today_calls: i64,
    pub this_month_calls: i64,
    pub lifetime_calls: i64,
    /// Per-model breakdown over the user's *lifetime* of usage. Sorted
    /// by `cost_micros` descending.
    pub by_model: Vec<ModelSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelSummary {
    pub model: String,
    pub provider: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_micros: i64,
}

/// Compute the summary at `now`. "Today" / "this month" use the offset
/// of `now` so the boundaries match the user's local clock.
pub fn summary(conn: &Connection, now: OffsetDateTime) -> Result<UsageSummary> {
    let offset = now.offset();
    let today_start = now.date().with_time(Time::MIDNIGHT).assume_offset(offset);
    let month_start = Date::from_calendar_date(now.year(), now.month(), 1)
        .expect("day 1 always valid")
        .with_time(Time::MIDNIGHT)
        .assume_offset(offset);
    let next_day = today_start + Duration::days(1);
    let next_month = if now.month() == Month::December {
        Date::from_calendar_date(now.year() + 1, Month::January, 1)
    } else {
        Date::from_calendar_date(now.year(), now.month().next(), 1)
    }
    .expect("first of next month is valid")
    .with_time(Time::MIDNIGHT)
    .assume_offset(offset);

    let (today_micros, today_calls) = sum_window(conn, today_start, next_day)?;
    let (this_month_micros, this_month_calls) = sum_window(conn, month_start, next_month)?;
    let (lifetime_micros, lifetime_calls) = sum_lifetime(conn)?;
    let by_model = per_model(conn)?;

    Ok(UsageSummary {
        today_micros,
        this_month_micros,
        lifetime_micros,
        today_calls,
        this_month_calls,
        lifetime_calls,
        by_model,
    })
}

fn sum_window(conn: &Connection, start: OffsetDateTime, end: OffsetDateTime) -> Result<(i64, i64)> {
    let mut stmt = conn.prepare_cached(
        "SELECT COALESCE(SUM(cost_micros), 0), COUNT(*)
         FROM llm_usage
         WHERE occurred_at >= ?1 AND occurred_at < ?2",
    )?;
    let row: (i64, i64) = stmt.query_row(params![start, end], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(row)
}

fn sum_lifetime(conn: &Connection) -> Result<(i64, i64)> {
    let mut stmt =
        conn.prepare_cached("SELECT COALESCE(SUM(cost_micros), 0), COUNT(*) FROM llm_usage")?;
    let row: (i64, i64) = stmt.query_row([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(row)
}

fn per_model(conn: &Connection) -> Result<Vec<ModelSummary>> {
    let mut stmt = conn.prepare_cached(
        "SELECT model, provider,
                COUNT(*) AS calls,
                COALESCE(SUM(input_tokens), 0)  AS input_total,
                COALESCE(SUM(output_tokens), 0) AS output_total,
                COALESCE(SUM(cost_micros), 0)   AS cost_total
         FROM llm_usage
         GROUP BY model, provider
         ORDER BY cost_total DESC, model ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ModelSummary {
                model: r.get(0)?,
                provider: r.get(1)?,
                calls: r.get(2)?,
                input_tokens: r.get(3)?,
                output_tokens: r.get(4)?,
                cost_micros: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use time::macros::datetime;

    fn fresh() -> Connection {
        let c = db::open_in_memory().unwrap();
        db::migrate(&c).unwrap();
        c
    }

    fn u(input: u32, output: u32) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            ..Default::default()
        }
    }

    #[test]
    fn empty_summary_is_all_zero() {
        let c = fresh();
        let now = datetime!(2026-04-15 12:00:00 UTC);
        let s = summary(&c, now).unwrap();
        assert_eq!(s.today_micros, 0);
        assert_eq!(s.this_month_micros, 0);
        assert_eq!(s.lifetime_micros, 0);
        assert_eq!(s.today_calls, 0);
        assert_eq!(s.lifetime_calls, 0);
        assert!(s.by_model.is_empty());
    }

    #[test]
    fn log_persists_with_computed_cost() {
        let c = fresh();
        let now = datetime!(2026-04-15 12:00:00 UTC);
        log(&c, "anthropic", "claude-haiku-4-5", &u(5_000, 500), now).unwrap();
        let s = summary(&c, now).unwrap();
        assert_eq!(s.today_calls, 1);
        assert_eq!(s.today_micros, 7_500);
        assert_eq!(s.lifetime_micros, 7_500);
    }

    #[test]
    fn log_unknown_model_records_zero_cost() {
        let c = fresh();
        let now = datetime!(2026-04-15 12:00:00 UTC);
        log(&c, "ollama", "llama3:8b", &u(1000, 200), now).unwrap();
        let s = summary(&c, now).unwrap();
        assert_eq!(s.today_calls, 1);
        assert_eq!(s.today_micros, 0);
    }

    #[test]
    fn today_and_month_windows_distinguish_old_rows() {
        let c = fresh();
        let now = datetime!(2026-04-15 12:00:00 UTC);
        // Yesterday — counts in month, not today
        log(
            &c,
            "anthropic",
            "claude-haiku-4-5",
            &u(1000, 0),
            datetime!(2026-04-14 12:00:00 UTC),
        )
        .unwrap();
        // Last month — counts in lifetime only
        log(
            &c,
            "anthropic",
            "claude-haiku-4-5",
            &u(1000, 0),
            datetime!(2026-03-30 12:00:00 UTC),
        )
        .unwrap();
        // Today
        log(&c, "anthropic", "claude-haiku-4-5", &u(1000, 0), now).unwrap();

        let s = summary(&c, now).unwrap();
        assert_eq!(s.today_calls, 1);
        assert_eq!(s.this_month_calls, 2);
        assert_eq!(s.lifetime_calls, 3);
    }

    #[test]
    fn per_model_breakdown_groups_by_model() {
        let c = fresh();
        let now = datetime!(2026-04-15 12:00:00 UTC);
        log(&c, "anthropic", "claude-haiku-4-5", &u(1000, 100), now).unwrap();
        log(&c, "anthropic", "claude-haiku-4-5", &u(2000, 200), now).unwrap();
        log(&c, "anthropic", "claude-sonnet-4-5", &u(1000, 100), now).unwrap();
        log(&c, "ollama", "llama3:8b", &u(5000, 500), now).unwrap();

        let s = summary(&c, now).unwrap();
        assert_eq!(s.by_model.len(), 3);
        // Sonnet most expensive per token, but only one call vs two haiku calls.
        // Haiku 1: 3000 in + 300 out = 0.003 + 0.0015 = $0.0045 = 4500 micros
        // Sonnet: 1000 in + 100 out = 0.003 + 0.0015 = $0.0045 = 4500 micros (tie)
        // Ollama: 0
        // Order: by cost desc, then model asc
        // Haiku and Sonnet are tied at 4500; haiku < sonnet alphabetically.
        let names: Vec<&str> = s.by_model.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(
            names,
            vec!["claude-haiku-4-5", "claude-sonnet-4-5", "llama3:8b"]
        );
        let haiku = &s.by_model[0];
        assert_eq!(haiku.calls, 2);
        assert_eq!(haiku.input_tokens, 3000);
    }
}
