//! Integration tests for the Telegram router and the agentic LLM loop.
//!
//! Uses stub implementations of `TelegramApi` and `LLMProvider` so no
//! network is required.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use moneypenny_lib::db;
use moneypenny_lib::llm::{
    ChatRequest, ChatResponse, ContentBlock, LLMProvider, StopReason, Usage,
};
use moneypenny_lib::telegram::auth::{generate_pairing_code, is_authorized, Role};
use moneypenny_lib::telegram::client::{Chat, Message, TelegramApi, Update, User};
use moneypenny_lib::telegram::router::{handle_update, RouterDeps};
use moneypenny_lib::telegram::state::BotState;
use rusqlite::Connection;
use serde_json::json;
use time::macros::datetime;
use time::OffsetDateTime;

// ---------------------------------------------------------------------
// Stubs
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
struct SentMessage {
    chat_id: i64,
    text: String,
}

#[derive(Default)]
struct StubTelegram {
    sent: Mutex<Vec<SentMessage>>,
}

impl StubTelegram {
    fn sent_to(&self, chat_id: i64) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.chat_id == chat_id)
            .map(|m| m.text.clone())
            .collect()
    }
}

#[async_trait]
impl TelegramApi for StubTelegram {
    async fn get_me(&self) -> Result<User> {
        Ok(User {
            id: 1,
            is_bot: true,
            first_name: "stub".into(),
            username: Some("stub_bot".into()),
        })
    }
    async fn get_updates(&self, _offset: i64, _timeout: u32) -> Result<Vec<Update>> {
        Ok(vec![])
    }
    async fn send_message(&self, chat_id: i64, text: &str) -> Result<Message> {
        self.sent.lock().unwrap().push(SentMessage {
            chat_id,
            text: text.to_string(),
        });
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
struct StubLlm {
    /// Canned responses, popped from the front in order.
    responses: Mutex<Vec<ChatResponse>>,
    /// Captured requests for inspection.
    requests: Mutex<Vec<ChatRequest>>,
}

impl StubLlm {
    fn enqueue(&self, response: ChatResponse) {
        self.responses.lock().unwrap().push(response);
    }
}

#[async_trait]
impl LLMProvider for StubLlm {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.requests.lock().unwrap().push(request);
        let mut q = self.responses.lock().unwrap();
        if q.is_empty() {
            anyhow::bail!("StubLlm: response queue empty");
        }
        Ok(q.remove(0))
    }
    fn provider_name(&self) -> &str {
        "stub"
    }
    fn model(&self) -> &str {
        "stub-model"
    }
}

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        stop_reason: StopReason::EndTurn,
        content: vec![ContentBlock::Text(text.to_string())],
        usage: Usage::default(),
    }
}

