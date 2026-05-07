//! Monte Carlo investment-path simulation.
//!
//! Runs N paths (default 1,000) where each month's return is drawn
//! from a Normal distribution: μ = annual_rate/12, σ = annual_vol/√12.
//! At each requested time-step we sort the path values across paths
//! and extract three percentiles: the median (P50) and the two edges
//! of a configurable confidence band (`band_pct`). Other code derives
//! "the central X% of outcomes" purely from these three numbers.
//!
//! Caller-supplied `band_pct` controls the band width:
//!
//! ```text
//! p_lo  =  ((1 - band_pct) / 2) × 100
//! p_hi  =  100 - p_lo
//! ```
//!
//! e.g., band_pct = 0.80 → P10..P90; 0.95 → P2.5..P97.5; 0.62 → P19..P81.
//!
//! ## Why parametric Normal (and not bootstrap)?
//!
//! Bootstrap from historical returns is more accurate when applicable
//! but requires ≥12 months of investing-category contribution + balance
//! data, which most users won't have for a long time after install.

use rand::distributions::{Distribution, Uniform};
use rand::SeedableRng;
use serde::{Deserialize, Serialize};

/// One scheduled cash flow into (or out of) the investment account at a
/// specific month of the simulation. `amount_cents` is signed: positive
/// = deposit (tax refund, bonus, gift), negative = withdrawal (planned
/// large expense). `month_offset` is 1-indexed and matches `m` in the
/// per-path loop, so a lump with `month_offset = 12` is applied at the
/// end of month 12 — *after* that month's GBM growth and monthly
/// contribution.
///
/// Lumps with `month_offset` outside `[1, horizon_months]` are silently
/// ignored. Multiple lumps at the same month sum together.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LumpSum {
    pub month_offset: u32,
    pub amount_cents: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PathInput {
    pub starting_balance_cents: i64,
    pub monthly_contribution_cents: i64,
    pub annual_return_pct: f64,
    /// Standard deviation of annual returns, as a percent. Convert to
    /// monthly via /√12 inside `simulate`.
    pub annual_volatility_pct: f64,
    pub horizon_years: u32,
    /// Default 1000 if zero or unset.
    pub n_paths: u32,
    /// How many time-steps to report bands at. Defaults to 30. Always
    /// includes t=0 and t=horizon.
    pub time_points: u32,
    /// Confidence band width (0..1). 0.80 = central 80% of outcomes.
    /// Defaults to 0.80 if 0 or unset.
    pub band_pct: f64,
    /// Optional fixed RNG seed for reproducibility (tests + replays).
    pub seed: Option<u64>,
    /// Scheduled one-time cash flows (positive = deposit, negative =
    /// withdrawal). See [`LumpSum`] for semantics.
    #[serde(default)]
    pub lump_sums: Vec<LumpSum>,
    /// Annual withdrawal rate as a percentage of *current* balance,
    /// spread evenly across 12 months. e.g. `4.0` means each month the
    /// path subtracts `balance × 4 / 100 / 12` after growth, monthly
    /// contribution, and any lump sum at this month. Negative values
    /// are clamped to zero — withdrawals only.
    ///
    /// Constant-percent semantics mean the withdrawal fluctuates with
    /// the market: smaller in down years, larger in up years. The
    /// portfolio asymptotes rather than ever fully depleting at a
    /// modest rate.
    #[serde(default)]
    pub withdrawal_rate_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthBand {
    pub month: u32,
    /// Lower edge of the band — `(1 - band_pct)/2` percentile.
    pub p_lo: i64,
    pub p50: i64,
    /// Upper edge of the band — complement of `p_lo`.
    pub p_hi: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathBands {
    pub points: Vec<MonthBand>,
    pub final_p_lo_cents: i64,
    pub final_p50_cents: i64,
    pub final_p_hi_cents: i64,
    pub n_paths: u32,
    /// Echoed back so the UI can label "X% probability band" without
    /// guessing what band_pct the call resolved to.
    pub band_pct: f64,
}

/// Run the simulation and produce per-month percentile bands at the
/// requested time-step grid.
pub fn simulate(input: &PathInput) -> PathBands {
    let n_paths = if input.n_paths == 0 {
        1000
    } else {
        input.n_paths
    } as usize;
    let n_months = (input.horizon_years as i64) * 12;
    let time_points = if input.time_points < 2 {
        30
    } else {
        input.time_points
    };
    let band_pct = if input.band_pct <= 0.0 || input.band_pct >= 1.0 {
        0.80
    } else {
        input.band_pct
    };
    let mu_monthly = input.annual_return_pct / 100.0 / 12.0;
    let sigma_monthly = (input.annual_volatility_pct / 100.0) / 12_f64.sqrt();

    let snapshot_months = even_grid(n_months, time_points);
    let lump_at = bucket_lumps(&input.lump_sums, n_months);
    let withdraw_monthly_factor = input.withdrawal_rate_pct.max(0.0) / 100.0 / 12.0;

    let mut grid: Vec<Vec<i64>> = (0..snapshot_months.len())
        .map(|_| Vec::with_capacity(n_paths))
        .collect();

    let mut rng = match input.seed {
        Some(s) => rand::rngs::StdRng::seed_from_u64(s),
        None => rand::rngs::StdRng::from_entropy(),
    };

    for _ in 0..n_paths {
        let mut value = input.starting_balance_cents as f64;
        let mut next_snap_idx = 0usize;
        if snapshot_months[0] == 0 {
            grid[0].push(value as i64);
            next_snap_idx = 1;
        }
        for m in 1..=n_months {
            let r = sample_normal(&mut rng, mu_monthly, sigma_monthly);
            value = value * (1.0 + r) + input.monthly_contribution_cents as f64;
            value += lump_at[m as usize] as f64;
            if value > 0.0 && withdraw_monthly_factor > 0.0 {
                value -= value * withdraw_monthly_factor;
            }
            // A negative-amount lump or a stray rounding tick can drive
            // the path below zero; clamp so the broke-trajectory shows
            // as flat-zero rather than negative wealth.
            if value < 0.0 {
                value = 0.0;
            }
            if next_snap_idx < snapshot_months.len() && snapshot_months[next_snap_idx] == m {
                grid[next_snap_idx].push(value as i64);
                next_snap_idx += 1;
            }
        }
    }

    // Compute the band-edge percentile points from band_pct.
    let lo_pct = ((1.0 - band_pct) / 2.0) * 100.0;
    let hi_pct = 100.0 - lo_pct;

    let mut points = Vec::with_capacity(snapshot_months.len());
    for (i, m) in snapshot_months.iter().enumerate() {
        let col = &mut grid[i];
        col.sort_unstable();
        points.push(MonthBand {
            month: *m as u32,
            p_lo: percentile_f(col, lo_pct),
            p50: percentile_f(col, 50.0),
            p_hi: percentile_f(col, hi_pct),
        });
    }

    let last = points.last().expect("at least t=horizon present");
    PathBands {
        final_p_lo_cents: last.p_lo,
        final_p50_cents: last.p50,
        final_p_hi_cents: last.p_hi,
        points,
        n_paths: n_paths as u32,
        band_pct,
    }
}

/// Goal-probability variant: counts what fraction of paths end above
/// `target_cents`. Reuses the same engine but skips percentiles.
pub fn goal_probability(input: &PathInput, target_cents: i64) -> f64 {
    let n_paths = if input.n_paths == 0 {
        1000
    } else {
        input.n_paths
    } as usize;
    let n_months = (input.horizon_years as i64) * 12;
    let mu_monthly = input.annual_return_pct / 100.0 / 12.0;
    let sigma_monthly = (input.annual_volatility_pct / 100.0) / 12_f64.sqrt();
    let lump_at = bucket_lumps(&input.lump_sums, n_months);
    let withdraw_monthly_factor = input.withdrawal_rate_pct.max(0.0) / 100.0 / 12.0;
    let mut hits = 0usize;
    let mut rng = match input.seed {
        Some(s) => rand::rngs::StdRng::seed_from_u64(s),
        None => rand::rngs::StdRng::from_entropy(),
    };
    for _ in 0..n_paths {
        let mut value = input.starting_balance_cents as f64;
        for m in 1..=n_months {
            let r = sample_normal(&mut rng, mu_monthly, sigma_monthly);
            value = value * (1.0 + r) + input.monthly_contribution_cents as f64;
            value += lump_at[m as usize] as f64;
            if value > 0.0 && withdraw_monthly_factor > 0.0 {
                value -= value * withdraw_monthly_factor;
            }
            if value < 0.0 {
                value = 0.0;
            }
        }
        if value as i64 >= target_cents {
            hits += 1;
        }
    }
    hits as f64 / n_paths as f64
}

/// Deterministic safe withdrawal rate (annual, % of current balance)
/// using the standard PMT annuity formula on the *real* (inflation-
/// adjusted) return. The result is the constant annual percentage of
/// current balance that, if withdrawn from `remaining_months` from
/// today onwards while the portfolio earns exactly the expected real
/// return, drains the balance to zero precisely at the horizon.
///
/// Notes:
/// - Independent of the absolute balance — same answer at any month
///   given the same `remaining_months`. The frontend multiplies by the
///   hovered point's balance to get a dollar amount.
/// - Returns 0 for `remaining_months <= 0` (already at horizon — no
///   future withdrawals to plan).
/// - Real return is `annual_return - annual_inflation`, computed as
///   monthly via division by 12. This matches the rest of the
///   simulator's arithmetic-monthly convention; the small geometric
///   drift is consistent with how the chart's Real trace is built.
/// - Negative real return: the formula is still well-defined and
///   returns the rate that exactly drains the portfolio (lower than
///   the real return because compounding works against you).
pub fn swr_deterministic_pct(
    annual_return_pct: f64,
    annual_inflation_pct: f64,
    remaining_months: i64,
) -> f64 {
    if remaining_months <= 0 {
        return 0.0;
    }
    let real_monthly = (annual_return_pct - annual_inflation_pct) / 100.0 / 12.0;
    // PMT = balance × r / (1 − (1+r)^(−n)). Convert monthly PMT to
    // annual percentage of balance: ×12 ×100 / balance.
    let n = remaining_months as f64;
    if real_monthly.abs() < 1e-12 {
        // Zero real return → linear drawdown: pmt_monthly = balance/n.
        // Annual % = 12/n × 100.
        return 12.0 / n * 100.0;
    }
    let denom = 1.0 - (1.0 + real_monthly).powf(-n);
    real_monthly * 100.0 * 12.0 / denom
}

/// Coalesce lump sums into a per-month total, indexed `[0..=n_months]`.
/// `month_offset` values outside `[1, n_months]` are silently dropped so
/// the simulator doesn't have to validate inputs at every call site.
/// Returns a Vec of length `n_months + 1` (so index 0..=n_months is
/// valid); index 0 is always zero.
pub(super) fn bucket_lumps(lumps: &[LumpSum], n_months: i64) -> Vec<i64> {
    let mut out = vec![0i64; (n_months as usize) + 1];
    for l in lumps {
        let m = l.month_offset as i64;
        if (1..=n_months).contains(&m) {
            out[m as usize] = out[m as usize].saturating_add(l.amount_cents);
        }
    }
    out
}

/// Box-Muller transform: turn two uniforms into one Normal sample.
fn sample_normal<R: rand::Rng>(rng: &mut R, mean: f64, sd: f64) -> f64 {
    let u: Uniform<f64> = Uniform::new_inclusive(f64::EPSILON, 1.0);
    let u1 = u.sample(rng);
    let u2 = u.sample(rng);
    let z0 = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
    mean + sd * z0
}

/// Generate `n` evenly spaced months from 0 to `total`, inclusive of
/// both endpoints. Always returns at least [0, total]; collapses
/// duplicates when `total < n`.
fn even_grid(total: i64, n: u32) -> Vec<i64> {
    let n = n.max(2) as usize;
    let mut out = Vec::with_capacity(n);
    let mut last = -1i64;
    for i in 0..n {
        let m = ((i as f64 / (n - 1) as f64) * total as f64).round() as i64;
        if m != last {
            out.push(m);
            last = m;
        }
    }
    if *out.last().unwrap_or(&-1) != total {
        out.push(total);
    }
    out
}

/// Linear-interpolated percentile on a sorted slice. p in [0, 100].
fn percentile_f(sorted: &[i64], p: f64) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let p = p.clamp(0.0, 100.0) / 100.0;
    let idx = p * (sorted.len() - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let frac = idx - lo as f64;
        let a = sorted[lo] as f64;
        let b = sorted[hi] as f64;
        (a + frac * (b - a)).round() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close_pct(actual: i64, expected: i64, tol_pct: f64) -> bool {
        let diff = (actual - expected).abs() as f64;
        diff / (expected.abs() as f64).max(1.0) <= tol_pct
    }

    fn defaults() -> PathInput {
        PathInput {
            starting_balance_cents: 0,
            monthly_contribution_cents: 50_000,
            annual_return_pct: 7.0,
            annual_volatility_pct: 0.0,
            horizon_years: 30,
            n_paths: 200,
            time_points: 30,
            band_pct: 0.80,
            seed: Some(42),
            lump_sums: vec![],
            withdrawal_rate_pct: 0.0,
        }
    }

    #[test]
    fn zero_volatility_p50_matches_closed_form() {
        let r = simulate(&defaults());
        let p50 = r.final_p50_cents;
        assert!(
            close_pct(p50, 60_998_571, 0.001),
            "P50 {} far from $609,985.71",
            p50
        );
        assert_eq!(r.final_p_lo_cents, p50);
        assert_eq!(r.final_p_hi_cents, p50);
    }

    #[test]
    fn band_widens_with_higher_band_pct() {
        let mut input = defaults();
        input.annual_volatility_pct = 15.0;
        input.n_paths = 1000;

        input.band_pct = 0.50;
        let r50 = simulate(&input);
        let span_50 = r50.final_p_hi_cents - r50.final_p_lo_cents;

        input.band_pct = 0.90;
        let r90 = simulate(&input);
        let span_90 = r90.final_p_hi_cents - r90.final_p_lo_cents;

        assert!(
            span_90 > span_50,
            "90% band ({span_90}) should be wider than 50% band ({span_50})"
        );
    }

    #[test]
    fn band_pct_default_when_zero() {
        let mut input = defaults();
        input.band_pct = 0.0;
        let r = simulate(&input);
        assert!(
            (r.band_pct - 0.80).abs() < 1e-9,
            "expected default 0.80, got {}",
            r.band_pct
        );
    }

    #[test]
    fn band_pct_echoed_back() {
        let mut input = defaults();
        input.annual_volatility_pct = 10.0;
        input.band_pct = 0.62;
        let r = simulate(&input);
        assert!((r.band_pct - 0.62).abs() < 1e-9);
    }

    #[test]
    fn seed_makes_results_reproducible() {
        let make = || {
            let mut input = defaults();
            input.starting_balance_cents = 1_000_000;
            input.monthly_contribution_cents = 100_000;
            input.annual_volatility_pct = 10.0;
            input.horizon_years = 10;
            input.n_paths = 200;
            input.time_points = 12;
            input.seed = Some(123);
            simulate(&input)
        };
        let a = make();
        let b = make();
        assert_eq!(a.final_p50_cents, b.final_p50_cents);
        assert_eq!(a.final_p_lo_cents, b.final_p_lo_cents);
        assert_eq!(a.final_p_hi_cents, b.final_p_hi_cents);
    }

    #[test]
    fn goal_probability_zero_vol_decisive() {
        let p = goal_probability(&defaults(), 100_000_000);
        assert!(p < 0.01, "expected ~0 probability, got {p}");
    }

    #[test]
    fn goal_probability_with_vol_is_in_range() {
        let mut input = defaults();
        input.annual_volatility_pct = 15.0;
        input.n_paths = 1000;
        let p = goal_probability(&input, 60_998_571);
        assert!(
            (0.30..=0.60).contains(&p),
            "probability {p} outside expected 0.30..0.60 band"
        );
    }

    #[test]
    fn percentile_linear_interpolation() {
        let s = vec![10, 20, 30, 40, 50];
        assert_eq!(percentile_f(&s, 0.0), 10);
        assert_eq!(percentile_f(&s, 100.0), 50);
        assert_eq!(percentile_f(&s, 50.0), 30);
        assert_eq!(percentile_f(&s, 25.0), 20);
    }

    #[test]
    fn zero_vol_with_positive_lump_matches_closed_form() {
        // 30y, 7% return, $500/mo, no vol → P50 = $609,985.71. Now add a
        // $10,000 deposit at month 12. The lump grows for the remaining
        // 348 months at 0.5833%/mo → final lump_value = 10000 * (1.005833)^348.
        let mut input = defaults();
        input.lump_sums = vec![LumpSum {
            month_offset: 12,
            amount_cents: 1_000_000, // $10k
        }];
        let r = simulate(&input);
        let p50 = r.final_p50_cents;
        // Expected: 60_998_571 + 1_000_000 * (1.0058333)^348
        let r_m = 0.07_f64 / 12.0;
        let expected_lump_fv = 1_000_000.0_f64 * (1.0_f64 + r_m).powi(348);
        let expected = 60_998_571.0_f64 + expected_lump_fv;
        let diff = (p50 as f64 - expected).abs();
        assert!(
            diff / expected < 0.001,
            "zero-vol P50 with $10k lump at m=12 was {p50}, expected ~{expected:.0}"
        );
    }

    #[test]
    fn negative_lump_subtracts_from_balance() {
        // 30y, 7%, $500/mo, no vol → $609,985 normally. Withdraw $20k
        // at month 240 (year 20) and the result drops by the FV of $20k
        // compounded at 7%/mo for the remaining 120 months.
        let mut input = defaults();
        input.lump_sums = vec![LumpSum {
            month_offset: 240,
            amount_cents: -2_000_000, // -$20k
        }];
        let r = simulate(&input);
        let r_m = 0.07_f64 / 12.0;
        let expected_drag = 2_000_000.0_f64 * (1.0_f64 + r_m).powi(120);
        let expected = 60_998_571.0_f64 - expected_drag;
        let diff = (r.final_p50_cents as f64 - expected).abs();
        assert!(
            diff / expected.abs() < 0.001,
            "P50 with -$20k at m=240 was {}, expected ~{:.0}",
            r.final_p50_cents,
            expected
        );
    }

    #[test]
    fn withdrawal_exceeding_balance_clamps_at_zero() {
        // 1y horizon, $100/mo, no growth → balance ~$1,200 at month 12.
        // Then withdraw $5,000 at month 12 — balance would go to -$3,800,
        // but the simulator clamps at zero.
        let mut input = defaults();
        input.starting_balance_cents = 0;
        input.monthly_contribution_cents = 10_000; // $100/mo
        input.annual_return_pct = 0.0;
        input.horizon_years = 1;
        input.lump_sums = vec![LumpSum {
            month_offset: 12,
            amount_cents: -500_000, // -$5k
        }];
        let r = simulate(&input);
        assert_eq!(
            r.final_p50_cents, 0,
            "huge withdrawal should clamp the path at zero"
        );
    }

    #[test]
    fn lump_at_month_offset_zero_is_silently_ignored() {
        // month_offset = 0 is out of [1, n_months], so the lump is dropped.
        // Result should match the no-lump case exactly.
        let baseline = simulate(&defaults());
        let mut input = defaults();
        input.lump_sums = vec![LumpSum {
            month_offset: 0,
            amount_cents: 9_999_999_999,
        }];
        let with_oob = simulate(&input);
        assert_eq!(baseline.final_p50_cents, with_oob.final_p50_cents);
    }

    #[test]
    fn duplicate_month_offset_lumps_sum() {
        // Two $5k lumps at the same month should equal one $10k lump.
        let mut a = defaults();
        a.lump_sums = vec![
            LumpSum {
                month_offset: 36,
                amount_cents: 500_000,
            },
            LumpSum {
                month_offset: 36,
                amount_cents: 500_000,
            },
        ];
        let mut b = defaults();
        b.lump_sums = vec![LumpSum {
            month_offset: 36,
            amount_cents: 1_000_000,
        }];
        let ra = simulate(&a);
        let rb = simulate(&b);
        assert_eq!(ra.final_p50_cents, rb.final_p50_cents);
    }

    #[test]
    fn goal_probability_responds_to_lump_sums() {
        // Adding a big lump should monotonically increase goal probability.
        let mut input = defaults();
        input.annual_volatility_pct = 12.0;
        input.n_paths = 1000;
        let p_no_lump = goal_probability(&input, 70_000_000);
        input.lump_sums = vec![LumpSum {
            month_offset: 6,
            amount_cents: 5_000_000, // $50k
        }];
        let p_with_lump = goal_probability(&input, 70_000_000);
        assert!(
            p_with_lump > p_no_lump,
            "p_no_lump={p_no_lump}, p_with_lump={p_with_lump} — should be strictly higher"
        );
    }

    #[test]
    fn zero_withdrawal_rate_matches_baseline() {
        // withdrawal_rate_pct = 0 must be byte-identical to the
        // pre-feature baseline. Same seed + n_paths must give same P50.
        let baseline = simulate(&defaults());
        let mut input = defaults();
        input.withdrawal_rate_pct = 0.0;
        let with = simulate(&input);
        assert_eq!(baseline.final_p50_cents, with.final_p50_cents);
    }

    #[test]
    fn higher_withdrawal_monotonically_reduces_final_balance() {
        let mut input = defaults();
        input.starting_balance_cents = 100_000_000; // $1M start
        input.monthly_contribution_cents = 0;
        input.annual_volatility_pct = 0.0; // deterministic for test stability
        let mut last = i64::MAX;
        for rate in [0.0, 2.0, 4.0, 6.0, 8.0] {
            input.withdrawal_rate_pct = rate;
            let r = simulate(&input);
            assert!(
                r.final_p50_cents < last,
                "rate {rate}: balance {} not strictly less than prior {last}",
                r.final_p50_cents
            );
            last = r.final_p50_cents;
        }
    }

    #[test]
    fn constant_pct_withdrawal_asymptotes_does_not_deplete() {
        // Constant-percent-of-current-balance withdrawal should drive
        // the balance toward an asymptote (or to zero with negative
        // real return), but never to a hard zero in finite time at
        // moderate rates. With 7% return and 4% withdrawal there's
        // 3% real growth left, so balance should *grow*.
        let mut input = defaults();
        input.starting_balance_cents = 100_000_000;
        input.monthly_contribution_cents = 0;
        input.annual_volatility_pct = 0.0;
        input.withdrawal_rate_pct = 4.0;
        let r = simulate(&input);
        assert!(
            r.final_p50_cents > 50_000_000,
            "30y of 7% return − 4% withdrawal should still leave > half the principal; got {}",
            r.final_p50_cents
        );
    }

    #[test]
    fn excessive_withdrawal_drains_to_near_zero() {
        // Withdrawing more than the real return per year drains the
        // portfolio asymptotically. At 7% return, 15% withdrawal is
        // -8%/yr real, so over 30 years the balance shrinks to a small
        // fraction of starting.
        let mut input = defaults();
        input.starting_balance_cents = 100_000_000;
        input.monthly_contribution_cents = 0;
        input.annual_volatility_pct = 0.0;
        input.withdrawal_rate_pct = 15.0;
        let r = simulate(&input);
        assert!(
            r.final_p50_cents < 10_000_000,
            "30y of 7% return − 15% withdrawal should leave well under 10% of principal; got {}",
            r.final_p50_cents
        );
    }

    #[test]
    fn swr_deterministic_pmt_drains_balance_in_remaining_months() {
        // Sanity: the SWR returned should be exactly the rate that, when
        // applied as a constant nominal $/year (annuity), drains a
        // $100k balance to zero in 30 years at 7% real return.
        let pct = swr_deterministic_pct(7.0, 0.0, 360);
        // Standard PMT for 7%/12 monthly over 360 months on $1 PV gives
        // monthly = $0.006653; annual ≈ $0.07984; so swr ≈ 7.984%.
        assert!((pct - 7.984).abs() < 0.01, "expected ≈ 7.984%, got {pct}");
    }

    #[test]
    fn swr_deterministic_uses_real_return() {
        // 7% nominal − 2.5% inflation = 4.5% real → SWR ≈ 6.07% over 30y.
        let pct = swr_deterministic_pct(7.0, 2.5, 360);
        assert!(
            (pct - 6.07).abs() < 0.05,
            "expected ≈ 6.07% (PMT at 4.5% real over 30y), got {pct}"
        );
    }

    #[test]
    fn swr_deterministic_zero_return_is_linear() {
        // r = 0 → SWR = 12 / n × 100. For n=120 (10 years), SWR = 10.0%.
        assert!((swr_deterministic_pct(0.0, 0.0, 120) - 10.0).abs() < 1e-9);
        // Equal-real-return = 0 (return matches inflation): same shape.
        assert!((swr_deterministic_pct(2.5, 2.5, 60) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn swr_deterministic_zero_at_horizon_end() {
        assert_eq!(swr_deterministic_pct(7.0, 2.5, 0), 0.0);
        assert_eq!(swr_deterministic_pct(7.0, 2.5, -5), 0.0);
    }

    #[test]
    fn swr_rises_as_remaining_horizon_shrinks() {
        // Mechanically: less time horizon means a higher annuity rate
        // can be sustained.
        let s_30y = swr_deterministic_pct(7.0, 0.0, 360);
        let s_15y = swr_deterministic_pct(7.0, 0.0, 180);
        let s_5y = swr_deterministic_pct(7.0, 0.0, 60);
        assert!(s_5y > s_15y);
        assert!(s_15y > s_30y);
    }

    #[test]
    fn even_grid_includes_endpoints_and_no_dupes() {
        let g = even_grid(120, 12);
        assert_eq!(g[0], 0);
        assert_eq!(*g.last().unwrap(), 120);
        for w in g.windows(2) {
            assert!(w[1] > w[0]);
        }
    }
}
