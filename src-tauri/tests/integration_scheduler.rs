//! Integration tests for the scheduler tick / dispatch path.
//!
//! Most of the queue plumbing is unit-tested in `scheduler::tests`. This
//! file covers the cross-module concerns: how `tick()` interacts with
//! stale jobs, how it leaves state alone when the placeholder handlers
//! return `Retry`, and that the singleton helper works against a real
//! migrated DB.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use moneypenny_lib::db;
use moneypenny_lib::domain::recurring::{Frequency, NewRecurringRule, RecurringMode};
use moneypenny_lib::llm::{ChatRequest, ChatResponse, LLMProvider};
use moneypenny_lib::repository::recurring_rules;
use moneypenny_lib::scheduler::{enqueue, ensure_singleton, list_due, tick, JobKind};
use moneypenny_lib::telegram::client::{Chat, Message, TelegramApi, Update, User};
use moneypenny_lib::telegram::router::RouterDeps;
use moneypenny_lib::telegram::state::BotState;
use rusqlite::Connection;
use time::macros::datetime;

#[derive(Default)]
struct NoopTelegram {
    sent: Mutex<Vec<(i64, String)>>,
}

impl NoopTelegram {
    fn sent_to(&self, chat_id: i64) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(c, _)| *c == chat_id)
            .map(|(_, t)| t.clone())
            .collect()
    }
}