fn tool_use_response(name: &str, input: serde_json::Value) -> ChatResponse {
    ChatResponse {
        stop_reason: StopReason::ToolUse,
        content: vec![ContentBlock::ToolUse {
            id: "tu_test".into(),
            name: name.to_string(),
            input,
        }],
        usage: Usage::default(),
    }
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn fresh() -> Connection {
    let c = db::open_in_memory().unwrap();
    db::migrate(&c).unwrap();
    // Migration 0003 disables most seed categories (only 14 active by
    // default). These router tests use Coffee as the canonical example
    // for the LLM tool path; reactivate everything seeded so they
    // don't depend on the curated default-active list.
    c.execute("UPDATE categories SET is_active = 1 WHERE is_seed = 1", [])
        .unwrap();
    c
}

fn make_deps(
    conn: Connection,
    llm: Arc<StubLlm>,
    telegram: Arc<StubTelegram>,
) -> (RouterDeps, Arc<Mutex<Connection>>) {
    let conn = Arc::new(Mutex::new(conn));
    let deps = RouterDeps {
        conn: conn.clone(),
        llm,
        client: telegram,
        state: Arc::new(BotState::new()),
        default_currency: "USD".into(),
    };
    (deps, conn)
}

fn message_update(update_id: i64, chat_id: i64, text: &str) -> Update {
    Update {
        update_id,
        message: Some(Message {
            message_id: update_id,
            date: 0,
            chat: Chat {
                id: chat_id,
                kind: "private".into(),
                username: None,
                first_name: None,
                last_name: None,
                title: None,
            },
            from: Some(User {
                id: chat_id,
                is_bot: false,
                first_name: "Test".into(),
                username: None,
            }),
            text: Some(text.to_string()),
        }),
    }
}

fn pair_owner(conn: &Mutex<Connection>, chat_id: i64, name: &str, now: OffsetDateTime) {
    let conn = conn.lock().unwrap();
    let code = generate_pairing_code(&conn, name, now).unwrap();
    moneypenny_lib::telegram::auth::redeem_pairing_code(&conn, chat_id, &code, now).unwrap();
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn unauthorized_chat_gets_polite_refusal() {
    let conn = fresh();
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, _conn) = make_deps(conn, llm.clone(), tg.clone());
    let now = datetime!(2026-04-28 12:00:00 UTC);

    handle_update(&deps, &message_update(1, 999, "$5 coffee"), now)
        .await
        .unwrap();

    let sent = tg.sent_to(999);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("private bot"));
    // LLM was never called for an unauthorized chat
    assert!(llm.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn start_with_valid_code_pairs_owner() {
    let conn = fresh();
    let now = datetime!(2026-04-28 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, conn_arc) = make_deps(conn, llm.clone(), tg.clone());

    // Pre-issue a pairing code outside the router.
    let code = {
        let c = conn_arc.lock().unwrap();
        generate_pairing_code(&c, "Wyatt", now).unwrap()
    };

    handle_update(
        &deps,
        &message_update(1, 111, &format!("/start {code}")),
        now,
    )
    .await
    .unwrap();

    let sent = tg.sent_to(111);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("Wyatt"));
    assert!(sent[0].contains("owner"));

    let auth = is_authorized(&conn_arc.lock().unwrap(), 111)
        .unwrap()
        .unwrap();
    assert_eq!(auth.role, Role::Owner);
}

#[tokio::test]
async fn start_with_invalid_code_refuses() {
    let conn = fresh();
    let now = datetime!(2026-04-28 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, _) = make_deps(conn, llm.clone(), tg.clone());

    handle_update(&deps, &message_update(1, 111, "/start 999999"), now)
        .await
        .unwrap();
    let sent = tg.sent_to(111);
    assert!(sent[0].contains("Couldn't pair"));
}

#[tokio::test]
async fn help_command_works_for_unpaired_chat() {
    // /help is auth-bypass so users can discover how to pair.
    let conn = fresh();
    let now = datetime!(2026-04-28 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, _) = make_deps(conn, llm, tg.clone());
    handle_update(&deps, &message_update(1, 999, "/help"), now)
        .await
        .unwrap();
    let sent = tg.sent_to(999);
    assert!(sent[0].contains("Mr. Moneypenny"));
}

#[tokio::test]
async fn free_text_logs_expense_via_llm_tool_call() {
    let conn = fresh();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, conn_arc) = make_deps(conn, llm.clone(), tg.clone());
    pair_owner(&conn_arc, 111, "Wyatt", now);

    // Two-turn: tool_use → tool result → final text.
    llm.enqueue(tool_use_response(
        "add_expense",
        json!({"amount": 5.0, "category": "Coffee"}),
    ));
    llm.enqueue(text_response("Logged $5 for Coffee."));

    handle_update(&deps, &message_update(1, 111, "$5 coffee"), now)
        .await
        .unwrap();

    let sent = tg.sent_to(111);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("Logged"));

    // Verify the expense actually landed.
    let count: i64 = conn_arc
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM expenses WHERE logged_by_chat_id = 111",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);

    // The dispatcher injected the chat_id; the LLM never specified it.
    let chat_id: i64 = conn_arc
        .lock()
        .unwrap()
        .query_row("SELECT logged_by_chat_id FROM expenses LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(chat_id, 111);
}

#[tokio::test]
async fn agent_loop_caps_at_max_iterations() {
    let conn = fresh();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, conn_arc) = make_deps(conn, llm.clone(), tg.clone());
    pair_owner(&conn_arc, 111, "Wyatt", now);

    // Endlessly emit tool_use to test the iteration cap.
    for _ in 0..10 {
        llm.enqueue(tool_use_response("list_categories", json!({})));
    }

    handle_update(&deps, &message_update(1, 111, "loop"), now)
        .await
        .unwrap();

    let sent = tg.sent_to(111);
    // After ≤ MAX_AGENT_ITERATIONS calls, the loop bails out with a
    // friendly "tangled up" message.
    assert!(sent.last().unwrap().contains("tangled"), "got: {:?}", sent);
}

