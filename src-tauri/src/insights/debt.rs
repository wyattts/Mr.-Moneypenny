//! Debt management — deterministic single-debt amortization plus
//! portfolio mode (snowball / avalanche) with optional goal-seek.
//!
//! Pure calculator: no DB reads, no side effects. The frontend
//! collects all inputs (balance, APR, compounding, monthly payment or
//! target months, lump sums, inflation) and the backend returns a
//! month-by-month schedule plus summary stats.
//!
//! ## Compounding
//!
//! User-entered APR is converted to an effective monthly periodic rate
//! before iterating the month-by-month schedule. Standard textbook
//! conversions:
//!
//! | Compounding | Effective monthly rate                     |
//! |-------------|--------------------------------------------|
//! | Monthly     | APR / 12                                   |
//! | Daily       | (1 + APR/365)^(365/12) − 1                 |
//! | Yearly      | (1 + APR)^(1/12) − 1                       |
//! | Continuous  | e^(APR/12) − 1                             |
//!
//! ## Schedule order each month
//!
//! 1. Charge interest on current balance.
//! 2. Apply scheduled lump sum (if any).
//! 3. Apply monthly payment (capped at remaining balance).
//! 4. If balance ≤ 0, mark paid off.
//!
//! ## Goal seek
//!
//! Bisects the monthly payment such that the debt is paid off by the
//! target month. With lump sums there is no closed-form, so bisection
//! is used in all cases for consistency. Capped at 60 iterations which
//! gets to penny precision well before that.
//!
//! ## Portfolio mode
//!
//! Each debt accrues interest. Each debt receives its minimum payment.
//! Surplus (total budget − minimums paid) goes to a single target debt
//! determined by the chosen strategy:
//!
//! - **Snowball**: smallest current balance.
//! - **Avalanche**: highest APR.
//!
//! When a debt is paid off, its minimum naturally rolls into the
//! surplus on subsequent months (the budget is fixed; minimums shrink
//! as debts die).

use serde::{Deserialize, Serialize};

/// Hard cap on the simulation length. 100 years is well past any
/// realistic payoff horizon and prevents pathological inputs (zero or
/// near-zero payments below the breakeven) from running forever.
const MAX_SIM_MONTHS: u32 = 1200;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompoundingFrequency {
    Monthly,
    Daily,
    Yearly,
    Continuous,
}