#[async_trait]
impl TelegramApi for NoopTelegram {
    async fn get_me(&self) -> Result<User> {
        Ok(User {
            id: 1,
            is_bot: true,
            first_name: "stub".into(),
            username: None,
        })
    }
    async fn get_updates(&self, _offset: i64, _timeout: u32) -> Result<Vec<Update>> {
        Ok(vec![])
    }
    async fn send_message(&self, chat_id: i64, text: &str) -> Result<Message> {
        self.sent.lock().unwrap().push((chat_id, text.to_string()));
        Ok(Message {
            message_id: 0,
            date: 0,
            chat: Chat {
                id: chat_id,
                kind: "private".into(),
                username: None,
                first_name: None,
                last_name: None,
                title: None,
            },
            from: None,
            text: Some(text.to_string()),
        })
    }
    async fn delete_webhook(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct NoopLlm;

#[async_trait]
impl LLMProvider for NoopLlm {
    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        anyhow::bail!("NoopLlm: no canned response — handlers must not call the LLM in this test")
    }
}

fn fresh_deps() -> (RouterDeps, Arc<Mutex<Connection>>, Arc<NoopTelegram>) {
    let conn = db::open_in_memory().unwrap();
    db::migrate(&conn).unwrap();
    // Reactivate all seed categories so handler tests can use them.
    conn.execute("UPDATE categories SET is_active = 1 WHERE is_seed = 1", [])
        .unwrap();
    let conn = Arc::new(Mutex::new(conn));
    let tg = Arc::new(NoopTelegram::default());
    let deps = RouterDeps {
        conn: conn.clone(),
        llm: Arc::new(NoopLlm),
        client: tg.clone(),
        state: Arc::new(BotState::new()),
        default_currency: "USD".into(),
    };
    (deps, conn, tg)
}

#[tokio::test]
async fn tick_with_empty_queue_is_a_noop() {
    let (deps, _, _) = fresh_deps();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let fired = tick(&deps, now).await.unwrap();
    assert_eq!(fired, 0);
}

#[tokio::test]
async fn tick_skips_stale_jobs_and_advances_them() {
    // A job 30 days overdue exceeds MAX_STALE_DAYS=7; tick should bump
    // its next_due_at forward by MAX_STALE_DAYS without firing it.
    let (deps, conn, _) = fresh_deps();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let stale_due = datetime!(2026-03-15 12:00:00 UTC);
    let id = {
        let c = conn.lock().unwrap();
        enqueue(&c, JobKind::WeeklySummary, "{}", stale_due).unwrap()
    };

    let fired = tick(&deps, now).await.unwrap();
    assert_eq!(fired, 0, "stale job is skipped, not counted as fired");

    let new_due: time::OffsetDateTime = {
        let c = conn.lock().unwrap();
        c.query_row(
            "SELECT next_due_at FROM scheduled_jobs WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(
        new_due > now,
        "stale job should have its next_due_at pushed beyond now"
    );
    let last_fired: Option<time::OffsetDateTime> = {
        let c = conn.lock().unwrap();
        c.query_row(
            "SELECT last_fired_at FROM scheduled_jobs WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(last_fired.is_none(), "stale job did not actually fire");
}

#[tokio::test]
async fn budget_alert_fires_at_80_pct_then_not_again_same_month() {
    let (deps, conn, tg) = fresh_deps();
    pair_owner(&conn, 111, "Wyatt");
    let cid = category_id_by_name(&conn, "Dining Out");
    // $100 monthly target.
    {
        let c = conn.lock().unwrap();
        c.execute(
            "UPDATE categories SET monthly_target_cents = 10000 WHERE id = ?1",
            [cid],
        )
        .unwrap();
        // Spend $85 (just past 80%, under 100%).
        c.execute(
            "INSERT INTO expenses (amount_cents, currency, category_id, occurred_at, source) \
             VALUES (8500, 'USD', ?1, '2026-04-10T12:00:00Z', 'manual')",
            [cid],
        )
        .unwrap();
    }
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let due = datetime!(2026-04-15 11:00:00 UTC);
    {
        let c = conn.lock().unwrap();
        enqueue(&c, JobKind::BudgetAlertSweep, "{}", due).unwrap();
    }

    // First tick fires the 80% alert.
    tick(&deps, now).await.unwrap();
    let sent_after_first = tg.sent_to(111);
    assert_eq!(sent_after_first.len(), 1);
    assert!(sent_after_first[0].contains("80%"));
    assert!(sent_after_first[0].contains("Dining Out"));

    // Bump the queue's next_due_at backward so it fires again on the next
    // tick (real scheduler would wait an hour).
    {
        let c = conn.lock().unwrap();
        c.execute(
            "UPDATE scheduled_jobs SET next_due_at = ?1 WHERE kind = 'budget_alert_sweep'",
            rusqlite::params![datetime!(2026-04-15 11:30:00 UTC)],
        )
        .unwrap();
    }
    tick(&deps, datetime!(2026-04-15 12:30:00 UTC))
        .await
        .unwrap();
    // No new alert — the threshold was already recorded for this month.
    assert_eq!(
        tg.sent_to(111).len(),
        1,
        "80% threshold already alerted this month — must not fire again"
    );
}

#[tokio::test]
async fn budget_alert_disabled_setting_short_circuits() {
    let (deps, conn, tg) = fresh_deps();
    pair_owner(&conn, 111, "Wyatt");
    let cid = category_id_by_name(&conn, "Dining Out");
    {
        let c = conn.lock().unwrap();
        c.execute(
            "UPDATE categories SET monthly_target_cents = 10000 WHERE id = ?1",
            [cid],
        )
        .unwrap();
        c.execute(
            "INSERT INTO expenses (amount_cents, currency, category_id, occurred_at, source) \
             VALUES (15000, 'USD', ?1, '2026-04-10T12:00:00Z', 'manual')",
            [cid],
        )
        .unwrap();
        // Disable budget alerts.
        moneypenny_lib::repository::settings::set(
            &c,
            moneypenny_lib::repository::settings::keys::BUDGET_ALERTS_ENABLED,
            "0",
        )
        .unwrap();
        enqueue(
            &c,
            JobKind::BudgetAlertSweep,
            "{}",
            datetime!(2026-04-15 11:00:00 UTC),
        )
        .unwrap();
    }
    let now = datetime!(2026-04-15 12:00:00 UTC);
    tick(&deps, now).await.unwrap();
    assert!(
        tg.sent_to(111).is_empty(),
        "disabled setting must skip alerts entirely"
    );
}

#[tokio::test]
async fn weekly_summary_no_owner_just_advances() {
    // No paired owner — weekly summary can't DM anyone, so it just
    // slips its schedule forward by 7 days and tries again next week.
    let (deps, conn, tg) = fresh_deps();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let due = datetime!(2026-04-15 11:00:00 UTC);
    let id = {
        let c = conn.lock().unwrap();
        enqueue(&c, JobKind::WeeklySummary, "{}", due).unwrap()
    };

    tick(&deps, now).await.unwrap();

    let next: time::OffsetDateTime = {
        let c = conn.lock().unwrap();
        c.query_row(
            "SELECT next_due_at FROM scheduled_jobs WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(
        next > now,
        "weekly summary should reschedule into the future even without owner"
    );
    assert!(tg.sent_to(0).is_empty()); // no chat_id to send to anyway
}

#[tokio::test]
async fn weekly_summary_with_owner_dms_owner() {
    let (deps, conn, tg) = fresh_deps();
    pair_owner(&conn, 111, "Wyatt");
    // Insert one expense in the last 7 days.
    {
        let c = conn.lock().unwrap();
        let cid = c
            .query_row(
                "SELECT id FROM categories WHERE name = 'Groceries'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
        c.execute(
            "INSERT INTO expenses (amount_cents, currency, category_id, occurred_at, source) \
             VALUES (5000, 'USD', ?1, '2026-04-13T12:00:00Z', 'manual')",
            [cid],
        )
        .unwrap();
    }
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let due = datetime!(2026-04-15 11:00:00 UTC);
    {
        let c = conn.lock().unwrap();
        enqueue(&c, JobKind::WeeklySummary, "{}", due).unwrap();
    }

    tick(&deps, now).await.unwrap();

    let sent = tg.sent_to(111);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("Last 7 days"));
    assert!(sent[0].contains("Groceries"));
}

// ---- recurring expense handler ----

fn category_id_by_name(conn: &Mutex<Connection>, name: &str) -> i64 {
    let c = conn.lock().unwrap();
    c.query_row("SELECT id FROM categories WHERE name = ?1", [name], |r| {
        r.get(0)
    })
    .unwrap()
}

fn pair_owner(conn: &Mutex<Connection>, chat_id: i64, name: &str) {
    let c = conn.lock().unwrap();
    c.execute(
        "INSERT INTO telegram_authorized_chats (chat_id, display_name, role) VALUES (?1, ?2, 'owner')",
        rusqlite::params![chat_id, name],
    )
    .unwrap();
}

#[tokio::test]
async fn auto_mode_recurring_inserts_expense_and_reschedules() {
    let (deps, conn, _tg) = fresh_deps();
    let cid = category_id_by_name(&conn, "Entertainment");
    let now = datetime!(2026-04-15 12:00:00 UTC);

    let (rule_id, job_id) = {
        let c = conn.lock().unwrap();
        let rid = recurring_rules::insert(
            &c,
            &NewRecurringRule {
                label: "Spotify".into(),
                amount_cents: 999,
                currency: "USD".into(),
                category_id: cid,
                frequency: Frequency::Monthly,
                anchor_day: 15,
                mode: RecurringMode::Auto,
            },
        )
        .unwrap();
        let jid = enqueue(
            &c,
            JobKind::RecurringExpense,
            &serde_json::json!({ "rule_id": rid }).to_string(),
            datetime!(2026-04-15 11:00:00 UTC),
        )
        .unwrap();
        (rid, jid)
    };

    let fired = tick(&deps, now).await.unwrap();
    assert_eq!(fired, 1);

    // Expense was inserted.
    let count: i64 = {
        let c = conn.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM expenses WHERE category_id = ?1",
            [cid],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(count, 1, "auto rule must insert an expense on fire");

    // Job rescheduled to next month (May 15).
    let next: time::OffsetDateTime = {
        let c = conn.lock().unwrap();
        c.query_row(
            "SELECT next_due_at FROM scheduled_jobs WHERE id = ?1",
            [job_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(next, datetime!(2026-05-15 00:00:00 UTC));
    let _ = rule_id;
}

#[tokio::test]
async fn confirm_mode_recurring_dms_owner_and_records_pending() {
    let (deps, conn, tg) = fresh_deps();
    pair_owner(&conn, 111, "Wyatt");
    let cid = category_id_by_name(&conn, "Entertainment");
    let now = datetime!(2026-04-15 12:00:00 UTC);

    let _rule_id = {
        let c = conn.lock().unwrap();
        let rid = recurring_rules::insert(
            &c,
            &NewRecurringRule {
                label: "Netflix".into(),
                amount_cents: 1_549,
                currency: "USD".into(),
                category_id: cid,
                frequency: Frequency::Monthly,
                anchor_day: 15,
                mode: RecurringMode::Confirm,
            },
        )
        .unwrap();
        enqueue(
            &c,
            JobKind::RecurringExpense,
            &serde_json::json!({ "rule_id": rid }).to_string(),
            datetime!(2026-04-15 11:00:00 UTC),
        )
        .unwrap();
        rid
    };

    tick(&deps, now).await.unwrap();

    // No expense yet — only the DM.
    let count: i64 = {
        let c = conn.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM expenses WHERE category_id = ?1",
            [cid],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(count, 0, "confirm rule must NOT insert until user replies");

    // Owner received a DM.
    let sent = tg.sent_to(111);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("Netflix"));
    assert!(sent[0].contains("yes"));

    // Pending row exists.
    let pending = {
        let c = conn.lock().unwrap();
        recurring_rules::get_pending(&c, 111).unwrap()
    };
    assert!(pending.is_some(), "pending confirmation must be persisted");
}

#[tokio::test]
async fn confirm_mode_defers_when_chat_already_has_pending() {
    let (deps, conn, _tg) = fresh_deps();
    pair_owner(&conn, 111, "Wyatt");
    let cid = category_id_by_name(&conn, "Entertainment");
    let now = datetime!(2026-04-15 12:00:00 UTC);

    // Two rules due simultaneously for the same chat.
    let (job1, job2) = {
        let c = conn.lock().unwrap();
        let r1 = recurring_rules::insert(
            &c,
            &NewRecurringRule {
                label: "Netflix".into(),
                amount_cents: 1_549,
                currency: "USD".into(),
                category_id: cid,
                frequency: Frequency::Monthly,
                anchor_day: 15,
                mode: RecurringMode::Confirm,
            },
        )
        .unwrap();
        let r2 = recurring_rules::insert(
            &c,
            &NewRecurringRule {
                label: "Hulu".into(),
                amount_cents: 1_299,
                currency: "USD".into(),
                category_id: cid,
                frequency: Frequency::Monthly,
                anchor_day: 15,
                mode: RecurringMode::Confirm,
            },
        )
        .unwrap();
        let j1 = enqueue(
            &c,
            JobKind::RecurringExpense,
            &serde_json::json!({ "rule_id": r1 }).to_string(),
            datetime!(2026-04-15 11:00:00 UTC),
        )
        .unwrap();
        let j2 = enqueue(
            &c,
            JobKind::RecurringExpense,
            &serde_json::json!({ "rule_id": r2 }).to_string(),
            datetime!(2026-04-15 11:00:00 UTC),
        )
        .unwrap();
        (j1, j2)
    };

    tick(&deps, now).await.unwrap();

    // Exactly one job should have rescheduled (the first one); the
    // second returns Retry and stays at its original next_due_at.
    let next1: time::OffsetDateTime = {
        let c = conn.lock().unwrap();
        c.query_row(
            "SELECT next_due_at FROM scheduled_jobs WHERE id = ?1",
            [job1],
            |r| r.get(0),
        )
        .unwrap()
    };
    let next2: time::OffsetDateTime = {
        let c = conn.lock().unwrap();
        c.query_row(
            "SELECT next_due_at FROM scheduled_jobs WHERE id = ?1",
            [job2],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_ne!(next1, next2, "one of the jobs must have been deferred");
}

#[tokio::test]
async fn paused_recurring_rule_just_advances_no_dm_no_insert() {
    let (deps, conn, tg) = fresh_deps();
    pair_owner(&conn, 111, "Wyatt");
    let cid = category_id_by_name(&conn, "Entertainment");
    let now = datetime!(2026-04-15 12:00:00 UTC);

    {
        let c = conn.lock().unwrap();
        let rid = recurring_rules::insert(
            &c,
            &NewRecurringRule {
                label: "Gym".into(),
                amount_cents: 3_000,
                currency: "USD".into(),
                category_id: cid,
                frequency: Frequency::Monthly,
                anchor_day: 15,
                mode: RecurringMode::Confirm,
            },
        )
        .unwrap();
        recurring_rules::set_enabled(&c, rid, false).unwrap();
        enqueue(
            &c,
            JobKind::RecurringExpense,
            &serde_json::json!({ "rule_id": rid }).to_string(),
            datetime!(2026-04-15 11:00:00 UTC),
        )
        .unwrap();
    };

    tick(&deps, now).await.unwrap();

    let exp_count: i64 = {
        let c = conn.lock().unwrap();
        c.query_row("SELECT COUNT(*) FROM expenses", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(exp_count, 0, "paused rule must not insert");
    assert!(tg.sent_to(111).is_empty(), "paused rule must not DM");
}

#[tokio::test]
async fn missing_rule_disables_orphan_job() {
    let (deps, conn, _tg) = fresh_deps();
    let now = datetime!(2026-04-15 12:00:00 UTC);

    let job_id = {
        let c = conn.lock().unwrap();
        enqueue(
            &c,
            JobKind::RecurringExpense,
            r#"{"rule_id": 99999}"#,
            datetime!(2026-04-15 11:00:00 UTC),
        )
        .unwrap()
    };

    tick(&deps, now).await.unwrap();
    let enabled: i64 = {
        let c = conn.lock().unwrap();
        c.query_row(
            "SELECT enabled FROM scheduled_jobs WHERE id = ?1",
            [job_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        enabled, 0,
        "orphan job referring to deleted rule must be disabled"
    );
}

#[tokio::test]
async fn ensure_singleton_idempotent_across_simulated_relaunches() {
    // Two consecutive tick cycles in the same process simulate the user
    // launching the app twice. The singleton row count must stay at 1.
    let (_deps, conn, _) = fresh_deps();
    let due = datetime!(2026-04-15 12:00:00 UTC);
    {
        let c = conn.lock().unwrap();
        ensure_singleton(&c, JobKind::WeeklySummary, due).unwrap();
        ensure_singleton(&c, JobKind::WeeklySummary, due).unwrap();
    }
    let count: i64 = {
        let c = conn.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM scheduled_jobs WHERE kind = 'weekly_summary'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(count, 1);

    // And `list_due` returns it once.
    let due_rows = {
        let c = conn.lock().unwrap();
        list_due(&c, due).unwrap()
    };
    assert_eq!(due_rows.len(), 1);
}
