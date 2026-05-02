//! Bidirectional Monte Carlo simulator.
//!
//! Wraps `monte_carlo` to answer two complementary questions:
//!
//! - **Confidence-first** (`solve_required_contribution`): given a
//!   target / horizon / return / σ / starting balance and a confidence
//!   threshold, what's the smallest monthly contribution that crosses
//!   that threshold? Uses bisection over contribution, runs Monte
//!   Carlo at each candidate.
//! - **Contribution-first** (`compute_probability`): given the same
//!   inputs plus a fixed monthly contribution, what's the probability
//!   of hitting the target? One Monte Carlo run.
//!
//! Both modes share an `inflate_target` helper so "today's $" and
//! "nominal future $" target interpretations are handled identically.
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

#[derive(Debug, Clone, Serialize)]
pub struct RequiredContributionResult {
    pub required_monthly_cents: i64,
    pub realized_probability: f64,
    pub effective_target_cents: i64,
    pub final_p10_cents: i64,
    pub final_p50_cents: i64,
    pub final_p90_cents: i64,
    pub iterations: u32,
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
    pub final_p10_cents: i64,
    pub final_p50_cents: i64,
    pub final_p90_cents: i64,
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
/// plus useful summary statistics from the final Monte Carlo run.
pub fn solve_required_contribution(
    input: &RequiredContributionInput,
) -> RequiredContributionResult {
    let target_eff = effective_target(&input.common, input.common.horizon_years);
    let mut lo = 0i64;
    // Upper bound: 2× (target / horizon_months) is enough to cover
    // most realistic cases. We expand if probability at upper isn't
    // yet ≥ confidence.
    let n_months = (input.common.horizon_years as i64).max(1) * 12;
    let mut hi = (target_eff * 2 / n_months).max(10_000); // at least $100
                                                          // Probe upper bound; expand up to 4× if needed.
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

    // Now bisect.
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

    // Final Monte Carlo run at the answer to extract P10/P50/P90 for
    // the histogram.
    let bands = simulate(&PathInput {
        starting_balance_cents: input.common.starting_balance_cents,
        monthly_contribution_cents: last_required,
        annual_return_pct: input.common.annual_return_pct,
        annual_volatility_pct: input.common.annual_volatility_pct,
        horizon_years: input.common.horizon_years,
        n_paths: 1000.max(input.common.n_paths),
        time_points: 2,
        seed: input.common.seed,
    });

    RequiredContributionResult {
        required_monthly_cents: last_required,
        realized_probability: last_prob,
        effective_target_cents: target_eff,
        final_p10_cents: bands.final_p10_cents,
        final_p50_cents: bands.final_p50_cents,
        final_p90_cents: bands.final_p90_cents,
        iterations,
    }
}

/// Single Monte Carlo run reporting probability + summary stats.
pub fn compute_probability(input: &ProbabilityInput) -> ProbabilityResult {
    let target_eff = effective_target(&input.common, input.common.horizon_years);
    let probability = goal_probability(
        &mc_input(&input.common, input.monthly_contribution_cents),
        target_eff,
    );
    let bands = simulate(&PathInput {
        starting_balance_cents: input.common.starting_balance_cents,
        monthly_contribution_cents: input.monthly_contribution_cents,
        annual_return_pct: input.common.annual_return_pct,
        annual_volatility_pct: input.common.annual_volatility_pct,
        horizon_years: input.common.horizon_years,
        n_paths: 1000.max(input.common.n_paths),
        time_points: 2,
        seed: input.common.seed,
    });
    ProbabilityResult {
        probability,
        effective_target_cents: target_eff,
        final_p10_cents: bands.final_p10_cents,
        final_p50_cents: bands.final_p50_cents,
        final_p90_cents: bands.final_p90_cents,
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
            // Cells use fewer paths to keep latency low; UI rounds the
            // tooltip to the nearest 5% so 200 paths is enough.
            common.n_paths = 200;
            common.horizon_years = horizon;
            // Fix per-cell seed so heatmap renders deterministically
            // for a given input set; otherwise Recharts re-renders are
            // visually noisy from sample-to-sample variance.
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
        seed: common.seed,
    }
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
        let mut c = common(100_000_000, 30); // $1M today
        c.target_mode = TargetMode::TodaysDollars;
        c.annual_inflation_pct = 2.5;
        let nominal = effective_target(&c, c.horizon_years);
        // (1 + 0.025/12)^360 ≈ 2.115. So $1M today ≈ $2.115M nominal.
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
        // Solve "required at 80% confidence" then feed back into
        // "compute probability" — should land ≥ 80%.
        let req = solve_required_contribution(&RequiredContributionInput {
            common: common(100_000_000, 30),
            confidence: 0.80,
        });
        assert!(req.realized_probability >= 0.80);

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
    fn required_contribution_zero_vol_matches_algebraic_floor() {
        // σ=0 makes everything deterministic. Required contribution
        // should be just enough for the closed-form FV to equal target.
        // For target=$1M, 30y, 7%, σ=0: monthly ≈ $819.71.
        let mut c = common(100_000_000, 30);
        c.annual_volatility_pct = 0.0;
        let req = solve_required_contribution(&RequiredContributionInput {
            common: c,
            confidence: 0.50,
        });
        // Allow some bisection slack; should be in the ballpark.
        assert!(
            (60_000..=120_000).contains(&req.required_monthly_cents),
            "required {} cents/mo not near expected ~$820/mo",
            req.required_monthly_cents
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
        assert!(
            req_90.required_monthly_cents > req_50.required_monthly_cents,
            "90% required ({}) should exceed 50% required ({})",
            req_90.required_monthly_cents,
            req_50.required_monthly_cents
        );
    }

    #[test]
    fn today_dollars_target_requires_more_than_nominal() {
        // At positive inflation, today's-$ interpretation inflates the
        // target → user needs to contribute more.
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
    fn heatmap_monotonic_in_contribution() {
        let input = HeatmapInput {
            common: common(100_000_000, 30),
            contribution_min_cents: 0,
            contribution_max_cents: 400_000,
            horizon_min_years: 30,
            horizon_max_years: 30,
        };
        let r = heatmap(&input);
        // For any fixed horizon row, probabilities should be
        // non-decreasing in contribution.
        let row: Vec<f64> = r.cells.iter().take(12).map(|c| c.probability).collect();
        for w in row.windows(2) {
            // Allow small Monte Carlo noise, but should be roughly
            // non-decreasing.
            assert!(w[1] >= w[0] - 0.05, "row not monotonic: {:?}", row);
        }
    }

    #[test]
    fn heatmap_higher_horizon_higher_prob() {
        // For a fixed reasonable contribution and 1y vs 30y horizon,
        // longer horizon must give materially higher probability.
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
