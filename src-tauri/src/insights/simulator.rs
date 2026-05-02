//! Bidirectional Monte Carlo simulator.
//!
//! Wraps `monte_carlo` to answer two complementary questions:
//!
//! - **Confidence-first** (`solve_required_contribution`): given a
//!   target / horizon / return / σ / starting balance and a confidence
//!   threshold, what's the smallest monthly contribution that crosses
//!   that threshold? Uses bisection over contribution, runs Monte
//!   Carlo at each candidate. Probability bands on the returned
//!   trajectory match the user's chosen confidence.
//! - **Contribution-first** (`compute_probability`): given the same
//!   inputs plus a fixed monthly contribution, what's the probability
//!   of hitting the target? One Monte Carlo run. Probability bands on
//!   the returned trajectory match that resulting probability — so
//!   "the central X% of where you'd actually end up" lines up with
//!   "your chance of hitting target = X%."
//!
//! Both modes return a `trajectory` of per-month points with the
//! deterministic Nominal value, the inflation-adjusted Real value, the
//! cumulative-Contributions value, and the band edges (p_lo / p50 /
//! p_hi). The frontend draws all four traces from this single payload.
//!
//! ## Heatmap
//!
//! `heatmap` produces a 12×12 grid of probabilities over (contribution,
//! horizon) for the click-to-snap UI. Each cell uses 200 paths instead
//! of 1,000 — trades resolution for latency since the grid would
//! otherwise be 144 × 1000 = 144k paths. UI rounds tooltip values to
//! the nearest 5%.

use serde::{Deserialize, Serialize};

use super::monte_carlo::{goal_probability, simulate, PathInput};

