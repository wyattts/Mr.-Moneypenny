//! CSV parsing — wrap the `csv` crate with bank-export friendly
//! defaults, then project rows through a `ColumnMapping`.
//!
//! ## Date format support
//!
//! The user picks one of a small fixed set on the mapping screen. We
//! avoid `time`'s format strings here (their token language is its own
//! footgun) and just do explicit `MM/DD/YYYY`-style parsing for the
//! shapes real bank exports use.
//!
//! ## Amount parsing
//!
//! Bank exports use `1234.56`, `-1,234.56`, `(1,234.56)` (parens for
//! negatives), and occasionally `$1,234.56`. We strip leading `$`, drop
//! commas, and treat parens as a sign flip.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time};

use crate::repository::csv_import_profiles::ColumnMapping;

/// One CSV row projected through a column mapping. Unmapped data is
/// dropped; the importer doesn't preserve the raw row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedRow {
    /// Stable index into the source file (0-based, after `skip_rows`).
    pub source_row_index: usize,
    pub occurred_at: OffsetDateTime,
    /// Always positive; `is_refund` carries the sign.
    pub amount_cents: i64,
    pub merchant: String,
    pub description: Option<String>,
    /// Category string from the bank export (rarely matches our
    /// categories but useful as a hint for the review screen).
    pub raw_category: Option<String>,
    /// True when the source amount was negative AND the profile says
    /// "negative means refund." The importer auto-marks these.
    pub is_refund: bool,
}

/// Preview payload for the mapping screen: header row + first N data
/// rows verbatim, plus the computed header signature.
#[derive(Debug, Clone, Serialize)]
pub struct PreviewResult {
    pub headers: Vec<String>,
    pub sample_rows: Vec<Vec<String>>,
    pub header_signature: String,
    pub total_rows: usize,
}

/// Build a CSV reader over a string of file content. The frontend
/// reads the file via the WebView (privacy-respecting; no native
/// dialog plugin needed) and sends the content over IPC.
fn reader_from_str(content: &str) -> csv::Reader<&[u8]> {
    csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(content.as_bytes())
}

/// Read a CSV's header row + first 10 data rows for the mapping
/// screen. Doesn't validate anything — that happens once a mapping
/// is applied.
pub fn parse_preview(content: &str) -> Result<PreviewResult> {
    let mut rdr = reader_from_str(content);
    let headers: Vec<String> = rdr
        .headers()?
        .iter()
        .map(|s| s.trim().to_string())
        .collect();
    let mut sample_rows = Vec::new();
    let mut total_rows = 0usize;
    for (i, rec) in rdr.records().enumerate() {
        let rec = rec.with_context(|| format!("reading row {i}"))?;
        total_rows += 1;
        if i < 10 {
            sample_rows.push(rec.iter().map(|s| s.to_string()).collect());
        }
    }
    let signature = crate::repository::csv_import_profiles::header_signature(&headers);
    Ok(PreviewResult {
        headers,
        sample_rows,
        header_signature: signature,
        total_rows,
    })
}

/// Parse the full content with the given mapping. Skips `skip_rows`
/// after the header, then projects each row. A row missing required
/// cells is a hard error so the user fixes the mapping rather than
/// silently losing data.
pub fn parse_with_mapping(content: &str, mapping: &ColumnMapping) -> Result<Vec<ParsedRow>> {
    let mut rdr = reader_from_str(content);
    let mut out = Vec::new();
    let skip = mapping.skip_rows;
    for (i, rec) in rdr.records().enumerate() {
        if i < skip {
            continue;
        }
        let rec = rec.with_context(|| format!("reading row {i}"))?;
        let cells: Vec<&str> = rec.iter().collect();
        let row = project_row(i, &cells, mapping)
            .with_context(|| format!("row {} mapping failed", i + 2))?;
        out.push(row);
    }
    Ok(out)
}

fn project_row(
    source_row_index: usize,
    cells: &[&str],
    mapping: &ColumnMapping,
) -> Result<ParsedRow> {
    let date_cell = cells
        .get(mapping.date_col)
        .ok_or_else(|| anyhow!("date column {} out of range", mapping.date_col))?;
    let amount_cell = cells
        .get(mapping.amount_col)
        .ok_or_else(|| anyhow!("amount column {} out of range", mapping.amount_col))?;
    let merchant_cell = cells
        .get(mapping.merchant_col)
        .ok_or_else(|| anyhow!("merchant column {} out of range", mapping.merchant_col))?;
    let description = mapping
        .description_col
        .and_then(|c| cells.get(c).map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty());
    let raw_category = mapping
        .category_col
        .and_then(|c| cells.get(c).map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty());

    let occurred_at = parse_date(date_cell.trim(), &mapping.date_format)?;
    let (amount_cents, neg) = parse_amount(amount_cell.trim())?;
    let is_refund = neg && mapping.neg_means_refund;
    Ok(ParsedRow {
        source_row_index,
        occurred_at,
        amount_cents,
        merchant: merchant_cell.trim().to_string(),
        description,
        raw_category,
        is_refund,
    })
}

