//! Forward-looking forecast tools for the v0.3.0 power-user view.
//!
//! All deterministic — Monte Carlo / bootstrap variants land in v0.3.1.
//!
//! ## What's here
//!
//! - **Investment projection**: starting balance + monthly contribution +
//!   annual return + horizon → trajectory + final value, in nominal and
//!   (optional) inflation-adjusted dollars.
//! - **Goal-seek**: target balance + horizon + return rate + starting balance
//!   → required monthly contribution. Algebraic inverse of the future-value
//!   formula.
//! - **Scenario delta**: given the user's current variable budget and a list
//!   of `(category_id, pct_change)` cuts/bumps, compute the resulting
//!   variable-budget delta. (Pure subtraction; sits here for discoverability
//!   alongside the other forecast tools.)
//!
//! ## Compounding convention
//!
//! Monthly compounding everywhere. `r_monthly = annual / 12`. Deposits
//! are end-of-month (ordinary annuity), matching how someone budgeting
//! via Mr. Moneypenny would actually contribute. Future-value formula:
//!
//! ```text
//! FV = P(1+r)^n + C * ((1+r)^n - 1) / r
//! ```
//!
//! where P = starting balance, C = monthly contribution, r = monthly
//! rate, n = months in horizon. When r ≈ 0 we collapse to the
//! straight-line approximation (P + C*n) to avoid divide-by-zero.

use serde::{Deserialize, Serialize};

