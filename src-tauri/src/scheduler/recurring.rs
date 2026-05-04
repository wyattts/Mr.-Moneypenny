//! Recurring-expense job handler.
//!
//! Each row in `recurring_rules` has a corresponding row in
//! `scheduled_jobs` of kind `recurring_expense` with payload
//! `{"rule_id": <id>}`. When the job fires, this handler:
//!
//!   1. Loads the rule. Missing → `Done` (rule was deleted; the job
//!      stays in the queue but is now an orphan).
//!   2. If the rule is disabled → reschedule for the next due date so
//!      the job stays in sync with the spec.
//!   3. In `auto` mode: insert the expense immediately, reschedule.
//!   4. In `confirm` mode: identify the household owner; if their chat
//!      already has a pending confirmation outstanding, return `Retry`
//!      so we wait until it's resolved before stacking a second one.
//!      Otherwise DM "label $X.YY today — yes / no / skip", insert the
//!      pending row, and reschedule for the *next* occurrence (the
//!      reply path inserts the expense for *this* occurrence
//!      independently).

use anyhow::Result;
use rusqlite::params;
use time::{Duration, OffsetDateTime};

use crate::domain::recurring::{self as rec, RecurringMode};
use crate::domain::{ExpenseSource, NewExpense};
use crate::repository::{expenses, recurring_rules};
use crate::telegram::router::RouterDeps;

use super::{Job, JobOutcome};

/// How long a confirmation DM stays valid before timing out (and being
/// treated as a "skip" on the next router-side cleanup).
const CONFIRM_TTL_HOURS: i64 = 36;

/// Format a money amount as e.g. "$15.49" / "€2.50". Mirrors
/// `telegram::formatter::format_money` for currencies it knows; otherwise
/// falls back to `15.49 USD`.
fn money(amount_cents: i64, currency: &str) -> String {
    crate::telegram::formatter::format_money(amount_cents, currency)
}

pub async fn handle(deps: &RouterDeps, job: &Job, now: OffsetDateTime) -> Result<JobOutcome> {
    // Parse the payload to find the rule_id.
    let parsed: serde_json::Value =
        serde_json::from_str(&job.payload).unwrap_or(serde_json::json!({}));
    let rule_id = match parsed.get("rule_id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => {
            tracing::warn!(
                target: "scheduler::recurring",
                job_id = job.id,
                payload = %job.payload,
                "recurring_expense job missing rule_id payload — disabling"
            );
            return Ok(JobOutcome::Done);
        }
    };

    let rule = {
        let conn = deps.conn.lock().unwrap();
        recurring_rules::get(&conn, rule_id)?
    };

    let Some(rule) = rule else {
        // Rule was deleted; the FK ON DELETE CASCADE on scheduled_jobs
        // should have removed this job, but if it slipped through for
        // any reason, retire it now.
        return Ok(JobOutcome::Done);
    };

    if !rule.enabled {
        // Paused — keep the job alive, just advance to the next due date
        // so we don't churn every tick.
        let next = rec::next_due(rule.frequency, rule.anchor_day, now);
        return Ok(JobOutcome::Reschedule(next));
    }

    // Stamp the expense at the rule's *intended* due time, not now.
    // After a multi-day offline period, catch-up may fire several due
    // jobs in a single tick; with `occurred_at = now` they all
    // collapse to one timestamp, polluting the spend-by-day chart.
    // `job.next_due_at` is the moment the scheduler thought this
    // occurrence should fire — the right timestamp historically.
    let occurred_at = job.next_due_at;
    let new_expense = NewExpense {
        amount_cents: rule.amount_cents,
        currency: rule.currency.clone(),
        category_id: Some(rule.category_id),
        description: Some(rule.label.clone()),
        occurred_at,
        source: ExpenseSource::Telegram,
        raw_message: Some(format!("recurring rule #{}", rule.id)),
        llm_confidence: None,
        logged_by_chat_id: None,
        is_refund: false,
        refund_for_expense_id: None,
    };

    match rule.mode {
        RecurringMode::Auto => {
            {
                let conn = deps.conn.lock().unwrap();
                expenses::insert(&conn, &new_expense)?;
            }
            tracing::info!(
                target: "scheduler::recurring",
                rule_id = rule.id,
                label = %rule.label,
                "auto-logged recurring expense"
            );
        }
        RecurringMode::Confirm => {
            // Find the household owner's chat_id. (If there's no owner,
            // we can't ask anyone — disable the job until pairing happens.)
            let owner: Option<i64> = {
                let conn = deps.conn.lock().unwrap();
                conn.query_row(
                    "SELECT chat_id FROM telegram_authorized_chats \
                     WHERE role = 'owner' ORDER BY chat_id LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .ok()
            };
            let Some(chat_id) = owner else {
                // No owner paired — leave for retry; once the user
                // pairs, the next tick will see them.
                return Ok(JobOutcome::Retry);
            };

            // Don't stack confirmations on the same chat.
            let already_pending = {
                let conn = deps.conn.lock().unwrap();
                recurring_rules::chat_has_pending(&conn, chat_id)?
            };
            if already_pending {
                return Ok(JobOutcome::Retry);
            }

            let amount_str = money(rule.amount_cents, &rule.currency);
            let prompt = format!(
                "Recurring: {label} {amount} today — reply *yes* to log, *no* to skip this time, or *skip* (alias of no).",
                label = rule.label,
                amount = amount_str,
            );

            // Send the DM.
            if let Err(e) = deps.client.send_message(chat_id, &prompt).await {
                tracing::warn!(
                    target: "scheduler::recurring",
                    rule_id = rule.id,
                    error = %e,
                    "failed to send confirmation DM — will retry next tick"
                );
                return Ok(JobOutcome::Retry);
            }

            // Insert the pending row.
            let expires_at = now + Duration::hours(CONFIRM_TTL_HOURS);
            {
                let conn = deps.conn.lock().unwrap();
                recurring_rules::insert_pending(&conn, chat_id, rule.id, now, expires_at)?;
                // Stash the not-yet-inserted expense's `occurred_at` and
                // amount on the pending row implicitly via the rule_id;
                // the router resolves it from the rule when the user
                // replies, so we don't store anything else here.
                let _ = params![]; // (no-op — keep imports tidy)
            }
            tracing::info!(
                target: "scheduler::recurring",
                rule_id = rule.id,
                chat_id,
                "asked for recurring expense confirmation"
            );
        }
    }

    let next = rec::next_due(rule.frequency, rule.anchor_day, now);
    Ok(JobOutcome::Reschedule(next))
}