/// Parse a date string against a known format token. Returns the date
/// at midnight UTC.
pub fn parse_date(input: &str, format: &str) -> Result<OffsetDateTime> {
    let (y, m, d) = match format {
        "MM/DD/YYYY" => parse_split(input, '/', |a, b, c| (c, a, b))?,
        "DD/MM/YYYY" => parse_split(input, '/', |a, b, c| (c, b, a))?,
        "YYYY-MM-DD" => parse_split(input, '-', |a, b, c| (a, b, c))?,
        "MM-DD-YYYY" => parse_split(input, '-', |a, b, c| (c, a, b))?,
        "DD-MM-YYYY" => parse_split(input, '-', |a, b, c| (c, b, a))?,
        "M/D/YYYY" => parse_split(input, '/', |a, b, c| (c, a, b))?,
        other => anyhow::bail!("unsupported date format: {other}"),
    };
    let month = Month::try_from(m as u8).map_err(|_| anyhow!("invalid month {m} in {input}"))?;
    let date = Date::from_calendar_date(y as i32, month, d as u8)
        .map_err(|e| anyhow!("invalid date {input}: {e}"))?;
    let dt = PrimitiveDateTime::new(date, Time::MIDNIGHT).assume_utc();
    Ok(dt)
}

/// Helper: split on `sep` into 3 numeric parts, then route them via
/// `route` to (year, month, day). The router lets one helper serve all
/// three orderings.
fn parse_split(
    input: &str,
    sep: char,
    route: impl Fn(u32, u32, u32) -> (u32, u32, u32),
) -> Result<(u32, u32, u32)> {
    let parts: Vec<u32> = input
        .split(sep)
        .map(|s| s.trim().parse::<u32>())
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| anyhow!("parsing date components from {input}: {e}"))?;
    if parts.len() != 3 {
        anyhow::bail!("expected 3 date parts, got {} in {input}", parts.len());
    }
    Ok(route(parts[0], parts[1], parts[2]))
}

