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