/// Target dollar interpretation. Mirror of the UI toggle.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetMode {
    /// Target is in today's purchasing power. Inflate to nominal
    /// before checking the simulated paths.
    TodaysDollars,
    /// Target is nominal future dollars at the horizon date.
    NominalFuture,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommonInputs {
    pub target_cents: i64,
    pub horizon_years: u32,
    pub starting_balance_cents: i64,
    pub annual_return_pct: f64,
    pub annual_volatility_pct: f64,
    pub annual_inflation_pct: f64,
    pub target_mode: TargetMode,
    /// 1000 by default; lower for faster heatmap cells.
    pub n_paths: u32,
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RequiredContributionInput {
    #[serde(flatten)]
    pub common: CommonInputs,
    /// 0..1. UI sends e.g., 0.80 for 80% confidence.
    pub confidence: f64,
}

/// One time-step on the projection chart. Carries everything the UI
/// needs to draw all four traces (Nominal, Real, Contributions, band)
/// without a separate IPC call.
#[derive(Debug, Clone, Serialize)]
pub struct TrajectoryPoint {
    pub month: u32,
    /// Deterministic future value at `(starting_balance, monthly_c,
    /// annual_return)` — closed-form FV.
    pub nominal_cents: i64,
    /// Nominal deflated to today's purchasing power.
    pub real_cents: i64,
    /// Starting balance + monthly contribution × month elapsed.
    pub contributions_cents: i64,
    /// Lower band edge — `(1 - band_pct)/2` percentile.
    pub p_lo_cents: i64,
    /// Median of the simulated distribution.
    pub p50_cents: i64,
    /// Upper band edge — complement of `p_lo`.
    pub p_hi_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequiredContributionResult {
    pub required_monthly_cents: i64,
    pub realized_probability: f64,
    pub effective_target_cents: i64,
    pub final_p_lo_cents: i64,
    pub final_p50_cents: i64,
    pub final_p_hi_cents: i64,
    pub iterations: u32,
    pub band_pct: f64,
    pub trajectory: Vec<TrajectoryPoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProbabilityInput {
    #[serde(flatten)]
    pub common: CommonInputs,
    pub monthly_contribution_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbabilityResult {
    pub probability: f64,
    pub effective_target_cents: i64,
    pub final_p_lo_cents: i64,
    pub final_p50_cents: i64,
    pub final_p_hi_cents: i64,
    pub band_pct: f64,
    pub trajectory: Vec<TrajectoryPoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HeatmapInput {
    #[serde(flatten)]
    pub common: CommonInputs,
    /// Range for monthly contribution axis (X). Cells span
    /// [min..max] in 12 even steps.
    pub contribution_min_cents: i64,
    pub contribution_max_cents: i64,
    /// Range for horizon-years axis (Y). Cells span [min..max] in
    /// 12 even steps.
    pub horizon_min_years: u32,
    pub horizon_max_years: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeatmapCell {
    pub contribution_cents: i64,
    pub horizon_years: u32,
    pub probability: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeatmapResult {
    pub cells: Vec<HeatmapCell>,
    pub effective_target_cents_at_each_horizon: Vec<i64>,
}

/// Compute the inflated-to-nominal target if the user asked for
/// today's-$ semantics; pass through otherwise.
pub fn effective_target(common: &CommonInputs, horizon_years: u32) -> i64 {
    match common.target_mode {
        TargetMode::NominalFuture => common.target_cents,
        TargetMode::TodaysDollars => {
            let monthly_inflation = common.annual_inflation_pct / 100.0 / 12.0;
            let n_months = (horizon_years as i64) * 12;
            let factor = (1.0 + monthly_inflation).powi(n_months as i32);
            (common.target_cents as f64 * factor).round() as i64
        }
    }
}

/// Bisection over monthly contribution. Finds the smallest contribution
/// that yields probability ≥ confidence. Returns the search outcome
/// plus a per-month trajectory whose bands span `confidence` of the
/// outcome distribution at the answer.
pub fn solve_required_contribution(
    input: &RequiredContributionInput,
) -> RequiredContributionResult {
    let target_eff = effective_target(&input.common, input.common.horizon_years);
    let mut lo = 0i64;
    let n_months = (input.common.horizon_years as i64).max(1) * 12;
    let mut hi = (target_eff * 2 / n_months).max(10_000);
    let mut probe_input = mc_input(&input.common, hi);
    let mut p_hi = goal_probability(&probe_input, target_eff);
    let mut expansions = 0u32;
    while p_hi < input.confidence && expansions < 3 {
        hi *= 2;
        probe_input = mc_input(&input.common, hi);
        p_hi = goal_probability(&probe_input, target_eff);
        expansions += 1;
    }

    let mut iterations = 1u32 + expansions;

    let mut last_prob = p_hi;
    let mut last_required = hi;
    for _ in 0..14 {
        if hi - lo < 1000 {
            break;
        }
        let mid = lo + (hi - lo) / 2;
        let p = goal_probability(&mc_input(&input.common, mid), target_eff);
        iterations += 1;
        if p >= input.confidence {
            hi = mid;
            last_prob = p;
            last_required = mid;
        } else {
            lo = mid;
        }
    }

    // Final Monte Carlo run at the answer for the per-month trajectory.
    // Bands span the user's chosen confidence.
    let bands = simulate(&PathInput {
        starting_balance_cents: input.common.starting_balance_cents,
        monthly_contribution_cents: last_required,
        annual_return_pct: input.common.annual_return_pct,
        annual_volatility_pct: input.common.annual_volatility_pct,
        horizon_years: input.common.horizon_years,
        n_paths: 1000.max(input.common.n_paths),
        time_points: 30,
        band_pct: input.confidence,
        seed: input.common.seed,
    });

    let trajectory = build_trajectory(&input.common, last_required, &bands);

    RequiredContributionResult {
        required_monthly_cents: last_required,
        realized_probability: last_prob,
        effective_target_cents: target_eff,
        final_p_lo_cents: bands.final_p_lo_cents,
        final_p50_cents: bands.final_p50_cents,
        final_p_hi_cents: bands.final_p_hi_cents,
        iterations,
        band_pct: bands.band_pct,
        trajectory,
    }
}

/// Single Monte Carlo run reporting probability + trajectory. Bands
/// span the *resulting* probability so "central X% of outcomes" tracks
/// "X% chance of hitting target."
pub fn compute_probability(input: &ProbabilityInput) -> ProbabilityResult {
    let target_eff = effective_target(&input.common, input.common.horizon_years);
    let probability = goal_probability(
        &mc_input(&input.common, input.monthly_contribution_cents),
        target_eff,
    );
    // Use the just-computed probability as the band width. Clamp to
    // (0.01, 0.99) so degenerate runs don't request P0 or P100.
    let band_pct = probability.clamp(0.01, 0.99);
    let bands = simulate(&PathInput {
        starting_balance_cents: input.common.starting_balance_cents,
        monthly_contribution_cents: input.monthly_contribution_cents,
        annual_return_pct: input.common.annual_return_pct,
        annual_volatility_pct: input.common.annual_volatility_pct,
        horizon_years: input.common.horizon_years,
        n_paths: 1000.max(input.common.n_paths),
        time_points: 30,
        band_pct,
        seed: input.common.seed,
    });
    let trajectory = build_trajectory(&input.common, input.monthly_contribution_cents, &bands);
    ProbabilityResult {
        probability,
        effective_target_cents: target_eff,
        final_p_lo_cents: bands.final_p_lo_cents,
        final_p50_cents: bands.final_p50_cents,
        final_p_hi_cents: bands.final_p_hi_cents,
        band_pct: bands.band_pct,
        trajectory,
    }
}

/// 12×12 grid over (contribution, horizon) → probability.
pub fn heatmap(input: &HeatmapInput) -> HeatmapResult {
    let n = 12usize;
    let dc = (input.contribution_max_cents - input.contribution_min_cents) as f64 / (n - 1) as f64;
    let dh = (input.horizon_max_years as f64 - input.horizon_min_years as f64) / (n - 1) as f64;
    let mut cells = Vec::with_capacity(n * n);
    let mut targets_per_horizon = Vec::with_capacity(n);
    for j in 0..n {
        let horizon = (input.horizon_min_years as f64 + dh * j as f64).round() as u32;
        let target_eff = effective_target(&input.common, horizon);
        targets_per_horizon.push(target_eff);
        for i in 0..n {
            let contribution = (input.contribution_min_cents as f64 + dc * i as f64).round() as i64;
            let mut common = input.common.clone();
            common.n_paths = 200;
            common.horizon_years = horizon;
            let derived_seed = input
                .common
                .seed
                .unwrap_or(0xa5a5_a5a5_5a5a_5a5a)
                .wrapping_add((i as u64) << 32 | j as u64);
            common.seed = Some(derived_seed);

            let p = goal_probability(&mc_input(&common, contribution), target_eff);
            cells.push(HeatmapCell {
                contribution_cents: contribution,
                horizon_years: horizon,
                probability: p,
            });
        }
    }
    HeatmapResult {
        cells,
        effective_target_cents_at_each_horizon: targets_per_horizon,
    }
}

fn mc_input(common: &CommonInputs, monthly_contribution_cents: i64) -> PathInput {
    PathInput {
        starting_balance_cents: common.starting_balance_cents,
        monthly_contribution_cents,
        annual_return_pct: common.annual_return_pct,
        annual_volatility_pct: common.annual_volatility_pct,
        horizon_years: common.horizon_years,
        n_paths: if common.n_paths == 0 {
            1000
        } else {
            common.n_paths
        },
        time_points: 2,
        band_pct: 0.80,
        seed: common.seed,
    }
}

/// Build a per-month trajectory with deterministic nominal + real +
/// contributions traces alongside the Monte Carlo bands. Lengths align
/// 1:1 by month with the bands snapshot grid.
fn build_trajectory(
    common: &CommonInputs,
    monthly_contribution_cents: i64,
    bands: &super::monte_carlo::PathBands,
) -> Vec<TrajectoryPoint> {
    let r_monthly = common.annual_return_pct / 100.0 / 12.0;
    let infl_monthly = common.annual_inflation_pct / 100.0 / 12.0;
    let p = common.starting_balance_cents as f64;
    let c = monthly_contribution_cents as f64;

    bands
        .points
        .iter()
        .map(|b| {
            let m = b.month as i64;
            let nominal = if r_monthly.abs() < 1e-12 {
                p + c * m as f64
            } else {
                let g = (1.0 + r_monthly).powi(m as i32);
                p * g + c * (g - 1.0) / r_monthly
            };
            let real = if infl_monthly.abs() < 1e-12 {
                nominal
            } else {
                nominal / (1.0 + infl_monthly).powi(m as i32)
            };
            let contributions = p + c * m as f64;
            TrajectoryPoint {
                month: b.month,
                nominal_cents: nominal.round() as i64,
                real_cents: real.round() as i64,
                contributions_cents: contributions.round() as i64,
                p_lo_cents: b.p_lo,
                p50_cents: b.p50,
                p_hi_cents: b.p_hi,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn common(target: i64, horizon: u32) -> CommonInputs {
        CommonInputs {
            target_cents: target,
            horizon_years: horizon,
            starting_balance_cents: 0,
            annual_return_pct: 7.0,
            annual_volatility_pct: 10.0,
            annual_inflation_pct: 0.0,
            target_mode: TargetMode::NominalFuture,
            n_paths: 1000,
            seed: Some(42),
        }
    }

    #[test]
    fn effective_target_today_dollars_inflates_to_nominal() {
        let mut c = common(100_000_000, 30);
        c.target_mode = TargetMode::TodaysDollars;
        c.annual_inflation_pct = 2.5;
        let nominal = effective_target(&c, c.horizon_years);
        assert!(nominal > 200_000_000);
        assert!(nominal < 220_000_000);
    }

    #[test]
    fn effective_target_nominal_pass_through() {
        let c = common(100_000_000, 30);
        assert_eq!(effective_target(&c, 30), 100_000_000);
    }

    #[test]
    fn required_contribution_bidirectional_consistency() {
        let req = solve_required_contribution(&RequiredContributionInput {
            common: common(100_000_000, 30),
            confidence: 0.80,
        });
        assert!(req.realized_probability >= 0.80);
        assert!(!req.trajectory.is_empty());
        // Bands should match the requested confidence.
        assert!((req.band_pct - 0.80).abs() < 1e-9);

        let prob = compute_probability(&ProbabilityInput {
            common: common(100_000_000, 30),
            monthly_contribution_cents: req.required_monthly_cents,
        });
        assert!(
            prob.probability >= 0.78,
            "round-trip probability {} far below confidence threshold",
            prob.probability
        );
    }

    #[test]
    fn probability_mode_band_pct_tracks_probability() {
        let prob = compute_probability(&ProbabilityInput {
            common: common(100_000_000, 30),
            monthly_contribution_cents: 100_000,
        });
        // Allow tiny epsilon for the (0.01, 0.99) clamp.
        let expected = prob.probability.clamp(0.01, 0.99);
        assert!((prob.band_pct - expected).abs() < 1e-9);
    }

    #[test]
    fn required_mode_widens_bands_with_higher_confidence() {
        let req_70 = solve_required_contribution(&RequiredContributionInput {
            common: common(100_000_000, 30),
            confidence: 0.70,
        });
        let req_95 = solve_required_contribution(&RequiredContributionInput {
            common: common(100_000_000, 30),
            confidence: 0.95,
        });
        let span_70 = req_70.final_p_hi_cents - req_70.final_p_lo_cents;
        let span_95 = req_95.final_p_hi_cents - req_95.final_p_lo_cents;
        // 95% band must be wider than 70% band on the same shape.
        // Note: contributions also differ between solves (95% needs
        // more $/mo), so absolute amounts shift. Span is the test.
        assert!(
            span_95 > span_70,
            "95% span ({span_95}) should exceed 70% span ({span_70})"
        );
    }

    #[test]
    fn higher_confidence_requires_more_contribution() {
        let req_50 = solve_required_contribution(&RequiredContributionInput {
            common: common(100_000_000, 30),
            confidence: 0.50,
        });
        let req_90 = solve_required_contribution(&RequiredContributionInput {
            common: common(100_000_000, 30),
            confidence: 0.90,
        });
        assert!(req_90.required_monthly_cents > req_50.required_monthly_cents);
    }

    #[test]
    fn today_dollars_target_requires_more_than_nominal() {
        let mut c_nominal = common(100_000_000, 30);
        c_nominal.annual_inflation_pct = 2.5;
        let req_nominal = solve_required_contribution(&RequiredContributionInput {
            common: c_nominal,
            confidence: 0.80,
        });
        let mut c_today = common(100_000_000, 30);
        c_today.annual_inflation_pct = 2.5;
        c_today.target_mode = TargetMode::TodaysDollars;
        let req_today = solve_required_contribution(&RequiredContributionInput {
            common: c_today,
            confidence: 0.80,
        });
        assert!(req_today.required_monthly_cents > req_nominal.required_monthly_cents);
    }

    #[test]
    fn trajectory_includes_nominal_real_contributions_and_bands() {
        let req = solve_required_contribution(&RequiredContributionInput {
            common: {
                let mut c = common(100_000_000, 30);
                c.annual_inflation_pct = 2.5;
                c.target_mode = TargetMode::TodaysDollars;
                c
            },
            confidence: 0.80,
        });
        // With non-zero inflation, real should be < nominal.
        let last = req.trajectory.last().unwrap();
        assert!(last.real_cents < last.nominal_cents);
        // Contributions should equal starting + monthly × months_horizon.
        let expected_contrib = req.required_monthly_cents * 30 * 12;
        assert!(
            (last.contributions_cents - expected_contrib).abs() < 100,
            "contributions {} far from expected {}",
            last.contributions_cents,
            expected_contrib
        );
        // Band edges should bracket the median in the simulated set
        // (and roughly bracket the deterministic nominal too, though
        // skew can put nominal outside the band on heavy-tail draws).
        assert!(last.p_lo_cents <= last.p50_cents);
        assert!(last.p50_cents <= last.p_hi_cents);
    }

    #[test]
    fn heatmap_monotonic_in_contribution() {
        let input = HeatmapInput {
            common: common(100_000_000, 30),
            contribution_min_cents: 0,
            contribution_max_cents: 400_000,
            horizon_min_years: 30,
            horizon_max_years: 30,
        };
        let r = heatmap(&input);
        let row: Vec<f64> = r.cells.iter().take(12).map(|c| c.probability).collect();
        for w in row.windows(2) {
            assert!(w[1] >= w[0] - 0.05, "row not monotonic: {:?}", row);
        }
    }

    #[test]
    fn heatmap_higher_horizon_higher_prob() {
        let mut c1 = common(100_000_000, 1);
        c1.target_mode = TargetMode::NominalFuture;
        let p_short = compute_probability(&ProbabilityInput {
            common: c1,
            monthly_contribution_cents: 50_000,
        });

        let c30 = common(100_000_000, 30);
        let p_long = compute_probability(&ProbabilityInput {
            common: c30,
            monthly_contribution_cents: 50_000,
        });
        assert!(p_long.probability > p_short.probability);
    }
}
