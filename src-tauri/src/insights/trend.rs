//! Per-category trend detection (opt-in tool, not auto-rendered).
//!
//! User picks a category from the Forecast view's Trend Analyzer
//! section; we run a least-squares linear regression over the last
//! N months of that category's totals and surface:
//!
//! - Slope, expressed as `$/mo per year` (multiply per-month-per-month
//!   slope by 12) so users can read it as a rate.
//! - R², so they know how much to trust the line.
//! - A plain-English headline.
//!
//! Why opt-in only: rendering this for every category on the
//! Categories tab would add 30+ chart panels and look like a stock-
//! trading app. The user explicitly asked for the dropdown form.

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;
use time::OffsetDateTime;

use crate::repository::expenses;

#[derive(Debug, Clone, Serialize)]
pub struct TrendResult {
    /// One element per month, oldest first. None when the user had no
    /// activity that month — left as nulls so the UI can render gaps.
    pub monthly_totals_cents: Vec<i64>,
    pub n_months_with_data: usize,
    /// Slope in cents per month per month. Multiply by 12 for the
    /// "$/mo per year" headline.
    pub slope_cents_per_month: f64,
    pub intercept_cents: f64,
    pub r_squared: f64,
    /// Direction word — "rising", "falling", "flat".
    pub direction: &'static str,
    /// Pre-formatted plain-English headline for the UI.
    pub headline: String,
}

/// Compute a category's trend over the last `months_back` months.
/// Empty / single-data-point histories return a trend with all-zero
/// fit — the caller should suppress the chart in that case.
pub fn compute(
    conn: &Connection,
    category_id: i64,
    now: OffsetDateTime,
    months_back: u32,
) -> Result<TrendResult> {
    let totals = expenses::monthly_totals_for_category(conn, category_id, now, months_back)?;
    let n_with_data = totals.iter().filter(|v| **v != 0).count();
    if totals.len() < 2 || n_with_data < 2 {
        return Ok(TrendResult {
            monthly_totals_cents: totals.clone(),
            n_months_with_data: n_with_data,
            slope_cents_per_month: 0.0,
            intercept_cents: 0.0,
            r_squared: 0.0,
            direction: "flat",
            headline: "Not enough history yet — log this category for a few more months.".into(),
        });
    }

    let (slope, intercept, r_sq) = least_squares(&totals);
    let slope_per_year = slope * 12.0;
    let direction = classify_direction(slope_per_year);
    let headline = build_headline(slope_per_year, r_sq, direction);

    Ok(TrendResult {
        monthly_totals_cents: totals,
        n_months_with_data: n_with_data,
        slope_cents_per_month: slope,
        intercept_cents: intercept,
        r_squared: r_sq,
        direction,
        headline,
    })
}

/// Closed-form least-squares: fit `y = slope*x + intercept` where `x`
/// is the month index 0..n-1 and `y` is the per-month total in cents.
/// Also returns R² (coefficient of determination).
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
    // R² = 1 - SS_res / SS_tot
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
    // Threshold: $10/year is below the "interesting" floor.
    if slope_per_year_cents.abs() < 1000.0 {
        "flat"
    } else if slope_per_year_cents > 0.0 {
        "rising"
    } else {
        "falling"
    }
}

fn build_headline(slope_per_year_cents: f64, r_sq: f64, direction: &str) -> String {
    match direction {
        "flat" => "Spending in this category is roughly flat over the period.".into(),
        _ => {
            let dollars_per_year = (slope_per_year_cents.abs() / 100.0).round() as i64;
            let strength = if r_sq >= 0.7 {
                "strong"
            } else if r_sq >= 0.4 {
                "moderate"
            } else {
                "weak"
            };
            format!(
                "Spending is {direction} at ${dollars_per_year}/mo per year — {strength} trend (R²={:.2}).",
                r_sq
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn least_squares_perfect_line() {
        // y = 100x + 0 → slope=100, intercept=0, R²=1
        let y: Vec<i64> = (0..12).map(|i| (i * 100) as i64).collect();
        let (slope, intercept, r_sq) = least_squares(&y);
        assert!((slope - 100.0).abs() < 1e-6);
        assert!(intercept.abs() < 1e-6);
        assert!((r_sq - 1.0).abs() < 1e-6);
    }

    #[test]
    fn least_squares_flat_line_zero_slope() {
        let y = vec![5000_i64; 12];
        let (slope, intercept, r_sq) = least_squares(&y);
        assert!(slope.abs() < 1e-6);
        assert!((intercept - 5000.0).abs() < 1e-6);
        // Flat data has zero variance → R² = 0 by convention.
        assert!((r_sq - 0.0).abs() < 1e-6);
    }

    #[test]
    fn least_squares_noisy_line_recovers_slope_and_partial_fit() {
        // y = 100x + noise. Slope still ~100, R² high but <1.
        let y: Vec<i64> = (0..12)
            .map(|i| (i * 100 + (i * 17 % 13)) as i64) // deterministic "noise"
            .collect();
        let (slope, _intercept, r_sq) = least_squares(&y);
        assert!((slope - 100.0).abs() < 5.0);
        assert!(r_sq > 0.95);
    }

    #[test]
    fn classify_direction_thresholds() {
        assert_eq!(classify_direction(0.0), "flat");
        assert_eq!(classify_direction(500.0), "flat"); // below $10/yr threshold
        assert_eq!(classify_direction(1500.0), "rising");
        assert_eq!(classify_direction(-1500.0), "falling");
    }

    #[test]
    fn headline_includes_strength_word() {
        let strong = build_headline(36000.0, 0.85, "rising");
        assert!(strong.contains("strong"));
        let moderate = build_headline(36000.0, 0.55, "rising");
        assert!(moderate.contains("moderate"));
        let weak = build_headline(36000.0, 0.2, "rising");
        assert!(weak.contains("weak"));
    }

    #[test]
    fn flat_headline_short_circuits() {
        let h = build_headline(0.0, 0.0, "flat");
        assert!(h.contains("flat"));
        assert!(!h.contains("R²"));
    }
}
