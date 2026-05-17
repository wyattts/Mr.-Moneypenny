// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//! The "open hub" — a one-way exit door for the user's own data.
//!
//! This module produces a complete, faithful **photocopy** of the
//! ledger in a format and timeframe the user picks. It is the only
//! supported way data leaves Mr. Moneypenny in bulk; it is initiated by
//! the user from the desktop app, writes to a path the user chooses,
//! and contacts no network and no third party.
//!
//! Design commitments (see docs/privacy.md):
//!
//! - **Snapshot, not a live feed.** Every export is a point-in-time read
//!   taken when the user asks. There is no daemon and no port.
//! - **One-way.** Export is a portability guarantee ("you can always
//!   leave with a perfect copy"), not a re-import pipeline.
//! - **Reads only the published contract.** All formats read the stable
//!   `v_ledger_v1` view (migration 0017), never the raw tables, so the
//!   internal schema can change without breaking exports.
//! - **Timeframe reuses the single source of truth.** Month / quarter /
//!   year bounds come from [`crate::insights::range::DateRange`], the
//!   same logic the dashboard and the bot use, so an exported "this
//!   year" matches what the app shows. `All` applies no date filter.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::Serialize;
use time::OffsetDateTime;

use crate::insights::range::DateRange;

/// The three formats the user can pick (one per export).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Ordinary spreadsheet — opens in Excel / Numbers / LibreOffice.
    Csv,
    /// Programmer-friendly: one JSON object per line (NDJSON).
    Jsonl,
    /// Plain-text accounting — plugs into the Beancount/Fava ecosystem.
    Beancount,
}

impl ExportFormat {
    /// Conventional file extension (no leading dot).
    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Jsonl => "jsonl",
            ExportFormat::Beancount => "beancount",
        }
    }
}

impl std::str::FromStr for ExportFormat {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "csv" => Ok(ExportFormat::Csv),
            "jsonl" => Ok(ExportFormat::Jsonl),
            "beancount" => Ok(ExportFormat::Beancount),
            other => anyhow::bail!("unknown export format: {other}"),
        }
    }
}

/// Which slice of history to export. `All` is the default and the
/// primary, on-ethos choice (the full portability guarantee); the rest
/// are conveniences for taxes / handing a year to an accountant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportTimeframe {
    All,
    ThisMonth,
    ThisQuarter,
    ThisYear,
}

impl ExportTimeframe {
    /// Half-open `[start, end)` bounds in `now`'s offset, or `None` for
    /// `All` (no date filter — the complete history).
    fn bounds(&self, now: OffsetDateTime) -> Option<(OffsetDateTime, OffsetDateTime)> {
        let range = match self {
            ExportTimeframe::All => return None,
            ExportTimeframe::ThisMonth => DateRange::ThisMonth,
            ExportTimeframe::ThisQuarter => DateRange::ThisQuarter,
            ExportTimeframe::ThisYear => DateRange::ThisYear,
        };
        Some(range.resolve(now))
    }

    /// Human label for the export provenance header.
    fn label(&self) -> &'static str {
        match self {
            ExportTimeframe::All => "all time",
            ExportTimeframe::ThisMonth => "this month",
            ExportTimeframe::ThisQuarter => "this quarter",
            ExportTimeframe::ThisYear => "this year",
        }
    }
}

impl std::str::FromStr for ExportTimeframe {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "all" => Ok(ExportTimeframe::All),
            "month" => Ok(ExportTimeframe::ThisMonth),
            "quarter" => Ok(ExportTimeframe::ThisQuarter),
            "year" => Ok(ExportTimeframe::ThisYear),
            other => anyhow::bail!("unknown export timeframe: {other}"),
        }
    }
}

/// One ledger row as read from the stable `v_ledger_v1` contract. The
/// `serde` derive is the JSON Lines shape; the field order here is the
/// column order everywhere.
#[derive(Debug, Clone, Serialize)]
pub struct LedgerRow {
    pub expense_id: i64,
    pub occurred_at: String,
    pub created_at: String,
    pub amount_cents: i64,
    pub signed_amount_cents: i64,
    pub currency: String,
    pub is_refund: bool,
    pub refund_for_expense_id: Option<i64>,
    pub category_name: Option<String>,
    pub category_kind: Option<String>,
    pub category_monthly_target_cents: Option<i64>,
    pub description: Option<String>,
    pub source: String,
    pub logged_by: Option<String>,
}