#[tokio::test]
async fn undo_removes_recent_expense() {
    let conn = fresh();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, conn_arc) = make_deps(conn, llm.clone(), tg.clone());
    pair_owner(&conn_arc, 111, "Wyatt", now);

    // First log an expense via LLM
    llm.enqueue(tool_use_response(
        "add_expense",
        json!({"amount": 5.0, "category": "Coffee"}),
    ));
    llm.enqueue(text_response("Logged."));
    handle_update(&deps, &message_update(1, 111, "$5 coffee"), now)
        .await
        .unwrap();

    // Then /undo
    handle_update(&deps, &message_update(2, 111, "/undo"), now)
        .await
        .unwrap();

    let sent = tg.sent_to(111);
    assert!(sent.last().unwrap().to_lowercase().contains("undone"));
    let count: i64 = conn_arc
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM expenses", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn undo_with_nothing_recent_replies_politely() {
    let conn = fresh();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, conn_arc) = make_deps(conn, llm.clone(), tg.clone());
    pair_owner(&conn_arc, 111, "Wyatt", now);

    handle_update(&deps, &message_update(1, 111, "/undo"), now)
        .await
        .unwrap();
    let sent = tg.sent_to(111);
    assert!(sent[0].contains("Nothing to undo"));
}

#[tokio::test]
async fn cancel_clears_conversation_history() {
    let conn = fresh();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, conn_arc) = make_deps(conn, llm.clone(), tg.clone());
    pair_owner(&conn_arc, 111, "Wyatt", now);

    // Send a message so history populates.
    llm.enqueue(text_response("hello"));
    handle_update(&deps, &message_update(1, 111, "hi"), now)
        .await
        .unwrap();
    // Now /cancel
    handle_update(&deps, &message_update(2, 111, "/cancel"), now)
        .await
        .unwrap();

    // Subsequent free-text should not include "hi" in the prompt
    // history. We only check that the sent reply mentions "Cancelled".
    let sent = tg.sent_to(111);
    assert!(sent.last().unwrap().contains("Cancelled"));
}

// ---- recurring rule confirmation intercept ----

fn insert_pending_for_test(
    conn: &Mutex<Connection>,
    chat_id: i64,
    rule_id: i64,
    now: OffsetDateTime,
) {
    use moneypenny_lib::repository::recurring_rules;
    use time::Duration;
    let c = conn.lock().unwrap();
    recurring_rules::insert_pending(&c, chat_id, rule_id, now, now + Duration::hours(36)).unwrap();
}