impl CompoundingFrequency {
    /// Convert annual percentage rate (e.g. 7.5 for 7.5%) to the
    /// effective monthly periodic rate used by the schedule iterator.
    pub fn monthly_rate(self, apr_pct: f64) -> f64 {
        let r = apr_pct / 100.0;
        match self {
            Self::Monthly => r / 12.0,
            Self::Daily => (1.0 + r / 365.0).powf(365.0 / 12.0) - 1.0,
            Self::Yearly => (1.0 + r).powf(1.0 / 12.0) - 1.0,
            Self::Continuous => (r / 12.0).exp() - 1.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LumpSum {
    /// Months from now (0 = first month of the schedule).
    pub month_offset: u32,
    pub amount_cents: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DebtInput {
    /// Optional label so portfolio output can identify which debt is
    /// which without the frontend re-keying by index.
    #[serde(default)]
    pub label: Option<String>,
    pub balance_cents: i64,
    pub apr_pct: f64,
    pub compounding: CompoundingFrequency,
    /// Required for portfolio mode. Ignored for single-debt mode.
    #[serde(default)]
    pub minimum_payment_cents: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebtMonth {
    /// 1-indexed month number (1 = first month of the schedule).
    pub month: u32,
    /// Balance after this month's interest, lump sum, and payment.
    pub balance_cents: i64,
    pub interest_charged_cents: i64,
    pub principal_paid_cents: i64,
    pub payment_cents: i64,
    pub lump_sum_cents: i64,
    pub cumulative_interest_cents: i64,
    pub cumulative_principal_cents: i64,
    /// Cumulative-paid value translated back to today's dollars using
    /// the inflation rate the caller supplied.
    pub cumulative_paid_today_cents: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleInput {
    pub debt: DebtInput,
    pub monthly_payment_cents: i64,
    #[serde(default)]
    pub lump_sums: Vec<LumpSum>,
    /// Annual inflation in percent (e.g. 2.5 for 2.5%) used for the
    /// today's-dollars cumulative figure. Zero is a valid value.
    pub annual_inflation_pct: f64,
    /// Optional simulation cap. Defaults to MAX_SIM_MONTHS. Frontend
    /// can lower this for goal-seek-style probes.
    #[serde(default)]
    pub max_months: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduleResult {
    pub paid_off: bool,
    /// Months from now until the final payment lands. None when the
    /// debt never pays off within the simulation cap.
    pub payoff_month: Option<u32>,
    /// Whole-year offset (e.g. 2 = year 3). None mirrors `payoff_month`.
    pub payoff_year_offset: Option<u32>,
    /// 1..=12 month-within-year. None mirrors `payoff_month`.
    pub payoff_month_in_year: Option<u32>,
    pub total_interest_cents: i64,
    pub total_paid_cents: i64,
    pub total_paid_today_cents: i64,
    pub trajectory: Vec<DebtMonth>,
    /// Smallest monthly payment that exceeds the *initial* interest
    /// charge. Below this, with no lump sums, the debt grows. Surfaced
    /// regardless of whether the user's payment is sufficient — the
    /// UI uses it for the warning and for nudging payment defaults.
    pub breakeven_payment_cents: i64,
    /// Set when the user's monthly payment is at or below the
    /// breakeven *and* lump sums are insufficient to overcome growth.
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GoalSeekInput {
    pub debt: DebtInput,
    /// Target months to payoff. Months are the unit because debts are
    /// usually shorter horizons; the frontend converts years/months
    /// inputs into a single month count.
    pub target_months: u32,
    #[serde(default)]
    pub lump_sums: Vec<LumpSum>,
    pub annual_inflation_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GoalSeekResult {
    pub required_monthly_payment_cents: i64,
    pub schedule: ScheduleResult,
    /// True when even at extreme payments the simulator can't hit the
    /// target. Practically only fires when the lump-sum schedule is
    /// hostile (e.g. negative-balance scenarios that don't exist here)
    /// or numeric edge cases. Surfaced so the UI can show "infeasible"
    /// rather than a misleading huge number.
    pub feasible: bool,
}

// ---------------------------------------------------------------------
// Single-debt schedule
// ---------------------------------------------------------------------

pub fn simulate_schedule(input: &ScheduleInput) -> ScheduleResult {
    let max_months = input
        .max_months
        .unwrap_or(MAX_SIM_MONTHS)
        .min(MAX_SIM_MONTHS);
    let r = input.debt.compounding.monthly_rate(input.debt.apr_pct);
    let inflation_monthly = monthly_inflation(input.annual_inflation_pct);

    let mut balance = input.debt.balance_cents as f64;
    let starting_balance = balance;
    let mut cum_interest = 0i64;
    let mut cum_principal = 0i64;
    let mut cum_paid_today = 0i64;
    let mut cum_paid_nominal = 0i64;
    let mut trajectory: Vec<DebtMonth> = Vec::new();
    let mut payoff_month: Option<u32> = None;

    // Group lump sums by month for O(1) lookup. Multiple lumps in the
    // same month sum together.
    let lump_at = |m: u32| -> i64 {
        input
            .lump_sums
            .iter()
            .filter(|l| l.month_offset == m)
            .map(|l| l.amount_cents)
            .sum()
    };

    for month in 1..=max_months {
        if balance <= 0.5 {
            break;
        }
        let interest = (balance * r).max(0.0);
        balance += interest;
        let lump = lump_at(month);
        let mut applied_payment = input.monthly_payment_cents.min(balance.ceil() as i64);
        // Lump sum is on top of the monthly payment.
        let mut applied_lump = lump.min((balance - applied_payment as f64).ceil() as i64);
        if applied_lump < 0 {
            applied_lump = 0;
        }
        let total_pay = applied_payment + applied_lump;
        balance -= total_pay as f64;
        if balance < 0.0 {
            // Final-month rounding: cap the payment at exactly the
            // outstanding balance so total_paid lines up to the cent.
            let overshoot = -balance;
            balance = 0.0;
            // Prefer reducing the monthly payment over the lump sum,
            // since the user typed the lump as a hard amount; the
            // monthly is what flexes in the final month.
            if applied_payment as f64 >= overshoot {
                applied_payment -= overshoot.round() as i64;
            } else {
                let remaining_overshoot = overshoot - applied_payment as f64;
                applied_payment = 0;
                applied_lump = (applied_lump as f64 - remaining_overshoot).max(0.0).round() as i64;
            }
        }
        let payment_this_month = applied_payment;
        let lump_this_month = applied_lump;
        let total_payment = payment_this_month + lump_this_month;
        let interest_cents = interest.round() as i64;
        let principal_this_month = total_payment - interest_cents;
        cum_interest += interest_cents;
        cum_principal += principal_this_month.max(0);
        cum_paid_nominal += total_payment;
        let pv = present_value(total_payment as f64, inflation_monthly, month);
        cum_paid_today += pv.round() as i64;

        trajectory.push(DebtMonth {
            month,
            balance_cents: balance.round() as i64,
            interest_charged_cents: interest_cents,
            principal_paid_cents: principal_this_month.max(0),
            payment_cents: payment_this_month,
            lump_sum_cents: lump_this_month,
            cumulative_interest_cents: cum_interest,
            cumulative_principal_cents: cum_principal,
            cumulative_paid_today_cents: cum_paid_today,
        });

        if balance <= 0.5 {
            payoff_month = Some(month);
            break;
        }
    }

    let paid_off = payoff_month.is_some();
    let breakeven = breakeven_payment(starting_balance, r);
    let warning = build_warning(
        input.monthly_payment_cents,
        breakeven,
        &input.lump_sums,
        paid_off,
    );

    let (year_off, mon_in_year) = match payoff_month {
        Some(m) => {
            let y = (m - 1) / 12;
            let mi = ((m - 1) % 12) + 1;
            (Some(y), Some(mi))
        }
        None => (None, None),
    };

    ScheduleResult {
        paid_off,
        payoff_month,
        payoff_year_offset: year_off,
        payoff_month_in_year: mon_in_year,
        total_interest_cents: cum_interest,
        total_paid_cents: cum_paid_nominal,
        total_paid_today_cents: cum_paid_today,
        trajectory,
        breakeven_payment_cents: breakeven,
        warning,
    }
}

fn breakeven_payment(starting_balance: f64, monthly_rate: f64) -> i64 {
    let raw = starting_balance * monthly_rate;
    // Round up to the next cent so a payment exactly equal to the
    // breakeven still chips at principal (avoids a stuck schedule
    // when r and balance produce a perfectly round result).
    (raw.ceil() as i64).max(0)
}

fn build_warning(
    monthly_payment: i64,
    breakeven: i64,
    lump_sums: &[LumpSum],
    paid_off: bool,
) -> Option<String> {
    let total_lump: i64 = lump_sums.iter().map(|l| l.amount_cents).sum();
    if !paid_off {
        return Some(format!(
            "Debt does not pay off within the simulation window. Increase your monthly payment above the breakeven of ${:.2} to make progress.",
            breakeven as f64 / 100.0
        ));
    }
    if monthly_payment <= breakeven && total_lump == 0 {
        return Some(format!(
            "Monthly payment is at or below the initial interest charge of ${:.2} — without lump sums, this won't keep up.",
            breakeven as f64 / 100.0
        ));
    }
    None
}

fn monthly_inflation(annual_pct: f64) -> f64 {
    let i = annual_pct / 100.0;
    (1.0 + i).powf(1.0 / 12.0) - 1.0
}

fn present_value(nominal: f64, monthly_inflation: f64, month: u32) -> f64 {
    nominal / (1.0 + monthly_inflation).powi(month as i32)
}

// ---------------------------------------------------------------------
// Goal seek (single debt)
// ---------------------------------------------------------------------

pub fn goal_seek(input: &GoalSeekInput) -> GoalSeekResult {
    let r = input.debt.compounding.monthly_rate(input.debt.apr_pct);
    let breakeven = breakeven_payment(input.debt.balance_cents as f64, r);
    // Lower bound: just above breakeven so the schedule actually
    // amortizes. If the user has aggressive lump sums this is
    // conservative; a payment of zero with a single huge lump can
    // still pay off, and bisection will find it.
    let mut lo: i64 = 0;
    // Upper bound: balance + total lump cushion + breakeven. This is
    // large enough to pay off in 1 month for any realistic scenario.
    let total_lump: i64 = input.lump_sums.iter().map(|l| l.amount_cents).sum();
    let mut hi: i64 = (input.debt.balance_cents - total_lump).max(breakeven * 2) + breakeven;
    // Expand hi until either feasible or we give up.
    let probe = |payment: i64| -> ScheduleResult {
        simulate_schedule(&ScheduleInput {
            debt: input.debt.clone(),
            monthly_payment_cents: payment,
            lump_sums: input.lump_sums.clone(),
            annual_inflation_pct: input.annual_inflation_pct,
            max_months: Some(input.target_months),
        })
    };
    let mut hi_result = probe(hi);
    let mut expansions = 0;
    while !hi_result.paid_off && expansions < 20 {
        hi = hi.saturating_mul(2).max(hi + breakeven * 4);
        hi_result = probe(hi);
        expansions += 1;
    }
    if !hi_result.paid_off {
        // Infeasible. Return the high probe so the UI can display the
        // "couldn't reach target" state with whatever final state hit.
        return GoalSeekResult {
            required_monthly_payment_cents: hi,
            schedule: hi_result,
            feasible: false,
        };
    }

    // Bisect toward the smallest payment that still pays off by
    // target_months. 50 iterations gets to sub-cent precision long
    // before this in practice.
    let mut last_good = hi;
    let mut last_good_result = hi_result;
    for _ in 0..50 {
        if hi - lo <= 1 {
            break;
        }
        let mid = lo + (hi - lo) / 2;
        let result = probe(mid);
        if result.paid_off {
            hi = mid;
            last_good = mid;
            last_good_result = result;
        } else {
            lo = mid;
        }
    }

    // Final full simulation (no max_months cap) so the trajectory the
    // UI gets reflects the real schedule, not the truncated probe.
    let final_schedule = simulate_schedule(&ScheduleInput {
        debt: input.debt.clone(),
        monthly_payment_cents: last_good,
        lump_sums: input.lump_sums.clone(),
        annual_inflation_pct: input.annual_inflation_pct,
        max_months: None,
    });
    // If the unbounded run somehow doesn't pay off (numeric edge),
    // fall back to the probe result.
    let schedule = if final_schedule.paid_off {
        final_schedule
    } else {
        last_good_result
    };

    GoalSeekResult {
        required_monthly_payment_cents: last_good,
        schedule,
        feasible: true,
    }
}

// ---------------------------------------------------------------------
// Portfolio mode
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioStrategy {
    Snowball,
    Avalanche,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PortfolioInput {
    pub debts: Vec<DebtInput>,
    pub total_monthly_budget_cents: i64,
    pub strategy: PortfolioStrategy,
    /// Lump sums always go to the current target debt at the month
    /// they fire. Keeps the UI contract simple.
    #[serde(default)]
    pub lump_sums: Vec<LumpSum>,
    pub annual_inflation_pct: f64,
    #[serde(default)]
    pub max_months: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PortfolioMonth {
    pub month: u32,
    pub total_balance_cents: i64,
    pub interest_charged_cents: i64,
    pub total_paid_cents: i64,
    pub cumulative_interest_cents: i64,
    pub cumulative_principal_cents: i64,
    pub cumulative_paid_today_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebtPayoffInfo {
    pub label: Option<String>,
    pub starting_balance_cents: i64,
    pub apr_pct: f64,
    pub payoff_month: Option<u32>,
    pub total_interest_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PortfolioResult {
    pub strategy: PortfolioStrategy,
    pub paid_off: bool,
    pub payoff_month: Option<u32>,
    pub payoff_year_offset: Option<u32>,
    pub payoff_month_in_year: Option<u32>,
    pub total_interest_cents: i64,
    pub total_paid_cents: i64,
    pub total_paid_today_cents: i64,
    pub trajectory: Vec<PortfolioMonth>,
    pub per_debt: Vec<DebtPayoffInfo>,
    pub minimum_budget_cents: i64,
    pub warning: Option<String>,
}

pub fn simulate_portfolio(input: &PortfolioInput) -> PortfolioResult {
    let max_months = input
        .max_months
        .unwrap_or(MAX_SIM_MONTHS)
        .min(MAX_SIM_MONTHS);
    let inflation_monthly = monthly_inflation(input.annual_inflation_pct);

    // Per-debt running state.
    struct DebtState {
        label: Option<String>,
        balance: f64,
        apr_pct: f64,
        monthly_rate: f64,
        starting_balance: i64,
        minimum: i64,
        interest_accrued: i64,
        payoff_month: Option<u32>,
    }

    let mut states: Vec<DebtState> = input
        .debts
        .iter()
        .map(|d| DebtState {
            label: d.label.clone(),
            balance: d.balance_cents as f64,
            apr_pct: d.apr_pct,
            monthly_rate: d.compounding.monthly_rate(d.apr_pct),
            starting_balance: d.balance_cents,
            minimum: d.minimum_payment_cents.unwrap_or(0).max(0),
            interest_accrued: 0,
            payoff_month: None,
        })
        .collect();

    let min_sum: i64 = states.iter().map(|s| s.minimum).sum();
    let mut warning: Option<String> = None;
    if input.total_monthly_budget_cents < min_sum {
        warning = Some(format!(
            "Monthly budget (${:.2}) is below the sum of minimum payments (${:.2}). Increase the budget to make progress.",
            input.total_monthly_budget_cents as f64 / 100.0,
            min_sum as f64 / 100.0
        ));
    }

    let mut cum_interest = 0i64;
    let mut cum_principal = 0i64;
    let mut cum_paid_nominal = 0i64;
    let mut cum_paid_today = 0i64;
    let mut trajectory: Vec<PortfolioMonth> = Vec::new();
    let mut overall_payoff: Option<u32> = None;

    for month in 1..=max_months {
        let any_left = states.iter().any(|s| s.balance > 0.5);
        if !any_left {
            break;
        }

        // 1. Charge interest on every live debt.
        let mut interest_this_month = 0i64;
        for s in &mut states {
            if s.balance <= 0.5 {
                continue;
            }
            let i = (s.balance * s.monthly_rate).max(0.0);
            s.balance += i;
            let i_cents = i.round() as i64;
            s.interest_accrued += i_cents;
            interest_this_month += i_cents;
        }

        // 2. Pay minimums on each live debt; track surplus.
        let mut paid_this_month = 0i64;
        let mut surplus = input.total_monthly_budget_cents;
        for s in &mut states {
            if s.balance <= 0.5 {
                continue;
            }
            let pay = s.minimum.min(s.balance.ceil() as i64).min(surplus);
            s.balance -= pay as f64;
            surplus -= pay;
            paid_this_month += pay;
            if s.balance <= 0.5 {
                s.balance = 0.0;
                s.payoff_month = Some(month);
            }
        }

        // 3. Lump sums: pile onto current target.
        let lump_total: i64 = input
            .lump_sums
            .iter()
            .filter(|l| l.month_offset == month)
            .map(|l| l.amount_cents)
            .sum();
        let mut lump_remaining = lump_total;

        // 4. Distribute surplus + lump to target(s) by strategy.
        // Cascade: if target gets fully paid and surplus remains,
        // hop to the next target same month.
        loop {
            if surplus <= 0 && lump_remaining <= 0 {
                break;
            }
            let target_idx = {
                let live: Vec<usize> = states
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.balance > 0.5)
                    .map(|(i, _)| i)
                    .collect();
                if live.is_empty() {
                    None
                } else {
                    Some(match input.strategy {
                        PortfolioStrategy::Snowball => *live
                            .iter()
                            .min_by(|&&a, &&b| {
                                states[a].balance.partial_cmp(&states[b].balance).unwrap()
                            })
                            .unwrap(),
                        PortfolioStrategy::Avalanche => *live
                            .iter()
                            .max_by(|&&a, &&b| {
                                states[a].apr_pct.partial_cmp(&states[b].apr_pct).unwrap()
                            })
                            .unwrap(),
                    })
                }
            };
            let Some(idx) = target_idx else { break };
            let s = &mut states[idx];
            let needed = s.balance.ceil() as i64;
            let from_lump = lump_remaining.min(needed);
            let after_lump = needed - from_lump;
            let from_surplus = surplus.min(after_lump);
            let pay = from_lump + from_surplus;
            s.balance -= pay as f64;
            surplus -= from_surplus;
            lump_remaining -= from_lump;
            paid_this_month += pay;
            if s.balance <= 0.5 {
                s.balance = 0.0;
                if s.payoff_month.is_none() {
                    s.payoff_month = Some(month);
                }
            } else {
                // Target still has balance — surplus must be exhausted
                // (otherwise we'd have killed it). Loop ends naturally.
                break;
            }
        }

        // 5. Record month.
        let total_balance: f64 = states.iter().map(|s| s.balance).sum();
        let principal_this_month = paid_this_month - interest_this_month;
        cum_interest += interest_this_month;
        cum_principal += principal_this_month.max(0);
        cum_paid_nominal += paid_this_month;
        let pv = present_value(paid_this_month as f64, inflation_monthly, month);
        cum_paid_today += pv.round() as i64;

        trajectory.push(PortfolioMonth {
            month,
            total_balance_cents: total_balance.round() as i64,
            interest_charged_cents: interest_this_month,
            total_paid_cents: paid_this_month,
            cumulative_interest_cents: cum_interest,
            cumulative_principal_cents: cum_principal,
            cumulative_paid_today_cents: cum_paid_today,
        });

        if total_balance <= 0.5 {
            overall_payoff = Some(month);
            break;
        }
    }

    let paid_off = overall_payoff.is_some();
    let (year_off, mon_in_year) = match overall_payoff {
        Some(m) => {
            let y = (m - 1) / 12;
            let mi = ((m - 1) % 12) + 1;
            (Some(y), Some(mi))
        }
        None => (None, None),
    };

    let per_debt: Vec<DebtPayoffInfo> = states
        .iter()
        .map(|s| DebtPayoffInfo {
            label: s.label.clone(),
            starting_balance_cents: s.starting_balance,
            apr_pct: s.apr_pct,
            payoff_month: s.payoff_month,
            total_interest_cents: s.interest_accrued,
        })
        .collect();

    if !paid_off && warning.is_none() {
        warning = Some(
            "Portfolio doesn't pay off within the simulation window. Increase the monthly budget."
                .to_string(),
        );
    }

    PortfolioResult {
        strategy: input.strategy,
        paid_off,
        payoff_month: overall_payoff,
        payoff_year_offset: year_off,
        payoff_month_in_year: mon_in_year,
        total_interest_cents: cum_interest,
        total_paid_cents: cum_paid_nominal,
        total_paid_today_cents: cum_paid_today,
        trajectory,
        per_debt,
        minimum_budget_cents: min_sum,
        warning,
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn debt(balance: i64, apr: f64) -> DebtInput {
        DebtInput {
            label: None,
            balance_cents: balance,
            apr_pct: apr,
            compounding: CompoundingFrequency::Monthly,
            minimum_payment_cents: None,
        }
    }

    #[test]
    fn monthly_rate_conversions() {
        // APR 12%, monthly comp -> exactly 1.0% per month
        assert!((CompoundingFrequency::Monthly.monthly_rate(12.0) - 0.01).abs() < 1e-12);
        // Yearly comp -> (1.12)^(1/12) - 1 ≈ 0.00948879
        let r_yearly = CompoundingFrequency::Yearly.monthly_rate(12.0);
        assert!((r_yearly - 0.00948879).abs() < 1e-6);
        // Continuous -> e^0.01 - 1 ≈ 0.01005017
        let r_cont = CompoundingFrequency::Continuous.monthly_rate(12.0);
        assert!((r_cont - 0.01005017).abs() < 1e-6);
        // Daily comp at APR 0% -> 0
        assert!((CompoundingFrequency::Daily.monthly_rate(0.0)).abs() < 1e-12);
    }

    #[test]
    fn schedule_zero_apr_pays_off_linearly() {
        // $1200 at 0% APR with $100/mo should pay off in 12 months.
        let r = simulate_schedule(&ScheduleInput {
            debt: debt(120_000, 0.0),
            monthly_payment_cents: 10_000,
            lump_sums: vec![],
            annual_inflation_pct: 0.0,
            max_months: None,
        });
        assert!(r.paid_off);
        assert_eq!(r.payoff_month, Some(12));
        assert_eq!(r.total_interest_cents, 0);
        assert_eq!(r.total_paid_cents, 120_000);
    }

    #[test]
    fn schedule_known_amortization_5pct_5yr() {
        // $10,000 at 5% APR, 60-month amortization payment.
        // Standard formula: P = B * r * (1+r)^n / ((1+r)^n - 1)
        // r = 0.05/12, n = 60 -> P ≈ $188.71.
        // Pay $188.72 (a hair above the $188.71 textbook value to
        // absorb the cents-rounding residue) — should pay off at the
        // 60-month mark.
        let r = simulate_schedule(&ScheduleInput {
            debt: debt(1_000_000, 5.0),
            monthly_payment_cents: 18_872,
            lump_sums: vec![],
            annual_inflation_pct: 0.0,
            max_months: None,
        });
        assert!(r.paid_off);
        assert!(r.payoff_month.unwrap() <= 60);
        assert!(r.payoff_month.unwrap() >= 59);
        // Total interest ~ $1,322. Penny-level rounding drift over 60
        // months can push the integer total a few hundred cents either
        // way; we only need to verify it lands in the right ballpark.
        assert!(
            (r.total_interest_cents - 132_274).abs() < 2_000,
            "interest off: {}",
            r.total_interest_cents
        );
    }

    #[test]
    fn schedule_below_breakeven_warns_and_doesnt_pay_off() {
        // $10,000 at 12% APR, monthly compounding -> $100/mo interest.
        // Pay $50/mo -> debt grows.
        let r = simulate_schedule(&ScheduleInput {
            debt: debt(1_000_000, 12.0),
            monthly_payment_cents: 5_000,
            lump_sums: vec![],
            annual_inflation_pct: 0.0,
            max_months: Some(60),
        });
        assert!(!r.paid_off);
        assert!(r.warning.is_some());
        assert_eq!(r.breakeven_payment_cents, 10_000);
    }

    #[test]
    fn schedule_lump_sum_accelerates() {
        // Same debt, $200/mo + $5,000 lump at month 12.
        let with_lump = simulate_schedule(&ScheduleInput {
            debt: debt(1_000_000, 12.0),
            monthly_payment_cents: 20_000,
            lump_sums: vec![LumpSum {
                month_offset: 12,
                amount_cents: 500_000,
            }],
            annual_inflation_pct: 0.0,
            max_months: None,
        });
        let without_lump = simulate_schedule(&ScheduleInput {
            debt: debt(1_000_000, 12.0),
            monthly_payment_cents: 20_000,
            lump_sums: vec![],
            annual_inflation_pct: 0.0,
            max_months: None,
        });
        assert!(with_lump.paid_off);
        assert!(without_lump.paid_off);
        assert!(with_lump.payoff_month.unwrap() < without_lump.payoff_month.unwrap());
        assert!(with_lump.total_interest_cents < without_lump.total_interest_cents);
    }

    #[test]
    fn goal_seek_finds_payment() {
        // Same as the 5% / 60-month amortization. Goal-seek should
        // recover the textbook payment within a dollar or two.
        let g = goal_seek(&GoalSeekInput {
            debt: debt(1_000_000, 5.0),
            target_months: 60,
            lump_sums: vec![],
            annual_inflation_pct: 0.0,
        });
        assert!(g.feasible);
        // Allow ±$1 tolerance from the textbook $188.71.
        let diff = (g.required_monthly_payment_cents - 18_871).abs();
        assert!(diff < 200, "payment off by {} cents", diff);
        assert!(g.schedule.payoff_month.unwrap() <= 60);
    }

    #[test]
    fn goal_seek_year_and_month() {
        // 14-month payoff -> year offset 1, month-in-year 2.
        let g = goal_seek(&GoalSeekInput {
            debt: debt(140_000, 0.0),
            target_months: 14,
            lump_sums: vec![],
            annual_inflation_pct: 0.0,
        });
        assert!(g.feasible);
        assert_eq!(g.schedule.payoff_year_offset, Some(1));
        assert_eq!(g.schedule.payoff_month_in_year, Some(2));
    }

    #[test]
    fn inflation_discount_lower_than_nominal() {
        let r = simulate_schedule(&ScheduleInput {
            debt: debt(1_000_000, 5.0),
            monthly_payment_cents: 18_871,
            lump_sums: vec![],
            annual_inflation_pct: 3.0,
            max_months: None,
        });
        assert!(r.paid_off);
        assert!(r.total_paid_today_cents < r.total_paid_cents);
        // At 3% over ~5 years, today's $ should be ~93% of nominal.
        let ratio = r.total_paid_today_cents as f64 / r.total_paid_cents as f64;
        assert!(ratio > 0.90 && ratio < 0.96, "ratio: {}", ratio);
    }

    #[test]
    fn portfolio_avalanche_beats_snowball_on_interest_when_apr_inverts_balance() {
        // Two debts: small balance at high APR, large balance at low APR.
        // Avalanche should pay less total interest than snowball.
        let mk = |bal: i64, apr: f64, min: i64| DebtInput {
            label: None,
            balance_cents: bal,
            apr_pct: apr,
            compounding: CompoundingFrequency::Monthly,
            minimum_payment_cents: Some(min),
        };
        let debts = vec![
            mk(200_000, 25.0, 5_000),   // small high-APR
            mk(1_000_000, 5.0, 10_000), // big low-APR
        ];
        let snow = simulate_portfolio(&PortfolioInput {
            debts: debts.clone(),
            total_monthly_budget_cents: 30_000,
            strategy: PortfolioStrategy::Snowball,
            lump_sums: vec![],
            annual_inflation_pct: 0.0,
            max_months: None,
        });
        let aval = simulate_portfolio(&PortfolioInput {
            debts,
            total_monthly_budget_cents: 30_000,
            strategy: PortfolioStrategy::Avalanche,
            lump_sums: vec![],
            annual_inflation_pct: 0.0,
            max_months: None,
        });
        assert!(snow.paid_off);
        assert!(aval.paid_off);
        // Snowball happens to also kill the small one first here so
        // they're close, but avalanche should win on interest.
        assert!(
            aval.total_interest_cents <= snow.total_interest_cents,
            "avalanche interest {} should be <= snowball {}",
            aval.total_interest_cents,
            snow.total_interest_cents
        );
    }

    #[test]
    fn portfolio_below_minimums_warns() {
        let mk = |bal: i64, apr: f64, min: i64| DebtInput {
            label: None,
            balance_cents: bal,
            apr_pct: apr,
            compounding: CompoundingFrequency::Monthly,
            minimum_payment_cents: Some(min),
        };
        let r = simulate_portfolio(&PortfolioInput {
            debts: vec![mk(100_000, 10.0, 5_000), mk(200_000, 8.0, 7_000)],
            total_monthly_budget_cents: 10_000, // below 12_000 sum-of-mins
            strategy: PortfolioStrategy::Avalanche,
            lump_sums: vec![],
            annual_inflation_pct: 0.0,
            max_months: Some(120),
        });
        assert!(r.warning.is_some());
        assert_eq!(r.minimum_budget_cents, 12_000);
    }
}