/// What an export produced: the bytes to write plus the row count, so
/// the UI can tell the user exactly how much left ("Exported 412
/// transactions").
pub struct ExportResult {
    pub bytes: Vec<u8>,
    pub row_count: usize,
}

/// Self-describing provenance for the Beancount header — so a copy
/// stays understandable years from now, without the app. Embedded only
/// in Beancount (it's comments there); CSV and JSON Lines stay pure so
/// their parsers don't choke, with provenance living in the filename
/// and docs/privacy.md instead.
struct Provenance {
    app_version: &'static str,
    schema: &'static str,
    timeframe: &'static str,
    generated_at: String,
    row_count: usize,
}

/// Read the chosen slice from `v_ledger_v1` and serialize it. Pure:
/// touches only the connection it is given, never the network or any
/// path. Caller (the Tauri command) owns writing the bytes to disk.
pub fn export(
    conn: &Connection,
    format: ExportFormat,
    timeframe: ExportTimeframe,
    now: OffsetDateTime,
) -> Result<ExportResult> {
    let rows = read_rows(conn, timeframe, now)?;
    let bytes = match format {
        ExportFormat::Csv => to_csv(&rows)?,
        ExportFormat::Jsonl => to_jsonl(&rows)?,
        ExportFormat::Beancount => {
            let prov = Provenance {
                app_version: env!("CARGO_PKG_VERSION"),
                schema: "v_ledger_v1",
                timeframe: timeframe.label(),
                generated_at: now
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_else(|_| now.unix_timestamp().to_string()),
                row_count: rows.len(),
            };
            to_beancount(&rows, &prov)
        }
    };
    Ok(ExportResult {
        bytes,
        row_count: rows.len(),
    })
}

const SELECT_COLS: &str = "expense_id, occurred_at, created_at, amount_cents, \
     signed_amount_cents, currency, is_refund, refund_for_expense_id, \
     category_name, category_kind, category_monthly_target_cents, \
     description, source, logged_by";

fn read_rows(
    conn: &Connection,
    timeframe: ExportTimeframe,
    now: OffsetDateTime,
) -> Result<Vec<LedgerRow>> {
    let map = |r: &rusqlite::Row<'_>| -> rusqlite::Result<LedgerRow> {
        Ok(LedgerRow {
            expense_id: r.get(0)?,
            occurred_at: r.get(1)?,
            created_at: r.get(2)?,
            amount_cents: r.get(3)?,
            signed_amount_cents: r.get(4)?,
            currency: r.get(5)?,
            is_refund: r.get::<_, i64>(6)? != 0,
            refund_for_expense_id: r.get(7)?,
            category_name: r.get(8)?,
            category_kind: r.get(9)?,
            category_monthly_target_cents: r.get(10)?,
            description: r.get(11)?,
            source: r.get(12)?,
            logged_by: r.get(13)?,
        })
    };

    match timeframe.bounds(now) {
        None => {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SELECT_COLS} FROM v_ledger_v1 \
                 ORDER BY occurred_at ASC, expense_id ASC"
            ))?;
            let rows = stmt
                .query_map([], map)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }
        Some((start, end)) => {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SELECT_COLS} FROM v_ledger_v1 \
                 WHERE occurred_at >= ?1 AND occurred_at < ?2 \
                 ORDER BY occurred_at ASC, expense_id ASC"
            ))?;
            let rows = stmt
                .query_map(params![start, end], map)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }
    }
}

