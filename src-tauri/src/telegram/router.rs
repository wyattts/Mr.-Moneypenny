//! Update router. Maps incoming Telegram messages to:
//!   - slash commands (`/start`, `/help`, `/undo`, `/cancel`)
//!   - the LLM agentic loop for free-text messages.
//!
//! The router holds NO live connections or HTTP clients itself; everything
//! comes through `RouterDeps` so this module can be unit-tested with a
//! stub `TelegramApi` and a stub `LLMProvider`.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use rusqlite::Connection;
use time::OffsetDateTime;

use crate::domain::{CategoryKind, ExpenseSource, NewExpense};
use crate::llm::dispatcher::{self, CallContext};
use crate::llm::system_prompt::{build_system_prompt, SystemPromptInput};
use crate::llm::tools::all_tools;
use crate::llm::{ChatRequest, ContentBlock, LLMProvider, Message, Role, StopReason};
use crate::repository::{categories, expenses, llm_usage, recurring_rules};

use super::auth::{self, AuthorizedChat};
use super::client::{TelegramApi, Update};
use super::formatter;
use super::state::BotState;

/// Hard cap on the LLM⇄dispatcher loop. Beyond this we refuse to keep
/// going and surface a polite "I'm stuck" reply.
const MAX_AGENT_ITERATIONS: usize = 5;
const MAX_TOKENS_PER_TURN: u32 = 1024;

#[derive(Clone)]
pub struct RouterDeps {
    pub conn: Arc<Mutex<Connection>>,
    pub llm: Arc<dyn LLMProvider>,
    pub client: Arc<dyn TelegramApi>,
    pub state: Arc<BotState>,
    pub default_currency: String,
}

/// Process a single Telegram update.
pub async fn handle_update(deps: &RouterDeps, update: &Update, now: OffsetDateTime) -> Result<()> {
    let Some(message) = &update.message else {
        return Ok(());
    };
    let chat_id = message.chat.id;
    let text = message.text.as_deref().unwrap_or("").trim().to_string();
    if text.is_empty() {
        return Ok(()); // photo / sticker / etc. — ignore for now
    }

    // ------------------------------------------------------------------
    // Slash commands that bypass the auth gate (only `/start` and `/help`).
    // ------------------------------------------------------------------
    if let Some(rest) = text.strip_prefix("/start") {
        return handle_start(deps, chat_id, rest.trim(), now).await;
    }
    if text == "/help" {
        return reply(deps, chat_id, formatter::help_text()).await;
    }

    // ------------------------------------------------------------------
    // Auth gate: every other interaction requires an authorized chat.
    // ------------------------------------------------------------------
    let auth_chat = {
        let conn = deps.conn.lock().unwrap();
        auth::is_authorized(&conn, chat_id)?
    };
    let Some(auth_chat) = auth_chat else {
        // Polite refusal. The plan also allows "silently ignore"; we
        // chose polite-refusal as the default so users who accidentally
        // open the bot from a search aren't left wondering.
        return reply(deps, chat_id, formatter::unauthorized_text()).await;
    };

    // ------------------------------------------------------------------
    // Pending recurring-rule confirmation: if the bot DM'd this chat a
    // "yes/no/skip" prompt and is waiting on the answer, intercept the
    // user's reply *before* it reaches the LLM. This is deliberate:
    // the LLM should never silently log money on the user's behalf.
    // ------------------------------------------------------------------
    if try_resolve_pending_confirmation(deps, chat_id, &text, now).await? {
        return Ok(());
    }

    // ------------------------------------------------------------------
    // Authenticated slash commands.
    // ------------------------------------------------------------------
    match text.as_str() {
        "/cancel" => {
            deps.state.conversations.lock().unwrap().clear(chat_id);
            // Also clear any pending recurring-rule confirmation so the
            // user doesn't have a stale prompt hanging over the next chat.
            {
                let conn = deps.conn.lock().unwrap();
                let _ = recurring_rules::delete_pending(&conn, chat_id);
            }
            return reply(deps, chat_id, "Cancelled. Anything else?".to_string()).await;
        }
        "/undo" => return handle_undo(deps, chat_id, &auth_chat, now).await,
        _ => {}
    }

    // ------------------------------------------------------------------
    // Free-text → LLM agentic loop.
    // ------------------------------------------------------------------
    handle_free_text(deps, chat_id, &auth_chat, &text, now).await
}

// ---------------------------------------------------------------------
// Slash command handlers.
// ---------------------------------------------------------------------

async fn handle_start(
    deps: &RouterDeps,
    chat_id: i64,
    arg: &str,
    now: OffsetDateTime,
) -> Result<()> {
    if arg.is_empty() {
        // Bare `/start` — give them help.
        return reply(deps, chat_id, formatter::help_text()).await;
    }
    let result = {
        let conn = deps.conn.lock().unwrap();
        auth::redeem_pairing_code(&conn, chat_id, arg, now)
    };
    match result {
        Ok(authd) => {
            let txt = formatter::paired_text(&authd.display_name, authd.role.as_str());
            reply(deps, chat_id, txt).await
        }
        Err(e) => {
            let txt = formatter::pairing_failed_text(&e.to_string());
            reply(deps, chat_id, txt).await
        }
    }
}