/// Parse a bank-style amount string. Returns `(positive_cents,
/// was_negative)`.
///
/// Handles:
/// - US format: `1,234.56` / `$1,234.56` / `-1234.56` / `(1,234.56)`
/// - European format: `1.234,56` / `1234,56` (comma is decimal,
///   period is thousands)
///
/// Locale heuristic: if a comma appears AFTER the last period, the
/// comma is the decimal separator (EU). Otherwise the period is decimal
/// (US). This catches the silent corruption where a German bank's CSV
/// (`1.234,56` = 123,456 cents) was previously parsed as 12,346 cents
/// — off by 100×.
///
/// Rejects: empty, NaN, infinity, scientific notation that doesn't
/// look like a real amount.
pub fn parse_amount(input: &str) -> Result<(i64, bool)> {
    let s = input.trim();
    if s.is_empty() {
        anyhow::bail!("empty amount");
    }
    let (s, neg_paren) = if s.starts_with('(') && s.ends_with(')') {
        (&s[1..s.len() - 1], true)
    } else {
        (s, false)
    };
    let s = s.trim().trim_start_matches('$').trim();

    // Locale-detect:
    let normalized = match (s.rfind(','), s.rfind('.')) {
        (Some(comma_idx), Some(period_idx)) if comma_idx > period_idx => {
            // EU format: '.' = thousands, ',' = decimal.
            // e.g., "1.234,56" → "1234.56"
            s.replace('.', "").replace(',', ".")
        }
        (Some(_), None) if s.matches(',').count() == 1 && {
            // Single comma + nothing after the dot — could be either
            // "1234,56" (EU decimal) or "1,234" (US thousands without
            // decimal). Use the trailing-fragment length: 1-2 digits
            // after the comma → decimal; 3 digits → thousands.
            let frag_len = s.split(',').next_back().unwrap_or("").trim_end_matches(|c: char| !c.is_ascii_digit()).len();
            (1..=2).contains(&frag_len)
        } =>
        {
            // EU shape: "1234,56"
            s.replace(',', ".")
        }
        _ => {
            // US shape: comma is thousands separator, drop it.
            s.replace(',', "")
        }
    };

    let f: f64 = normalized
        .parse()
        .map_err(|_| anyhow!("not a number: {input}"))?;
    if !f.is_finite() {
        anyhow::bail!("amount must be finite: {input}");
    }
    let neg = neg_paren || f < 0.0;
    let cents = (f.abs() * 100.0).round() as i64;
    Ok((cents, neg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping() -> ColumnMapping {
        ColumnMapping {
            date_col: 0,
            amount_col: 1,
            merchant_col: 2,
            description_col: None,
            category_col: None,
            date_format: "MM/DD/YYYY".into(),
            neg_means_refund: true,
            skip_rows: 0,
        }
    }

    #[test]
    fn parse_amount_positive_simple() {
        let (c, n) = parse_amount("12.34").unwrap();
        assert_eq!(c, 1234);
        assert!(!n);
    }

    #[test]
    fn parse_amount_negative_with_minus() {
        let (c, n) = parse_amount("-12.34").unwrap();
        assert_eq!(c, 1234);
        assert!(n);
    }

    #[test]
    fn parse_amount_negative_with_parens() {
        let (c, n) = parse_amount("(1,234.56)").unwrap();
        assert_eq!(c, 123456);
        assert!(n);
    }

    #[test]
    fn parse_amount_strips_dollar_and_comma() {
        let (c, n) = parse_amount("$1,234.56").unwrap();
        assert_eq!(c, 123456);
        assert!(!n);
    }

    #[test]
    fn parse_amount_rejects_garbage() {
        assert!(parse_amount("oops").is_err());
    }

    #[test]
    fn parse_amount_eu_format_with_thousands_dot() {
        // "1.234,56" — German/French — comma is decimal, dot is thousands.
        // Pre-fix this returned 12346 cents (10x too small). Must be 123456.
        let (c, n) = parse_amount("1.234,56").unwrap();
        assert_eq!(c, 123456);
        assert!(!n);
    }

    #[test]
    fn parse_amount_eu_format_no_thousands() {
        // "1234,56" — comma is decimal, no thousands separator.
        let (c, n) = parse_amount("1234,56").unwrap();
        assert_eq!(c, 123456);
        assert!(!n);
    }

    #[test]
    fn parse_amount_eu_format_with_parens() {
        let (c, n) = parse_amount("(1.234,56)").unwrap();
        assert_eq!(c, 123456);
        assert!(n);
    }

    #[test]
    fn parse_amount_us_thousands_without_decimal() {
        // "1,234" — US thousands, no decimal.
        let (c, n) = parse_amount("1,234").unwrap();
        assert_eq!(c, 123400);
        assert!(!n);
    }

    #[test]
    fn parse_amount_rejects_infinity() {
        assert!(parse_amount("inf").is_err());
        assert!(parse_amount("infinity").is_err());
        assert!(parse_amount("-inf").is_err());
    }

    #[test]
    fn parse_amount_rejects_nan() {
        assert!(parse_amount("nan").is_err());
        assert!(parse_amount("NaN").is_err());
    }

    #[test]
    fn parse_amount_rejects_empty() {
        assert!(parse_amount("").is_err());
        assert!(parse_amount("   ").is_err());
    }

    #[test]
    fn parse_date_us_format() {
        let d = parse_date("01/15/2026", "MM/DD/YYYY").unwrap();
        assert_eq!(d.year(), 2026);
        assert_eq!(d.month() as u8, 1);
        assert_eq!(d.day(), 15);
    }

    #[test]
    fn parse_date_eu_format() {
        let d = parse_date("15/01/2026", "DD/MM/YYYY").unwrap();
        assert_eq!(d.day(), 15);
        assert_eq!(d.month() as u8, 1);
    }

    #[test]
    fn parse_date_iso() {
        let d = parse_date("2026-01-15", "YYYY-MM-DD").unwrap();
        assert_eq!(d.year(), 2026);
        assert_eq!(d.month() as u8, 1);
        assert_eq!(d.day(), 15);
    }

    #[test]
    fn parse_date_rejects_invalid() {
        assert!(parse_date("13/45/2026", "MM/DD/YYYY").is_err());
        assert!(parse_date("not-a-date", "MM/DD/YYYY").is_err());
    }

    #[test]
    fn project_row_handles_negative_as_refund() {
        let cells = vec!["01/15/2026", "-15.49", "Netflix"];
        let r = project_row(0, &cells, &mapping()).unwrap();
        assert_eq!(r.amount_cents, 1549);
        assert!(r.is_refund);
        assert_eq!(r.merchant, "Netflix");
    }

    #[test]
    fn project_row_neg_with_neg_means_refund_false_stays_positive_no_refund() {
        let mut m = mapping();
        m.neg_means_refund = false;
        let cells = vec!["01/15/2026", "-15.49", "Netflix"];
        let r = project_row(0, &cells, &m).unwrap();
        assert_eq!(r.amount_cents, 1549);
        assert!(!r.is_refund);
    }

    #[test]
    fn parse_with_mapping_round_trips_a_chase_export() {
        let csv_data = "Posting Date,Amount,Description\n\
01/15/2026,-12.50,STARBUCKS #4521\n\
01/16/2026,-89.99,AMAZON.COM\n\
01/20/2026,1500.00,DIRECT DEPOSIT\n";
        let rows = parse_with_mapping(csv_data, &mapping()).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].amount_cents, 1250);
        assert!(rows[0].is_refund);
        assert_eq!(rows[2].amount_cents, 150_000);
        assert!(!rows[2].is_refund);
    }

    #[test]
    fn parse_preview_reports_headers_and_sample() {
        let csv_data = "Posting Date,Amount,Description\n\
01/15/2026,-12.50,STARBUCKS\n\
01/16/2026,-89.99,AMAZON\n";
        let p = parse_preview(csv_data).unwrap();
        assert_eq!(p.headers, vec!["Posting Date", "Amount", "Description"]);
        assert_eq!(p.sample_rows.len(), 2);
        assert_eq!(p.total_rows, 2);
        assert!(!p.header_signature.is_empty());
    }
}
