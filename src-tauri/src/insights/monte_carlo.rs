//! Monte Carlo investment-path simulation.
//!
//! Runs 1,000 paths (configurable) where each month's return is drawn
//! from a Normal distribution: μ = annual_rate/12, σ = annual_vol/√12.
//! At each requested time-step we sort the path values across paths and
//! extract percentiles, producing probability bands the UI overlays
//! on the deterministic projection from `forecast::project_investment`.
//!
//! ## Why parametric Normal (and not bootstrap)?
//!
//! Bootstrap from historical returns is more accurate when applicable
//! but requires ≥12 months of investing-category contribution + balance
//! data, which most users won't have for a long time after install.
//! See `Patches/v0.3.3.md` decision D2 for the trade-off discussion.
//!
//! ## Volatility presets
//!
//! Tied to the user's chosen return preset on the calculator UI:
//!
//! | Preset                | Return | σ (annual) |
//! |-----------------------|--------|------------|
//! | Conservative (HYSA)   | 4%     | 5%         |
//! | Balanced (60/40)      | 7%     | 10%        |
//! | Stock-heavy (S&P)     | 10%    | 15%        |
//!
//! These match historical asset-class numbers within the precision
//! that matters for personal-finance forecasting.

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
    /// How many time-steps to report bands at. Defaults to 30 (matches
    /// `project_investment` trajectory_points). Always includes t=0
    /// and t=horizon.
    pub time_points: u32,
    /// Optional fixed RNG seed for reproducibility (tests + replays).
    /// None → thread RNG, fresh each call.
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthBand {
    pub month: u32,
    pub p5: i64,
    pub p10: i64,
    pub p25: i64,
    pub p50: i64,
    pub p75: i64,
    pub p90: i64,
    pub p95: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathBands {
    pub points: Vec<MonthBand>,
    pub final_p5_cents: i64,
    pub final_p50_cents: i64,
    pub final_p95_cents: i64,
    pub n_paths: u32,
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
    let mu_monthly = input.annual_return_pct / 100.0 / 12.0;
    let sigma_monthly = (input.annual_volatility_pct / 100.0) / 12_f64.sqrt();

    // Pre-compute the months we'll snapshot: evenly spaced including
    // 0 and n_months.
    let snapshot_months = even_grid(n_months, time_points);

    // Allocate one column per snapshot row × n_paths so we can sort
    // in-place per-snapshot at the end.
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

    // For each snapshot row, sort and extract percentiles.
    let mut points = Vec::with_capacity(snapshot_months.len());
    for (i, m) in snapshot_months.iter().enumerate() {
        let col = &mut grid[i];
        col.sort_unstable();
        points.push(MonthBand {
            month: *m as u32,
            p5: percentile(col, 5),
            p10: percentile(col, 10),
            p25: percentile(col, 25),
            p50: percentile(col, 50),
            p75: percentile(col, 75),
            p90: percentile(col, 90),
            p95: percentile(col, 95),
        });
    }

    let last = points.last().expect("at least t=horizon present");
    PathBands {
        final_p5_cents: last.p5,
        final_p50_cents: last.p50,
        final_p95_cents: last.p95,
        points,
        n_paths: n_paths as u32,
    }
}

/// Goal-probability variant: counts what fraction of paths end above
/// `target_cents`. Reuses the same engine.
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
/// We discard the second one for code simplicity (~20% throughput hit
/// vs caching it; not worth the complexity for 1k-360k samples).
fn sample_normal<R: rand::Rng>(rng: &mut R, mean: f64, sd: f64) -> f64 {
    let u: Uniform<f64> = Uniform::new_inclusive(f64::EPSILON, 1.0);
    let u1 = u.sample(rng);
    let u2 = u.sample(rng);
    let z0 = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
    mean + sd * z0
}

/// Generate `n` evenly spaced months from 0 to `total`, inclusive of
/// both endpoints. Always returns at least [0, total]; may collapse
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
fn percentile(sorted: &[i64], p: u32) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let p = p.clamp(0, 100) as f64 / 100.0;
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

    #[test]
    fn zero_volatility_p50_matches_closed_form() {
        // With σ=0 the simulation collapses to deterministic compounding.
        // $0 + $500/mo @ 7% × 30y → $609,985.71 (Excel FV).
        let r = simulate(&PathInput {
            starting_balance_cents: 0,
            monthly_contribution_cents: 50_000,
            annual_return_pct: 7.0,
            annual_volatility_pct: 0.0,
            horizon_years: 30,
            n_paths: 200,
            time_points: 30,
            seed: Some(42),
        });
        // All percentiles should equal the deterministic answer when σ=0.
        let p50 = r.final_p50_cents;
        assert!(
            close_pct(p50, 60_998_571, 0.001),
            "P50 {} far from $609,985.71",
            p50
        );
        assert_eq!(r.final_p5_cents, p50);
        assert_eq!(r.final_p95_cents, p50);
    }

    #[test]
    fn nonzero_volatility_produces_band_spread() {
        let r = simulate(&PathInput {
            starting_balance_cents: 0,
            monthly_contribution_cents: 50_000,
            annual_return_pct: 7.0,
            annual_volatility_pct: 15.0,
            horizon_years: 30,
            n_paths: 1000,
            time_points: 30,
            seed: Some(42),
        });
        // P95 should be meaningfully above P5 with 15% vol.
        assert!(r.final_p95_cents > r.final_p5_cents);
        assert!(r.final_p95_cents > r.final_p50_cents);
        assert!(r.final_p50_cents > r.final_p5_cents);
        // Sanity: deterministic FV is ~$610k. With 15% vol, the band
        // should span at least ±20% around that.
        assert!(r.final_p5_cents < 50_000_000);
        assert!(r.final_p95_cents > 70_000_000);
    }

    #[test]
    fn seed_makes_results_reproducible() {
        let make = || {
            simulate(&PathInput {
                starting_balance_cents: 1_000_000,
                monthly_contribution_cents: 100_000,
                annual_return_pct: 7.0,
                annual_volatility_pct: 10.0,
                horizon_years: 10,
                n_paths: 200,
                time_points: 12,
                seed: Some(123),
            })
        };
        let a = make();
        let b = make();
        assert_eq!(a.final_p50_cents, b.final_p50_cents);
        assert_eq!(a.final_p5_cents, b.final_p5_cents);
    }

    #[test]
    fn goal_probability_zero_vol_decisive() {
        // σ=0, contribution=$500/mo, 30y, target=$1M (well above $610k FV).
        // Probability should be 0 with no variance.
        let p = goal_probability(
            &PathInput {
                starting_balance_cents: 0,
                monthly_contribution_cents: 50_000,
                annual_return_pct: 7.0,
                annual_volatility_pct: 0.0,
                horizon_years: 30,
                n_paths: 200,
                time_points: 30,
                seed: Some(42),
            },
            100_000_000,
        );
        assert!(p < 0.01, "expected ~0 probability, got {p}");
    }

    #[test]
    fn goal_probability_with_vol_is_in_range() {
        // σ=15%, target = deterministic FV. Should be roughly 50% by
        // symmetry of the distribution (slightly below 50% because
        // log-normal drift, but in the 35-55% ballpark).
        let p = goal_probability(
            &PathInput {
                starting_balance_cents: 0,
                monthly_contribution_cents: 50_000,
                annual_return_pct: 7.0,
                annual_volatility_pct: 15.0,
                horizon_years: 30,
                n_paths: 1000,
                time_points: 30,
                seed: Some(42),
            },
            60_998_571,
        );
        assert!(
            (0.30..=0.60).contains(&p),
            "probability {p} outside expected 0.30..0.60 band"
        );
    }

    #[test]
    fn percentile_linear_interpolation() {
        let s = vec![10, 20, 30, 40, 50];
        assert_eq!(percentile(&s, 0), 10);
        assert_eq!(percentile(&s, 100), 50);
        assert_eq!(percentile(&s, 50), 30);
        // 25%: idx = 1.0 → s[1] = 20
        assert_eq!(percentile(&s, 25), 20);
    }

    #[test]
    fn even_grid_includes_endpoints_and_no_dupes() {
        let g = even_grid(120, 12);
        assert_eq!(g[0], 0);
        assert_eq!(*g.last().unwrap(), 120);
        // Strictly increasing.
        for w in g.windows(2) {
            assert!(w[1] > w[0]);
        }
    }

    #[test]
    fn even_grid_handles_short_horizon() {
        let g = even_grid(3, 30);
        // total=3 with 30 points → at most 4 unique values [0,1,2,3]
        assert_eq!(g[0], 0);
        assert_eq!(*g.last().unwrap(), 3);
        for w in g.windows(2) {
            assert!(w[1] > w[0]);
        }
    }
}