fn insert_test_recurring_rule(conn: &Mutex<Connection>, label: &str) -> i64 {
    use moneypenny_lib::domain::recurring::{Frequency, NewRecurringRule, RecurringMode};
    use moneypenny_lib::repository::{categories, recurring_rules};
    let c = conn.lock().unwrap();
    let cat = categories::get_by_name(&c, "Entertainment")
        .unwrap()
        .unwrap();
    recurring_rules::insert(
        &c,
        &NewRecurringRule {
            label: label.into(),
            amount_cents: 1_549,
            currency: "USD".into(),
            category_id: cat.id,
            frequency: Frequency::Monthly,
            anchor_day: 15,
            mode: RecurringMode::Confirm,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn pending_yes_inserts_expense_and_clears_pending() {
    let conn = fresh();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default()); // intentionally empty: must NOT be called
    let tg = Arc::new(StubTelegram::default());
    let (deps, conn_arc) = make_deps(conn, llm.clone(), tg.clone());
    pair_owner(&conn_arc, 111, "Wyatt", now);
    let rule_id = insert_test_recurring_rule(&conn_arc, "Netflix");
    insert_pending_for_test(&conn_arc, 111, rule_id, now);

    handle_update(&deps, &message_update(1, 111, "yes"), now)
        .await
        .unwrap();

    let sent = tg.sent_to(111);
    assert!(sent.last().unwrap().contains("Logged"));

    // Expense was inserted.
    let count: i64 = {
        let c = conn_arc.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM expenses WHERE description = 'Netflix'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(count, 1);

    // Pending cleared.
    let pending_count: i64 = {
        let c = conn_arc.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM pending_recurring_confirmations WHERE chat_id = 111",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(pending_count, 0);

    // LLM was NOT called for a pending-resolution message.
    assert!(
        llm.requests.lock().unwrap().is_empty(),
        "yes/no must short-circuit before LLM dispatch"
    );
}

#[tokio::test]
async fn pending_skip_clears_without_inserting() {
    let conn = fresh();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, conn_arc) = make_deps(conn, llm.clone(), tg.clone());
    pair_owner(&conn_arc, 111, "Wyatt", now);
    let rule_id = insert_test_recurring_rule(&conn_arc, "Netflix");
    insert_pending_for_test(&conn_arc, 111, rule_id, now);

    handle_update(&deps, &message_update(1, 111, "skip"), now)
        .await
        .unwrap();

    let sent = tg.sent_to(111);
    assert!(sent.last().unwrap().contains("Skipped"));

    let count: i64 = {
        let c = conn_arc.lock().unwrap();
        c.query_row("SELECT COUNT(*) FROM expenses", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count, 0, "skip must not insert an expense");
}

#[tokio::test]
async fn pending_unknown_reply_re_prompts_without_dropping_pending() {
    let conn = fresh();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, conn_arc) = make_deps(conn, llm.clone(), tg.clone());
    pair_owner(&conn_arc, 111, "Wyatt", now);
    let rule_id = insert_test_recurring_rule(&conn_arc, "Netflix");
    insert_pending_for_test(&conn_arc, 111, rule_id, now);

    handle_update(&deps, &message_update(1, 111, "what?"), now)
        .await
        .unwrap();

    let sent = tg.sent_to(111);
    assert!(sent.last().unwrap().contains("yes"));

    // Pending NOT cleared.
    let pending_count: i64 = {
        let c = conn_arc.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM pending_recurring_confirmations WHERE chat_id = 111",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(pending_count, 1);
}

#[tokio::test]
async fn expired_pending_falls_through_to_llm() {
    let conn = fresh();
    let asked_at = datetime!(2026-04-10 12:00:00 UTC);
    let now = datetime!(2026-04-15 12:00:00 UTC); // 5 days later, well past 36h TTL
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, conn_arc) = make_deps(conn, llm.clone(), tg.clone());
    pair_owner(&conn_arc, 111, "Wyatt", now);
    let rule_id = insert_test_recurring_rule(&conn_arc, "Netflix");
    insert_pending_for_test(&conn_arc, 111, rule_id, asked_at);

    // The LLM will be hit by free-text — give it a canned response.
    llm.enqueue(text_response("hi"));
    handle_update(&deps, &message_update(1, 111, "yes"), now)
        .await
        .unwrap();

    // LLM was called (we fell through).
    assert_eq!(llm.requests.lock().unwrap().len(), 1);

    // Expired pending was cleaned up.
    let pending_count: i64 = {
        let c = conn_arc.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM pending_recurring_confirmations WHERE chat_id = 111",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(pending_count, 0);
}

#[tokio::test]
async fn cancel_clears_pending_confirmation() {
    let conn = fresh();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, conn_arc) = make_deps(conn, llm.clone(), tg.clone());
    pair_owner(&conn_arc, 111, "Wyatt", now);
    let rule_id = insert_test_recurring_rule(&conn_arc, "Netflix");
    insert_pending_for_test(&conn_arc, 111, rule_id, now);

    handle_update(&deps, &message_update(1, 111, "/cancel"), now)
        .await
        .unwrap();

    let pending_count: i64 = {
        let c = conn_arc.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM pending_recurring_confirmations WHERE chat_id = 111",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(pending_count, 0);
}

#[tokio::test]
async fn empty_text_message_ignored() {
    let conn = fresh();
    let now = datetime!(2026-04-15 12:00:00 UTC);
    let llm = Arc::new(StubLlm::default());
    let tg = Arc::new(StubTelegram::default());
    let (deps, conn_arc) = make_deps(conn, llm.clone(), tg.clone());
    pair_owner(&conn_arc, 111, "Wyatt", now);

    let mut upd = message_update(1, 111, "");
    if let Some(m) = upd.message.as_mut() {
        m.text = Some(String::new());
    }
    handle_update(&deps, &upd, now).await.unwrap();
    assert!(tg.sent_to(111).is_empty(), "no reply for empty text");
}