/// Signed cents → a plain decimal string, e.g. `-1234` → `"-12.34"`,
/// `5` → `"0.05"`. No thousands separators (machine-friendly); the
/// integer `amount_cents` column carries exact fidelity regardless.
fn cents_to_decimal(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

fn opt(s: &Option<String>) -> &str {
    s.as_deref().unwrap_or("")
}

fn to_csv(rows: &[LedgerRow]) -> Result<Vec<u8>> {
    let mut w = csv::Writer::from_writer(Vec::new());
    // `date` (YYYY-MM-DD) and `month` (YYYY-MM) are convenience columns
    // derived from the exact `occurred_at` timestamp: a spreadsheet
    // pivot / monthly breakdown becomes one drag with zero formulas,
    // while the full timestamp is still there for fidelity.
    w.write_record([
        "occurred_at",
        "date",
        "month",
        "amount",
        "amount_cents",
        "currency",
        "category",
        "category_kind",
        "is_refund",
        "description",
        "source",
        "logged_by",
        "expense_id",
    ])
    .context("writing CSV header")?;
    for row in rows {
        w.write_record([
            row.occurred_at.as_str(),
            row.occurred_at.get(0..10).unwrap_or(""),
            row.occurred_at.get(0..7).unwrap_or(""),
            &cents_to_decimal(row.signed_amount_cents),
            &row.signed_amount_cents.to_string(),
            row.currency.as_str(),
            opt(&row.category_name),
            opt(&row.category_kind),
            if row.is_refund { "true" } else { "false" },
            opt(&row.description),
            row.source.as_str(),
            opt(&row.logged_by),
            &row.expense_id.to_string(),
        ])
        .context("writing CSV row")?;
    }
    w.flush().context("flushing CSV")?;
    w.into_inner().context("finalizing CSV buffer")
}

fn to_jsonl(rows: &[LedgerRow]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for row in rows {
        let line = serde_json::to_vec(row).context("serializing JSON Lines row")?;
        out.extend_from_slice(&line);
        out.push(b'\n');
    }
    Ok(out)
}

/// Map a category name to a single Beancount account leaf. Beancount
/// account components must start with an uppercase letter and contain
/// only `[A-Za-z0-9-]`; we keep alphanumerics, upper-case the first
/// letter of each whitespace-delimited word, and fall back to
/// `Uncategorized` for blank/uncategorized rows.
fn beancount_account(category: &Option<String>, kind: &Option<String>) -> String {
    let leaf = match category {
        None => "Uncategorized".to_string(),
        Some(name) => {
            let mut leaf = String::new();
            for word in name.split_whitespace() {
                let mut chars = word.chars().filter(|c| c.is_ascii_alphanumeric());
                if let Some(first) = chars.next() {
                    leaf.push(first.to_ascii_uppercase());
                    leaf.extend(chars);
                }
            }
            if leaf.is_empty() || !leaf.chars().next().unwrap().is_ascii_alphabetic() {
                format!("X{leaf}")
            } else {
                leaf
            }
        }
    };
    // Investing contributions are an asset transfer, not consumption;
    // everything else is an expense. Keeps double-entry honest.
    let top = if kind.as_deref() == Some("investing") {
        "Assets:Investments"
    } else {
        "Expenses"
    };
    format!("{top}:{leaf}")
}

fn beancount_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ")
}

