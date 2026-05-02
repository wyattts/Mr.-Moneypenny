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
    let mut hits = 0usize;
    let mut rng = match input.seed {
        Some(s) => rand::rngs::StdRng::seed_from_u64(s),
        None => rand::rngs::StdRng::from_entropy(),
    };
    for _ in 0..n_paths {
        let mut value = input.starting_balance_cents as f64;
        for _ in 1..=n_months {
            let r = sample_normal(&mut rng, mu_monthly, sigma_monthly);
            value = value * (1.0 + r) + input.monthly_contribution_cents as f64;
        }
        if value as i64 >= target_cents {
            hits += 1;
        }
    }
    hits as f64 / n_paths as f64
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
    fn even_grid_includes_endpoints_and_no_dupes() {
        let g = even_grid(120, 12);
        assert_eq!(g[0], 0);
        assert_eq!(*g.last().unwrap(), 120);
        for w in g.windows(2) {
            assert!(w[1] > w[0]);
        }
    }
}