/// One point on the investment trajectory curve.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectionPoint {
    /// Months elapsed since the projection started (0 = present).
    pub month: u32,
    /// Nominal dollar value, in cents.
    pub nominal_cents: i64,
    /// Inflation-adjusted (real) value in present-day cents. Set equal
    /// to nominal when inflation_pct is 0.
    pub real_cents: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectInvestmentInput {
    pub starting_balance_cents: i64,
    pub monthly_contribution_cents: i64,
    /// Annual nominal return as a percentage (e.g., 7.0 for 7%).
    pub annual_return_pct: f64,
    /// Annual inflation as a percentage. 0 disables real-vs-nominal.
    pub annual_inflation_pct: f64,
    pub horizon_years: u32,
    /// How many points to include in the trajectory. Caller picks based
    /// on horizon (12 ≈ one per year for ≤12yr, monthly otherwise).
    pub trajectory_points: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct InvestmentProjection {
    pub trajectory: Vec<ProjectionPoint>,
    pub final_nominal_cents: i64,
    pub final_real_cents: i64,
    pub total_contributed_cents: i64,
    /// final_nominal - starting - total_contributed.
    pub total_growth_cents: i64,
}

pub fn project_investment(input: &ProjectInvestmentInput) -> InvestmentProjection {
    let n_months = (input.horizon_years as i64) * 12;
    let r_monthly = input.annual_return_pct / 100.0 / 12.0;
    let infl_monthly = input.annual_inflation_pct / 100.0 / 12.0;

    let p = input.starting_balance_cents as f64;
    let c = input.monthly_contribution_cents as f64;

    let mut trajectory: Vec<ProjectionPoint> = Vec::new();
    let pts = input.trajectory_points.max(2) as i64;
    let step = (n_months as f64 / pts as f64).max(1.0);

    let nominal_at = |months: i64| -> f64 { future_value(p, c, r_monthly, months) };
    let deflate = |nominal: f64, months: i64| -> f64 {
        if infl_monthly.abs() < 1e-12 {
            nominal
        } else {
            nominal / (1.0 + infl_monthly).powi(months as i32)
        }
    };

    // Always include t=0 and t=horizon. Fill in `pts-1` evenly-spaced
    // intermediate points so the trajectory line is smooth.
    let mut last_pushed: i64 = -1;
    for i in 0..=pts {
        let m = ((i as f64) * step).round() as i64;
        let m = m.min(n_months);
        if m == last_pushed {
            continue;
        }
        last_pushed = m;
        let nom = nominal_at(m).round() as i64;
        let real = deflate(nominal_at(m), m).round() as i64;
        trajectory.push(ProjectionPoint {
            month: m as u32,
            nominal_cents: nom,
            real_cents: real,
        });
    }
    // Defensive: ensure horizon endpoint present.
    if trajectory.last().map(|p| p.month as i64) != Some(n_months) {
        let nom = nominal_at(n_months).round() as i64;
        let real = deflate(nominal_at(n_months), n_months).round() as i64;
        trajectory.push(ProjectionPoint {
            month: n_months as u32,
            nominal_cents: nom,
            real_cents: real,
        });
    }

    let final_nominal_cents = nominal_at(n_months).round() as i64;
    let final_real_cents = deflate(nominal_at(n_months), n_months).round() as i64;
    let total_contributed_cents =
        (input.monthly_contribution_cents as i128 * n_months as i128) as i64;
    let total_growth_cents =
        final_nominal_cents - input.starting_balance_cents - total_contributed_cents;

    InvestmentProjection {
        trajectory,
        final_nominal_cents,
        final_real_cents,
        total_contributed_cents,
        total_growth_cents,
    }
}

/// Closed-form future value of P + monthly C with monthly rate r over
/// n months. Handles r=0 with the straight-line collapse.
fn future_value(p: f64, c: f64, r: f64, n_months: i64) -> f64 {
    if r.abs() < 1e-12 {
        return p + c * n_months as f64;
    }
    let growth = (1.0 + r).powi(n_months as i32);
    p * growth + c * (growth - 1.0) / r
}

#[derive(Debug, Clone, Deserialize)]
pub struct GoalSeekInput {
    pub target_cents: i64,
    pub starting_balance_cents: i64,
    pub annual_return_pct: f64,
    pub horizon_years: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GoalSeekResult {
    pub required_monthly_cents: i64,
    /// True when the starting balance + return alone exceeds the
    /// target — no contribution required (or contribution can be zero).
    pub already_on_track: bool,
}

/// Solve the FV formula for `C` (monthly contribution).
///
/// FV = P(1+r)^n + C * ((1+r)^n - 1) / r
/// → C = (FV - P(1+r)^n) * r / ((1+r)^n - 1)
pub fn solve_goal_seek(input: &GoalSeekInput) -> GoalSeekResult {
    let n_months = (input.horizon_years as i64) * 12;
    if n_months <= 0 {
        // Caller asked for "today" — required monthly is the entire gap.
        let required = (input.target_cents - input.starting_balance_cents).max(0);
        return GoalSeekResult {
            required_monthly_cents: required,
            already_on_track: required == 0,
        };
    }
    let r = input.annual_return_pct / 100.0 / 12.0;
    let p = input.starting_balance_cents as f64;
    let target = input.target_cents as f64;

    let growth = if r.abs() < 1e-12 {
        1.0
    } else {
        (1.0 + r).powi(n_months as i32)
    };
    let p_grown = p * growth;
    if p_grown >= target {
        return GoalSeekResult {
            required_monthly_cents: 0,
            already_on_track: true,
        };
    }
    let needed = target - p_grown;
    let monthly = if r.abs() < 1e-12 {
        needed / n_months as f64
    } else {
        needed * r / (growth - 1.0)
    };
    GoalSeekResult {
        required_monthly_cents: monthly.ceil() as i64,
        already_on_track: false,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioCut {
    pub category_id: i64,
    /// Percent change. -25.0 means cut 25%; +10.0 means raise the cap 10%.
    pub pct_change: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    pub original_variable_budget_cents: i64,
    pub adjusted_variable_budget_cents: i64,
    /// Per-cut detail (positive when cap dropped, negative when raised).
    pub savings_per_year_cents: i64,
}

/// Apply a set of percentage adjustments to a list of category targets
/// and report the annualized savings/spend impact. Caller passes in the
/// list of (category_id, original_target_cents) for the variable
/// categories we're projecting; cuts not in the list are ignored.
pub fn scenario_delta(targets: &[(i64, i64)], cuts: &[ScenarioCut]) -> ScenarioResult {
    let original_variable_budget_cents: i64 = targets.iter().map(|(_, t)| *t).sum();
    let mut adjusted = original_variable_budget_cents;
    for cut in cuts {
        if let Some(&(_, target)) = targets.iter().find(|(id, _)| *id == cut.category_id) {
            // delta = target * pct_change / 100
            let delta = (target as f64 * cut.pct_change / 100.0).round() as i64;
            adjusted = adjusted.saturating_add(delta);
        }
    }
    let savings_per_year_cents = (original_variable_budget_cents - adjusted).saturating_mul(12);
    ScenarioResult {
        original_variable_budget_cents,
        adjusted_variable_budget_cents: adjusted,
        savings_per_year_cents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close_enough(a: i64, b: i64, tol: i64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn investment_projection_zero_return_is_straight_line() {
        let p = project_investment(&ProjectInvestmentInput {
            starting_balance_cents: 100_000,    // $1000
            monthly_contribution_cents: 50_000, // $500/mo
            annual_return_pct: 0.0,
            annual_inflation_pct: 0.0,
            horizon_years: 1,
            trajectory_points: 12,
        });
        // After 12 months: 1000 + 500*12 = $7000 = 700_000 cents
        assert_eq!(p.final_nominal_cents, 700_000);
        assert_eq!(p.final_real_cents, 700_000);
        assert_eq!(p.total_contributed_cents, 600_000);
        assert_eq!(p.total_growth_cents, 0);
    }

    #[test]
    fn investment_projection_matches_excel_fv_formula() {
        // $0 starting + $500/mo + 7% annual + 30 years, ordinary annuity
        // (deposits end-of-month). Excel: =FV(0.07/12, 360, -500, 0, 0)
        // = $609,985.71. We're storing in cents → 60_998_571.
        let p = project_investment(&ProjectInvestmentInput {
            starting_balance_cents: 0,
            monthly_contribution_cents: 50_000,
            annual_return_pct: 7.0,
            annual_inflation_pct: 0.0,
            horizon_years: 30,
            trajectory_points: 30,
        });
        // Allow $5 of rounding wiggle (500 cents).
        assert!(
            close_enough(p.final_nominal_cents, 60_998_571, 500),
            "got {} expected ~60_998_571",
            p.final_nominal_cents
        );
    }

    #[test]
    fn investment_projection_with_inflation_deflates_to_real() {
        // Same as above but with 2.5% inflation. Real value should be
        // smaller than nominal — and meaningfully so over 30 years.
        //
        // Note: this differs from the Fisher real-rate approximation
        // (\"compound at 4.4%\") because contributions are NOMINAL — fixed
        // $500 today, the same fixed $500 in year 30, which is worth
        // much less by then. So the proper computation is:
        //   1. Compute nominal FV at 7% over 30y of $500/mo: $609,986
        //   2. Deflate by (1 + 0.025/12)^360 = ~2.115
        //   3. Real FV ≈ $288,361
        let p = project_investment(&ProjectInvestmentInput {
            starting_balance_cents: 0,
            monthly_contribution_cents: 50_000,
            annual_return_pct: 7.0,
            annual_inflation_pct: 2.5,
            horizon_years: 30,
            trajectory_points: 30,
        });
        assert!(p.final_real_cents < p.final_nominal_cents);
        assert!(
            close_enough(p.final_real_cents, 28_836_163, 50_000),
            "real value {} far from expected ~28_836_163",
            p.final_real_cents
        );
    }

    #[test]
    fn investment_trajectory_starts_at_starting_balance() {
        let p = project_investment(&ProjectInvestmentInput {
            starting_balance_cents: 500_000,
            monthly_contribution_cents: 0,
            annual_return_pct: 0.0,
            annual_inflation_pct: 0.0,
            horizon_years: 5,
            trajectory_points: 10,
        });
        let first = p.trajectory.first().unwrap();
        assert_eq!(first.month, 0);
        assert_eq!(first.nominal_cents, 500_000);
    }

    #[test]
    fn investment_trajectory_endpoint_matches_final() {
        let p = project_investment(&ProjectInvestmentInput {
            starting_balance_cents: 0,
            monthly_contribution_cents: 10_000,
            annual_return_pct: 5.0,
            annual_inflation_pct: 0.0,
            horizon_years: 10,
            trajectory_points: 12,
        });
        let last = p.trajectory.last().unwrap();
        assert_eq!(last.month, 120); // 10 years × 12
        assert_eq!(last.nominal_cents, p.final_nominal_cents);
    }

    #[test]
    fn goal_seek_inverts_projection() {
        // Goal-seek for $1M in 30 years at 7% from $0 start should give a
        // monthly that, when fed back into project_investment, lands at
        // ~$1M.
        let goal = solve_goal_seek(&GoalSeekInput {
            target_cents: 100_000_000, // $1M expressed in cents
            starting_balance_cents: 0,
            annual_return_pct: 7.0,
            horizon_years: 30,
        });
        assert!(!goal.already_on_track);
        let proj = project_investment(&ProjectInvestmentInput {
            starting_balance_cents: 0,
            monthly_contribution_cents: goal.required_monthly_cents,
            annual_return_pct: 7.0,
            annual_inflation_pct: 0.0,
            horizon_years: 30,
            trajectory_points: 12,
        });
        // Should land within $1k of target after rounding (we ceil the
        // monthly contribution).
        assert!(
            (proj.final_nominal_cents - 100_000_000).abs() < 100_000,
            "projected {} far from $1M target",
            proj.final_nominal_cents
        );
        assert!(
            proj.final_nominal_cents >= 100_000_000,
            "ceiling on contribution should leave us ≥ target"
        );
    }

    #[test]
    fn goal_seek_already_funded_returns_zero_monthly() {
        // $500k start, target $400k, 5 years, 5% return.
        // P grown alone exceeds target → already on track.
        let r = solve_goal_seek(&GoalSeekInput {
            // 40k and 50k dollars in cents.
            target_cents: 4_000_000,
            starting_balance_cents: 5_000_000,
            annual_return_pct: 5.0,
            horizon_years: 5,
        });
        assert!(r.already_on_track);
        assert_eq!(r.required_monthly_cents, 0);
    }

    #[test]
    fn scenario_cut_reduces_variable_budget() {
        let targets = vec![(1, 30_000), (2, 20_000), (3, 10_000)];
        let cuts = vec![ScenarioCut {
            category_id: 1,
            pct_change: -20.0, // cut category 1 by 20%
        }];
        let r = scenario_delta(&targets, &cuts);
        assert_eq!(r.original_variable_budget_cents, 60_000);
        // 20% of 30000 = 6000 → adjusted = 60000 - 6000 = 54000
        assert_eq!(r.adjusted_variable_budget_cents, 54_000);
        // Annual savings = 6000 × 12 = 72000
        assert_eq!(r.savings_per_year_cents, 72_000);
    }

    #[test]
    fn scenario_unknown_category_id_ignored() {
        let targets = vec![(1, 30_000)];
        let cuts = vec![ScenarioCut {
            category_id: 999,
            pct_change: -50.0,
        }];
        let r = scenario_delta(&targets, &cuts);
        assert_eq!(r.adjusted_variable_budget_cents, 30_000);
        assert_eq!(r.savings_per_year_cents, 0);
    }
}