async fn handle_undo(
    deps: &RouterDeps,
    chat_id: i64,
    auth_chat: &AuthorizedChat,
    _now: OffsetDateTime,
) -> Result<()> {
    // Find the most recent expense logged by THIS chat, within the last
    // 5 minutes. Older than that → ask them to delete via free-text
    // ("delete the $5 coffee from yesterday") so they don't accidentally
    // wipe something they meant to keep.
    let target = {
        let conn = deps.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT e.id, e.amount_cents, e.currency, COALESCE(c.name, '(uncategorized)'), e.description
             FROM expenses e LEFT JOIN categories c ON c.id = e.category_id
             WHERE e.logged_by_chat_id = ?1
               AND e.created_at > datetime('now', '-5 minutes')
             ORDER BY e.id DESC LIMIT 1",
        )?;
        stmt.query_row::<(i64, i64, String, String, Option<String>), _, _>(
            rusqlite::params![auth_chat.chat_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .ok()
    };

    let Some((id, amount_cents, currency, category, description)) = target else {
        return reply(
            deps,
            chat_id,
            "Nothing to undo from the last 5 minutes.".to_string(),
        )
        .await;
    };

    let removed = {
        let conn = deps.conn.lock().unwrap();
        expenses::delete(&conn, id)?
    };
    if !removed {
        return reply(deps, chat_id, "Couldn't undo (already gone?).".to_string()).await;
    }
    let amount = formatter::format_money(amount_cents, &currency);
    let desc = description.map(|d| format!(" ({d})")).unwrap_or_default();
    let msg = format!("Undone: {amount} {category}{desc}.");
    reply(deps, chat_id, msg).await
}

// ---------------------------------------------------------------------
// Agentic free-text loop.
// ---------------------------------------------------------------------

async fn handle_free_text(
    deps: &RouterDeps,
    chat_id: i64,
    auth_chat: &AuthorizedChat,
    text: &str,
    now: OffsetDateTime,
) -> Result<()> {
    // Build the system prompt once per turn (categories don't change mid-turn).
    let (system_prompt, members) = {
        let conn = deps.conn.lock().unwrap();
        let cats = categories::list(&conn, false)?;
        let cat_pairs: Vec<(String, CategoryKind)> =
            cats.into_iter().map(|c| (c.name, c.kind)).collect();
        let members = auth::list_members(&conn)?;
        let prompt = build_system_prompt(&SystemPromptInput {
            now,
            authorized_chat_name: Some(auth_chat.display_name.clone()),
            authorized_chat_role: Some(auth_chat.role.as_str().to_string()),
            categories: cat_pairs,
            user_currency: deps.default_currency.clone(),
            household_members: members.iter().map(|m| m.display_name.clone()).collect(),
        });
        (prompt, members)
    };
    let _ = members; // currently unused outside the prompt; keep for future per-member handlers

    // Append the user's message to history.
    deps.state
        .conversations
        .lock()
        .unwrap()
        .append(chat_id, Message::user_text(text), now);

    let tools = all_tools();
    let mut iterations = 0;
    let final_text = loop {
        iterations += 1;
        if iterations > MAX_AGENT_ITERATIONS {
            tracing::warn!(target: "telegram::router", chat_id, "agent loop exceeded {MAX_AGENT_ITERATIONS} iterations");
            break "I got tangled up — could you try rephrasing?".to_string();
        }

        let messages = deps
            .state
            .conversations
            .lock()
            .unwrap()
            .history(chat_id, now);
        let request = ChatRequest {
            system_prompt: system_prompt.clone(),
            messages,
            tools: tools.clone(),
            max_tokens: MAX_TOKENS_PER_TURN,
        };

        let response = match deps.llm.chat(request).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(target: "telegram::router", chat_id, error=%e, "llm call failed");
                break format!("Sorry — the LLM is unhappy: {e}");
            }
        };

        // Persist usage row for the cost tracker. Best-effort: a DB blip
        // here must not fail the user's chat turn.
        {
            let conn = deps.conn.lock().unwrap();
            if let Err(e) = llm_usage::log(
                &conn,
                deps.llm.provider_name(),
                deps.llm.model(),
                &response.usage,
                now,
            ) {
                tracing::warn!(target: "telegram::router", error=%e, "logging llm usage failed");
            }
        }

        // Persist the assistant turn into history before doing anything else
        // so a subsequent error still leaves the user message + the
        // assistant's last attempt visible to the next turn.
        let assistant_msg = Message {
            role: Role::Assistant,
            content: response.content.clone(),
        };
        deps.state
            .conversations
            .lock()
            .unwrap()
            .append(chat_id, assistant_msg, now);

        // If no tool_use, we're done — reply with the text.
        let tool_uses: Vec<(String, String, serde_json::Value)> = response
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::ToolUse { id, name, input } => {
                    Some((id.clone(), name.clone(), input.clone()))
                }
                _ => None,
            })
            .collect();
        if tool_uses.is_empty() || response.stop_reason == StopReason::EndTurn {
            break response.assistant_text();
        }

        // Execute each tool call and append its result to history.
        for (id, name, input) in tool_uses {
            let output = {
                let conn = deps.conn.lock().unwrap();
                let ctx = CallContext {
                    now,
                    authorized_chat_id: Some(auth_chat.chat_id),
                    authorized_chat_name: Some(auth_chat.display_name.clone()),
                    default_currency: deps.default_currency.clone(),
                };
                dispatcher::execute(&conn, &ctx, &id, &name, &input)
            };
            let tool_msg = if output.is_error {
                Message::tool_error(output.tool_use_id, output.content)
            } else {
                Message::tool_result(output.tool_use_id, output.content)
            };
            deps.state
                .conversations
                .lock()
                .unwrap()
                .append(chat_id, tool_msg, now);
        }
        // Loop again so the LLM can synthesize a final answer from the
        // tool results.
    };

    let final_text = if final_text.trim().is_empty() {
        "Done.".to_string()
    } else {
        final_text
    };
    reply(deps, chat_id, final_text).await
}