/// Plain-text-accounting export. Every account used is `open`ed at a
/// fixed early epoch so the file is self-contained and valid even for a
/// partial-timeframe slice (the documented Beancount caveat is only
/// that a slice is less *self-explanatory* than full history, not
/// invalid). The balancing account is `Assets:Cash`.
fn to_beancount(rows: &[LedgerRow], prov: &Provenance) -> Vec<u8> {
    use std::collections::BTreeSet;
    let mut out = String::new();
    out.push_str(";; Mr. Moneypenny export — plain-text accounting (Beancount)\n");
    out.push_str(";; A faithful one-way photocopy. Money is exact; amounts are\n");
    out.push_str(";; signed (refunds negative). Dates are the UTC calendar day.\n");
    out.push_str(";;\n");
    out.push_str(&format!(
        ";; generated-by: Mr. Moneypenny v{}\n",
        prov.app_version
    ));
    out.push_str(&format!(";; schema: {}\n", prov.schema));
    out.push_str(&format!(";; timeframe: {}\n", prov.timeframe));
    out.push_str(&format!(";; generated-at: {}\n", prov.generated_at));
    out.push_str(&format!(";; rows: {}\n", prov.row_count));
    out.push_str("option \"title\" \"Mr. Moneypenny\"\n");
    out.push_str("option \"operating_currency\" \"USD\"\n\n");

    let mut accounts: BTreeSet<String> = BTreeSet::new();
    accounts.insert("Assets:Cash".to_string());
    for row in rows {
        accounts.insert(beancount_account(&row.category_name, &row.category_kind));
    }
    for acct in &accounts {
        out.push_str(&format!("1970-01-01 open {acct}\n"));
    }
    out.push('\n');

    for row in rows {
        let date = row.occurred_at.get(0..10).unwrap_or("1970-01-01");
        let narration = row
            .description
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(row.category_name.as_deref())
            .unwrap_or("(no description)");
        let acct = beancount_account(&row.category_name, &row.category_kind);
        // signed_amount_cents already encodes refund direction. The
        // category posting takes the signed amount; Cash balances it.
        let amount = cents_to_decimal(row.signed_amount_cents);
        let neg_amount = cents_to_decimal(-row.signed_amount_cents);
        // Refunds carry a #refund tag; stable identity + origin travel
        // as metadata so the export reconciles and audits cleanly in
        // Fava without the app.
        let tag = if row.is_refund { " #refund" } else { "" };
        out.push_str(&format!(
            "{date} * \"{}\"{tag}\n",
            beancount_escape(narration)
        ));
        out.push_str(&format!("  id: {}\n", row.expense_id));
        out.push_str(&format!(
            "  source: \"{}\"\n",
            beancount_escape(&row.source)
        ));
        if let Some(by) = row.logged_by.as_deref().filter(|s| !s.trim().is_empty()) {
            out.push_str(&format!("  logged_by: \"{}\"\n", beancount_escape(by)));
        }
        out.push_str(&format!("  {acct}  {amount} {}\n", row.currency));
        out.push_str(&format!("  Assets:Cash  {neg_amount} {}\n\n", row.currency));
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn seed(conn: &Connection) {
        // A non-seed name so the unique constraint on the migration's
        // seed categories isn't tripped.
        conn.execute(
            "INSERT INTO categories (name, kind, is_recurring, is_active, is_seed, monthly_target_cents) \
             VALUES ('Track Days', 'variable', 0, 1, 0, 12000)",
            [],
        )
        .unwrap();
        let dining: i64 = conn
            .query_row(
                "SELECT id FROM categories WHERE name='Track Days'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // A normal expense in March, a refund in April, an uncategorized
        // expense in April.
        conn.execute(
            "INSERT INTO expenses (amount_cents, currency, category_id, description, occurred_at, source) \
             VALUES (4250, 'USD', ?1, 'Sushi', '2026-03-10T18:00:00Z', 'telegram')",
            [dining],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO expenses (amount_cents, currency, category_id, description, occurred_at, source, is_refund) \
             VALUES (1000, 'USD', ?1, 'Returned dessert', '2026-04-02T12:00:00Z', 'manual', 1)",
            [dining],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO expenses (amount_cents, currency, category_id, description, occurred_at, source) \
             VALUES (799, 'USD', NULL, 'Mystery', '2026-04-05T09:00:00Z', 'manual')",
            [],
        )
        .unwrap();
    }

    fn fresh() -> Connection {
        let conn = crate::db::open_in_memory().unwrap();
        crate::db::migrate(&conn).unwrap();
        seed(&conn);
        conn
    }

    #[test]
    fn decimal_formatting_is_exact_and_signed() {
        assert_eq!(cents_to_decimal(0), "0.00");
        assert_eq!(cents_to_decimal(5), "0.05");
        assert_eq!(cents_to_decimal(-5), "-0.05");
        assert_eq!(cents_to_decimal(1234), "12.34");
        assert_eq!(cents_to_decimal(-100000), "-1000.00");
    }

    #[test]
    fn all_timeframe_exports_every_row() {
        let conn = fresh();
        let r = export(
            &conn,
            ExportFormat::Jsonl,
            ExportTimeframe::All,
            datetime!(2026-05-17 12:00:00 UTC),
        )
        .unwrap();
        assert_eq!(r.row_count, 3);
        let text = String::from_utf8(r.bytes).unwrap();
        assert_eq!(text.lines().count(), 3);
        // Refund is negative in signed cents.
        assert!(text.contains("\"signed_amount_cents\":-1000"));
        assert!(text.contains("\"category_name\":null"));
    }

    #[test]
    fn timeframe_filters_to_the_slice() {
        let conn = fresh();
        // "This month" anchored in April → only the two April rows.
        let r = export(
            &conn,
            ExportFormat::Csv,
            ExportTimeframe::ThisMonth,
            datetime!(2026-04-20 12:00:00 UTC),
        )
        .unwrap();
        assert_eq!(r.row_count, 2);
        let csv = String::from_utf8(r.bytes).unwrap();
        assert!(csv.contains("Returned dessert"));
        assert!(!csv.contains("Sushi"), "March row must be excluded");
        // Year anchored in 2026 → all three.
        let y = export(
            &conn,
            ExportFormat::Csv,
            ExportTimeframe::ThisYear,
            datetime!(2026-04-20 12:00:00 UTC),
        )
        .unwrap();
        assert_eq!(y.row_count, 3);
    }

    #[test]
    fn csv_has_header_and_signed_amounts() {
        let conn = fresh();
        let r = export(
            &conn,
            ExportFormat::Csv,
            ExportTimeframe::All,
            datetime!(2026-05-17 12:00:00 UTC),
        )
        .unwrap();
        let csv = String::from_utf8(r.bytes).unwrap();
        let mut lines = csv.lines();
        assert_eq!(
            lines.next().unwrap(),
            "occurred_at,date,month,amount,amount_cents,currency,category,category_kind,is_refund,description,source,logged_by,expense_id"
        );
        assert!(csv.contains("-10.00,-1000,USD,Track Days,variable,true,Returned dessert"));
        // Convenience columns derived from the exact timestamp: the
        // March Sushi row → date 2026-03-10, month 2026-03.
        assert!(csv.contains("2026-03-10T18:00:00Z,2026-03-10,2026-03,42.50,4250,USD"));
    }

    #[test]
    fn beancount_is_balanced_and_opens_accounts() {
        let conn = fresh();
        let r = export(
            &conn,
            ExportFormat::Beancount,
            ExportTimeframe::All,
            datetime!(2026-05-17 12:00:00 UTC),
        )
        .unwrap();
        let text = String::from_utf8(r.bytes).unwrap();
        assert!(text.contains("1970-01-01 open Assets:Cash"));
        assert!(text.contains("1970-01-01 open Expenses:TrackDays"));
        assert!(text.contains("1970-01-01 open Expenses:Uncategorized"));
        // The Sushi transaction: 42.50 to TrackDays, -42.50 to Cash.
        assert!(text.contains("2026-03-10 * \"Sushi\""));
        assert!(text.contains("Expenses:TrackDays  42.50 USD"));
        assert!(text.contains("Assets:Cash  -42.50 USD"));
        // The refund inverts and is tagged.
        assert!(text.contains("Expenses:TrackDays  -10.00 USD"));
        assert!(text.contains("\"Returned dessert\" #refund"));
        assert!(!text.contains("\"Sushi\" #refund"), "non-refund untagged");
        // Provenance header is self-describing.
        assert!(text.contains(&format!(
            ";; generated-by: Mr. Moneypenny v{}",
            env!("CARGO_PKG_VERSION")
        )));
        assert!(text.contains(";; schema: v_ledger_v1"));
        assert!(text.contains(";; timeframe: all time"));
        assert!(text.contains(";; rows: 3"));
        // Stable identity + origin travel as metadata.
        assert!(text.contains("  source: \"telegram\""));
        let sushi_id: i64 = conn
            .query_row(
                "SELECT expense_id FROM v_ledger_v1 WHERE description='Sushi'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(text.contains(&format!("  id: {sushi_id}")));
    }

    #[test]
    fn view_is_the_only_thing_read() {
        // Belt-and-suspenders: the export query targets v_ledger_v1, so
        // confirm the view exists post-migration and is selectable.
        let conn = fresh();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM v_ledger_v1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 3);
    }
}