// ---------------------------------------------------------------------
// Recurring-rule confirmation intercept.
// ---------------------------------------------------------------------

/// If `chat_id` has an outstanding recurring-rule confirmation, parse
/// `text` as the user's answer and act on it. Returns `Ok(true)` when
/// the message was consumed (caller should stop processing it), or
/// `Ok(false)` when it should fall through to the normal command/LLM
/// dispatch path.
async fn try_resolve_pending_confirmation(
    deps: &RouterDeps,
    chat_id: i64,
    text: &str,
    now: time::OffsetDateTime,
) -> Result<bool> {
    // Slash commands always fall through — `/cancel` should be able to
    // dismiss a pending confirmation without being misread as a reply.
    if text.starts_with('/') {
        return Ok(false);
    }
    let pending = {
        let conn = deps.conn.lock().unwrap();
        recurring_rules::get_pending(&conn, chat_id)?
    };
    let Some(pending) = pending else {
        return Ok(false);
    };

    // Expired? Drop it silently and let the message go through normally.
    if pending.expires_at <= now {
        let conn = deps.conn.lock().unwrap();
        let _ = recurring_rules::delete_pending(&conn, chat_id);
        return Ok(false);
    }

    let normalized = text.trim().to_lowercase();
    let action = match normalized.as_str() {
        "yes" | "y" | "confirm" | "ok" | "okay" | "sure" | "👍" => Some(true),
        "no" | "n" | "skip" | "s" | "decline" | "👎" => Some(false),
        _ => None,
    };

    let Some(should_log) = action else {
        // Unknown reply — re-prompt. Don't drop the pending so the user
        // can correct.
        reply(
            deps,
            chat_id,
            "Reply *yes* to log the recurring expense, or *no* / *skip* to skip it.".to_string(),
        )
        .await?;
        return Ok(true);
    };

    // Load the rule (it may have been deleted between ask and answer —
    // treat as a polite "never mind").
    let rule = {
        let conn = deps.conn.lock().unwrap();
        recurring_rules::get(&conn, pending.rule_id)?
    };
    let Some(rule) = rule else {
        {
            let conn = deps.conn.lock().unwrap();
            let _ = recurring_rules::delete_pending(&conn, chat_id);
        }
        reply(
            deps,
            chat_id,
            "That recurring rule was deleted. Nothing to log.".to_string(),
        )
        .await?;
        return Ok(true);
    };

    // Compute the response text *and* drop the connection guard before
    // the await — Send-safety on the spawned task requires no
    // MutexGuard to be live across .await.
    let response_text = {
        let conn = deps.conn.lock().unwrap();
        let text = if should_log {
            let amount = super::formatter::format_money(rule.amount_cents, &rule.currency);
            expenses::insert(
                &conn,
                &NewExpense {
                    amount_cents: rule.amount_cents,
                    currency: rule.currency.clone(),
                    category_id: Some(rule.category_id),
                    description: Some(rule.label.clone()),
                    occurred_at: now,
                    source: ExpenseSource::Telegram,
                    raw_message: Some(format!("recurring rule #{} (confirmed)", rule.id)),
                    llm_confidence: None,
                    logged_by_chat_id: Some(chat_id),
                    is_refund: false,
                    refund_for_expense_id: None,
                },
            )?;
            let _ = recurring_rules::delete_pending(&conn, chat_id);
            format!("Logged {amount} for {label}.", label = rule.label)
        } else {
            let _ = recurring_rules::delete_pending(&conn, chat_id);
            format!("Skipped {} this time.", rule.label)
        };
        drop(conn);
        text
    };

    reply(deps, chat_id, response_text).await?;
    Ok(true)
}

// ---------------------------------------------------------------------
// Tiny helper.
// ---------------------------------------------------------------------

async fn reply(deps: &RouterDeps, chat_id: i64, text: String) -> Result<()> {
    deps.client
        .send_message(chat_id, &text)
        .await
        .map(|_| ())
        .map_err(|e| anyhow!("send_message failed: {e}"))
}
