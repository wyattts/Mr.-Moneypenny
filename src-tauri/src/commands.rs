// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//! Tauri IPC commands exposed to the React frontend.
//!
//! Each `#[tauri::command]` is the only way the frontend can touch the
//! database, secrets, or network — keep these handlers small and route
//! into the typed library code, never inline business logic here.

use serde::{Deserialize, Serialize};
use tauri::State;
use time::{Date, Duration, OffsetDateTime, Time};

use rusqlite::Connection;

use crate::app_state::AppState;
use crate::domain::{CategoryKind, ExpenseSource, NewCategory};
use crate::insights::{
    dashboard,
    forecast::{self as forecast_mod, ScenarioCut, ScenarioResult},
    range::DateRange,
    stats::{self as stats_mod, DescriptiveStats, Histogram},
    DashboardSnapshot,
};
use crate::llm::anthropic::{AnthropicProvider, DEFAULT_MODEL as DEFAULT_ANTHROPIC_MODEL};
use crate::llm::ollama::OllamaProvider;
use crate::llm::system_prompt::SystemPrompt;
use crate::llm::{ChatRequest, LLMProvider, Message};
use crate::repository::{categories, expenses, llm_usage, settings};
use crate::secrets;
use crate::telegram::auth::{self, AuthorizedChat};
use crate::telegram::client::{TelegramApi, TelegramClient};

/// Convert any anyhow / wrapped error to a String the frontend can display.
fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

// ---------------------------------------------------------------------
// Setup state.
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SetupState {
    pub setup_complete: bool,
    pub last_completed_step: u8,
    pub llm_provider: Option<String>,
    pub anthropic_key_set: bool,
    pub telegram_token_set: bool,
    pub authorized_chat_count: i64,
    pub default_currency: String,
    pub locale: Option<String>,
    pub ollama_endpoint: Option<String>,
    pub ollama_model: Option<String>,
}

#[tauri::command]
pub async fn get_setup_state(state: State<'_, AppState>) -> Result<SetupState, String> {
    // Anthropic + Telegram secret checks happen outside the actor —
    // they go through the secrets module which has its own concurrency
    // model. Everything else folds into one actor round-trip.
    let anthropic_key_set = secrets::exists(secrets::keys::ANTHROPIC_API_KEY).map_err(err)?;
    let telegram_token_set = secrets::exists(secrets::keys::TELEGRAM_BOT_TOKEN).map_err(err)?;
    state
        .db_actor
        .run(move |conn| {
            let setup_complete =
                settings::get(conn, settings::keys::SETUP_COMPLETE)?.as_deref() == Some("1");
            let last_completed_step = settings::get(conn, settings::keys::SETUP_STEP)?
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(0);
            let llm_provider = settings::get(conn, settings::keys::LLM_PROVIDER)?;
            let default_currency =
                settings::get_or_default(conn, settings::keys::DEFAULT_CURRENCY, "USD")?;
            let locale = settings::get(conn, settings::keys::LOCALE)?;
            let ollama_endpoint = settings::get(conn, settings::keys::OLLAMA_ENDPOINT)?;
            let ollama_model = settings::get(conn, settings::keys::OLLAMA_MODEL)?;

            let authorized_chat_count: i64 =
                conn.query_row("SELECT COUNT(*) FROM telegram_authorized_chats", [], |r| {
                    r.get(0)
                })?;

            Ok(SetupState {
                setup_complete,
                last_completed_step,
                llm_provider,
                anthropic_key_set,
                telegram_token_set,
                authorized_chat_count,
                default_currency,
                locale,
                ollama_endpoint,
                ollama_model,
            })
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_setup_step(step: u8, state: State<'_, AppState>) -> Result<(), String> {
    state
        .db_actor
        .run(move |conn| settings::set(conn, settings::keys::SETUP_STEP, &step.to_string()))
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// LLM provider selection + config.
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn save_llm_provider(provider: String, state: State<'_, AppState>) -> Result<(), String> {
    if provider != "anthropic" && provider != "ollama" {
        return Err(format!("invalid provider: {provider}"));
    }
    state
        .db_actor
        .run(move |conn| settings::set(conn, settings::keys::LLM_PROVIDER, &provider))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn save_anthropic_key(api_key: String) -> Result<(), String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("API key cannot be empty".into());
    }
    secrets::store(secrets::keys::ANTHROPIC_API_KEY, trimmed).map_err(err)
}

#[tauri::command]
pub async fn test_anthropic(state: State<'_, AppState>) -> Result<String, String> {
    let key = secrets::retrieve(secrets::keys::ANTHROPIC_API_KEY)
        .map_err(err)?
        .ok_or_else(|| "no Anthropic API key saved".to_string())?;
    let model = state
        .db_actor
        .run(|conn| {
            settings::get_or_default(
                conn,
                settings::keys::ANTHROPIC_MODEL,
                DEFAULT_ANTHROPIC_MODEL,
            )
        })
        .await
        .map_err(err)?;
    let provider =
        AnthropicProvider::with_options(key, &model, "https://api.anthropic.com").map_err(err)?;
    // Minimal probe: 1-output-token call. Costs ~$0.0001 but validates auth + model.
    let request = ChatRequest {
        system_prompt: SystemPrompt {
            stable: "Reply with the single word: ok".into(),
            volatile: String::new(),
        },
        messages: vec![Message::user_text("ping")],
        tools: vec![],
        max_tokens: 4,
    };
    provider.chat(request).await.map_err(err)?;
    Ok(model)
}

#[tauri::command]
pub async fn save_ollama_config(
    endpoint: String,
    model: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Validation needs the allow_remote setting; one actor round-trip
    // reads it. We validate outside the actor (CPU-only) and the
    // second round-trip writes both keys together.
    let allow_remote = state
        .db_actor
        .run(|conn| {
            Ok(settings::get_or_default(conn, settings::keys::OLLAMA_ALLOW_REMOTE, "0")? == "1")
        })
        .await
        .map_err(err)?;
    let cleaned = crate::llm::ollama::validate_endpoint(&endpoint, allow_remote).map_err(err)?;
    state
        .db_actor
        .run(move |conn| {
            settings::set(conn, settings::keys::OLLAMA_ENDPOINT, &cleaned)?;
            settings::set(conn, settings::keys::OLLAMA_MODEL, &model)?;
            Ok(())
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn list_ollama_models(
    endpoint: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let allow_remote = state
        .db_actor
        .run(|conn| {
            Ok(settings::get_or_default(conn, settings::keys::OLLAMA_ALLOW_REMOTE, "0")? == "1")
        })
        .await
        .map_err(err)?;
    let cleaned = crate::llm::ollama::validate_endpoint(&endpoint, allow_remote).map_err(err)?;
    let url = format!("{}/api/tags", cleaned.trim_end_matches('/'));
    let resp = reqwest::Client::new().get(&url).send().await.map_err(err)?;
    if !resp.status().is_success() {
        return Err(format!("ollama returned {}", resp.status()));
    }
    #[derive(Deserialize)]
    struct Tags {
        models: Vec<TagModel>,
    }
    #[derive(Deserialize)]
    struct TagModel {
        name: String,
    }
    let tags: Tags = resp.json().await.map_err(err)?;
    Ok(tags.models.into_iter().map(|m| m.name).collect())
}

#[tauri::command]
pub async fn test_ollama(state: State<'_, AppState>) -> Result<String, String> {
    let (endpoint, model): (String, String) = state
        .db_actor
        .run(|conn| {
            let endpoint = settings::get_or_default(
                conn,
                settings::keys::OLLAMA_ENDPOINT,
                "http://localhost:11434",
            )?;
            let model = settings::get_or_default(conn, settings::keys::OLLAMA_MODEL, "llama3:8b")?;
            Ok((endpoint, model))
        })
        .await
        .map_err(err)?;
    let provider = OllamaProvider::with_base_url(model.clone(), endpoint).map_err(err)?;
    let request = ChatRequest {
        system_prompt: SystemPrompt {
            stable: "Reply with the single word: ok".into(),
            volatile: String::new(),
        },
        messages: vec![Message::user_text("ping")],
        tools: vec![],
        max_tokens: 4,
    };
    provider.chat(request).await.map_err(err)?;
    Ok(model)
}

// ---------------------------------------------------------------------
// Telegram configuration.
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct TelegramBotInfo {
    pub id: i64,
    pub username: Option<String>,
    pub first_name: String,
}

#[tauri::command]
pub async fn save_telegram_token(
    token: String,
    state: State<'_, AppState>,
) -> Result<TelegramBotInfo, String> {
    let trimmed = token.trim();
    // Basic shape check before we hit the API: `<bot_id>:<random suffix>`.
    if !trimmed.contains(':') || trimmed.len() < 35 {
        return Err("token doesn't look like a Telegram bot token".into());
    }
    let client = TelegramClient::new(trimmed).map_err(err)?;
    let me = client.get_me().await.map_err(err)?;
    if !me.is_bot {
        return Err("this is not a bot account".into());
    }
    // Defensive: ensure no leftover webhook is set, otherwise long-polling
    // will return 409 Conflict.
    client.delete_webhook().await.map_err(err)?;

    secrets::store(secrets::keys::TELEGRAM_BOT_TOKEN, trimmed).map_err(err)?;

    // Restart (or first-spawn) the poller so the in-memory TelegramClient
    // picks up the new token. Plain ensure_poller_running is no-op once
    // started, which would leave the previous bot's poll loop running
    // against the old credentials and the new bot silently unattended.
    state.restart_poller().map_err(err)?;

    Ok(TelegramBotInfo {
        id: me.id,
        username: me.username,
        first_name: me.first_name,
    })
}

#[tauri::command]
pub async fn generate_pairing_code(
    display_name: String,
    is_owner_invite: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let kind = if is_owner_invite {
        auth::PairingKind::Owner
    } else {
        auth::PairingKind::Member
    };
    state
        .db_actor
        .run(move |conn| {
            auth::generate_pairing_code(conn, &display_name, kind, time::OffsetDateTime::now_utc())
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn list_authorized_chats(
    state: State<'_, AppState>,
) -> Result<Vec<AuthorizedChat>, String> {
    state
        .db_actor
        .run(|conn| auth::list_members(conn))
        .await
        .map_err(err)
}

/// Wipe every authorized chat and pending pairing code. Used by the
/// Settings UI's "factory reset" toggle when rotating the bot token.
/// Returns the number of authorized chats that were removed.
#[tauri::command]
pub async fn clear_authorized_chats(state: State<'_, AppState>) -> Result<usize, String> {
    state
        .db_actor
        .run(|conn| auth::clear_all(conn))
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// Currency, locale, categories.
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn save_currency_locale(
    currency: String,
    locale: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db_actor
        .run(move |conn| {
            settings::set(conn, settings::keys::DEFAULT_CURRENCY, &currency)?;
            settings::set(conn, settings::keys::LOCALE, &locale)?;
            Ok(())
        })
        .await
        .map_err(err)
}

#[derive(Debug, Serialize)]
pub struct CategoryView {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub monthly_target_cents: Option<i64>,
    pub is_recurring: bool,
    pub recurrence_day_of_month: Option<u8>,
    pub is_active: bool,
    pub is_seed: bool,
}

#[tauri::command]
pub async fn list_categories(
    include_inactive: bool,
    state: State<'_, AppState>,
) -> Result<Vec<CategoryView>, String> {
    let cats = state
        .db_actor
        .run(move |conn| categories::list(conn, include_inactive))
        .await
        .map_err(err)?;
    Ok(cats
        .into_iter()
        .map(|c| CategoryView {
            id: c.id,
            kind: c.kind.as_str().into(),
            name: c.name,
            monthly_target_cents: c.monthly_target_cents,
            is_recurring: c.is_recurring,
            recurrence_day_of_month: c.recurrence_day_of_month,
            is_active: c.is_active,
            is_seed: c.is_seed,
        })
        .collect())
}

#[tauri::command]
pub async fn set_category_target(
    id: i64,
    monthly_target_cents: Option<i64>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db_actor
        .run(move |conn| {
            categories::set_monthly_target(conn, id, monthly_target_cents)?;
            Ok(())
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_category_active(
    id: i64,
    is_active: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db_actor
        .run(move |conn| {
            categories::set_active(conn, id, is_active)?;
            Ok(())
        })
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// Finalize.
// ---------------------------------------------------------------------

#[tauri::command]
#[allow(clippy::collapsible_match)]
pub async fn finalize_setup(state: State<'_, AppState>) -> Result<(), String> {
    // The secrets:: checks have their own sync model; do them outside
    // the actor so they don't tie up the writer thread on a slow file
    // read.
    let has_anthropic = secrets::exists(secrets::keys::ANTHROPIC_API_KEY).map_err(err)?;
    let has_tg_token = secrets::exists(secrets::keys::TELEGRAM_BOT_TOKEN).map_err(err)?;
    if !has_tg_token {
        return Err("save your Telegram bot token first".into());
    }
    state
        .db_actor
        .run(move |conn| {
            // Sanity: refuse to finalize if prerequisites are missing.
            let provider = settings::get(conn, settings::keys::LLM_PROVIDER)?
                .ok_or_else(|| anyhow::anyhow!("pick an LLM provider first"))?;
            match provider.as_str() {
                "anthropic" => {
                    if !has_anthropic {
                        anyhow::bail!("save your Anthropic API key first");
                    }
                }
                "ollama" => {
                    if settings::get(conn, settings::keys::OLLAMA_ENDPOINT)?.is_none() {
                        anyhow::bail!("save your Ollama endpoint first");
                    }
                }
                _ => {}
            }
            let auth_count: i64 =
                conn.query_row("SELECT COUNT(*) FROM telegram_authorized_chats", [], |r| {
                    r.get(0)
                })?;
            if auth_count == 0 {
                anyhow::bail!("pair a chat first (send /start <code> to your bot)");
            }
            settings::set(conn, settings::keys::SETUP_COMPLETE, "1")?;
            Ok(())
        })
        .await
        .map_err(err)?;
    // Make sure the poller is running with the final config.
    state.ensure_poller_running().map_err(err)?;
    Ok(())
}

// ---------------------------------------------------------------------
// Dashboard.
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RangeArg {
    ThisWeek,
    ThisMonth,
    ThisQuarter,
    ThisYear,
    Ytd,
    Custom { from: Date, to: Date },
    Month { year: i32, month: u8 },
}

impl From<RangeArg> for DateRange {
    fn from(value: RangeArg) -> Self {
        match value {
            RangeArg::ThisWeek => DateRange::ThisWeek,
            RangeArg::ThisMonth => DateRange::ThisMonth,
            RangeArg::ThisQuarter => DateRange::ThisQuarter,
            RangeArg::ThisYear => DateRange::ThisYear,
            RangeArg::Ytd => DateRange::Ytd,
            RangeArg::Custom { from, to } => DateRange::Custom { from, to },
            RangeArg::Month { year, month } => DateRange::Month { year, month },
        }
    }
}

#[tauri::command]
pub async fn get_dashboard(
    range: RangeArg,
    state: State<'_, AppState>,
) -> Result<DashboardSnapshot, String> {
    state
        .db_actor
        .run(move |conn| dashboard(conn, range.into(), OffsetDateTime::now_utc()))
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// Ledger / expenses.
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct ExpenseFilters {
    #[serde(default)]
    pub category_id: Option<i64>,
    #[serde(default)]
    pub start_date: Option<Date>,
    #[serde(default)]
    pub end_date: Option<Date>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct LedgerRow {
    pub id: i64,
    pub amount_cents: i64,
    pub currency: String,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub category_kind: Option<String>,
    pub description: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
    pub source: String,
    pub logged_by_chat_id: Option<i64>,
    pub logged_by_name: Option<String>,
}

/// Plain helper version of the `list_expenses` SQL pipeline so the
/// integration tests in `tests/integration_commands.rs` can exercise
/// it without going through Tauri's `State<>` machinery (audit T-1).
/// The Tauri command wrapper below is a one-liner that defers here.
pub fn list_expenses_query(
    conn: &Connection,
    filters: &ExpenseFilters,
    tz: time::UtcOffset,
) -> anyhow::Result<Vec<LedgerRow>> {
    let limit = filters.limit.unwrap_or(100).min(500);
    let offset = filters.offset.unwrap_or(0);
    let mut sql = String::from(
        "SELECT e.id, e.amount_cents, e.currency, e.category_id, c.name, c.kind, e.description,
                e.occurred_at, e.source, e.logged_by_chat_id, t.display_name
         FROM expenses e
         LEFT JOIN categories c ON c.id = e.category_id
         LEFT JOIN telegram_authorized_chats t ON t.chat_id = e.logged_by_chat_id
         WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(id) = filters.category_id {
        sql.push_str(" AND e.category_id = ?");
        params.push(Box::new(id));
    }
    if let Some(d) = filters.start_date {
        sql.push_str(" AND e.occurred_at >= ?");
        params.push(Box::new(d.with_time(Time::MIDNIGHT).assume_offset(tz)));
    }
    if let Some(d) = filters.end_date {
        let next = d + Duration::days(1);
        sql.push_str(" AND e.occurred_at < ?");
        params.push(Box::new(next.with_time(Time::MIDNIGHT).assume_offset(tz)));
    }
    if let Some(s) = filters.search.as_ref().filter(|s| !s.trim().is_empty()) {
        sql.push_str(" AND (e.description LIKE ? OR e.raw_message LIKE ?)");
        let pattern = format!("%{}%", s.trim());
        params.push(Box::new(pattern.clone()));
        params.push(Box::new(pattern));
    }
    sql.push_str(" ORDER BY e.occurred_at DESC, e.id DESC LIMIT ? OFFSET ?");
    params.push(Box::new(limit));
    params.push(Box::new(offset));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            let kind: Option<CategoryKind> = r.get(5)?;
            Ok(LedgerRow {
                id: r.get(0)?,
                amount_cents: r.get(1)?,
                currency: r.get(2)?,
                category_id: r.get(3)?,
                category_name: r.get(4)?,
                category_kind: kind.map(|k| k.as_str().to_string()),
                description: r.get(6)?,
                occurred_at: r.get(7)?,
                source: r.get::<_, ExpenseSource>(8)?.as_str().to_string(),
                logged_by_chat_id: r.get(9)?,
                logged_by_name: r.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[tauri::command]
pub async fn list_expenses(
    filters: ExpenseFilters,
    state: State<'_, AppState>,
) -> Result<Vec<LedgerRow>, String> {
    let now = OffsetDateTime::now_utc();
    let tz = now.offset();
    state
        .db_actor
        .run(move |conn| list_expenses_query(conn, &filters, tz))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn delete_expense(id: i64, state: State<'_, AppState>) -> Result<bool, String> {
    state
        .db_actor
        .run(move |conn| expenses::delete(conn, id))
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// Categories (CRUD beyond setup).
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct NewCategoryArg {
    pub name: String,
    pub kind: String, // "fixed" | "variable"
    #[serde(default)]
    pub monthly_target_cents: Option<i64>,
    #[serde(default)]
    pub is_recurring: bool,
    #[serde(default)]
    pub recurrence_day_of_month: Option<u8>,
}

#[tauri::command]
pub async fn create_category(
    arg: NewCategoryArg,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let kind: CategoryKind = arg.kind.parse().map_err(err)?;
    state
        .db_actor
        .run(move |conn| {
            categories::insert(
                conn,
                &NewCategory {
                    name: arg.name,
                    kind,
                    monthly_target_cents: arg.monthly_target_cents,
                    is_recurring: arg.is_recurring,
                    recurrence_day_of_month: arg.recurrence_day_of_month,
                },
            )
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn delete_category(id: i64, state: State<'_, AppState>) -> Result<bool, String> {
    state
        .db_actor
        .run(move |conn| categories::delete(conn, id))
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// Household.
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn remove_household_member(
    chat_id: i64,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    state
        .db_actor
        .run(move |conn| auth::remove_member(conn, chat_id))
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// Background mode + autostart.
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn get_run_in_background(state: State<'_, AppState>) -> Result<bool, String> {
    state
        .db_actor
        .run(|conn| {
            Ok(settings::get(conn, settings::keys::RUN_IN_BACKGROUND)?.as_deref() != Some("0"))
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_run_in_background(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db_actor
        .run(move |conn| {
            settings::set(
                conn,
                settings::keys::RUN_IN_BACKGROUND,
                if enabled { "1" } else { "0" },
            )
        })
        .await
        .map_err(err)
}

/// Default in micros (1e-6 USD) — keep in sync with
/// `telegram/router.rs::DEFAULT_DAILY_COST_CAP_MICROS`. The setting is
/// stored as the raw integer; the UI talks dollars.
const DEFAULT_DAILY_COST_CAP_MICROS: i64 = 1_000_000;

#[tauri::command]
pub async fn get_llm_daily_cost_cap_micros(state: State<'_, AppState>) -> Result<i64, String> {
    state
        .db_actor
        .run(|conn| {
            let raw = settings::get(conn, settings::keys::LLM_DAILY_COST_CAP_MICROS)?;
            Ok(raw
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(DEFAULT_DAILY_COST_CAP_MICROS))
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_llm_daily_cost_cap_micros(
    cap_micros: i64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Negative caps are accepted and mean "disabled"; storing them as-is
    // keeps the disable semantics observable in the DB. We DO reject
    // absurd positive values to catch obvious UI input mistakes.
    if cap_micros > 10_000_000_000 {
        return Err("Cap is unreasonably large (max $10,000/day)".into());
    }
    state
        .db_actor
        .run(move |conn| {
            settings::set(
                conn,
                settings::keys::LLM_DAILY_COST_CAP_MICROS,
                &cap_micros.to_string(),
            )
        })
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// AI Report Wizard (v0.4.0). Deterministic figures come from
// `insights::report`; the LLM only narrates them. Cost is logged to
// `llm_usage` and gated by the same daily cap as the bot.
// ---------------------------------------------------------------------

use crate::insights::report::{build_report_data, ReportData};
use crate::llm::report as report_writer;
use report_writer::{ReportRequest, REPORT_ANTHROPIC_MODEL};

/// What model + provider a report would use, given current settings.
/// Anthropic forces Sonnet; Ollama uses the configured local model
/// (unpriced → zero estimated cost).
fn report_model(provider_choice: &str, ollama_model: &str) -> (String, String) {
    if provider_choice == "ollama" {
        ("ollama".to_string(), ollama_model.to_string())
    } else {
        ("anthropic".to_string(), REPORT_ANTHROPIC_MODEL.to_string())
    }
}

#[derive(Serialize)]
pub struct ReportEstimate {
    pub provider: String,
    pub model: String,
    pub estimate_micros: i64,
    pub today_micros: i64,
    pub cap_micros: i64,
    /// True when the cap is enabled (>= 0) and today's spend plus the
    /// estimate would cross it. The UI warns but still lets the user
    /// proceed; the hard refusal is server-side in `report_generate`.
    pub would_exceed_cap: bool,
}

struct ReportCtx {
    data: ReportData,
    provider_choice: String,
    ollama_model: String,
    ollama_endpoint: String,
    today_micros: i64,
    cap_micros: i64,
}

fn load_report_ctx(
    conn: &Connection,
    req: &ReportRequest,
    now: OffsetDateTime,
) -> anyhow::Result<ReportCtx> {
    let data = build_report_data(
        conn,
        req.timeframe,
        now,
        req.monthly_income_cents,
        req.include_merchant_samples,
    )?;
    let provider_choice =
        settings::get(conn, settings::keys::LLM_PROVIDER)?.unwrap_or_else(|| "anthropic".into());
    let ollama_model = settings::get_or_default(conn, settings::keys::OLLAMA_MODEL, "llama3:8b")?;
    let ollama_endpoint = settings::get_or_default(
        conn,
        settings::keys::OLLAMA_ENDPOINT,
        "http://localhost:11434",
    )?;
    let cap_micros = settings::get(conn, settings::keys::LLM_DAILY_COST_CAP_MICROS)?
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(DEFAULT_DAILY_COST_CAP_MICROS);
    let today_micros = llm_usage::today_cost_micros(conn, now)?;
    Ok(ReportCtx {
        data,
        provider_choice,
        ollama_model,
        ollama_endpoint,
        today_micros,
        cap_micros,
    })
}

#[tauri::command]
pub async fn report_estimate(
    input: ReportRequest,
    state: State<'_, AppState>,
) -> Result<ReportEstimate, String> {
    let now = OffsetDateTime::now_utc();
    let req = input.clone();
    let ctx = state
        .db_actor
        .run(move |conn| load_report_ctx(conn, &req, now))
        .await
        .map_err(err)?;

    let (provider, model) = report_model(&ctx.provider_choice, &ctx.ollama_model);
    let (prompt, max_tokens) = report_writer::preview_prompt(&ctx.data, &input);
    let estimate_micros = report_writer::estimate_micros(&model, &prompt, max_tokens);
    let would_exceed_cap =
        ctx.cap_micros >= 0 && ctx.today_micros + estimate_micros > ctx.cap_micros;

    Ok(ReportEstimate {
        provider,
        model,
        estimate_micros,
        today_micros: ctx.today_micros,
        cap_micros: ctx.cap_micros,
        would_exceed_cap,
    })
}

#[derive(Serialize)]
pub struct GeneratedReportResponse {
    pub report: report_writer::GeneratedReport,
    /// The deterministic figures the narrative is built on — the UI
    /// renders these as the authoritative numbers alongside the prose.
    pub figures: serde_json::Value,
    pub provider: String,
    pub model: String,
    pub cost_micros: i64,
    pub generated_at: String,
    pub timeframe_label: String,
}

#[tauri::command]
pub async fn report_generate(
    input: ReportRequest,
    state: State<'_, AppState>,
) -> Result<GeneratedReportResponse, String> {
    let now = OffsetDateTime::now_utc();
    let req = input.clone();
    let ctx = state
        .db_actor
        .run(move |conn| load_report_ctx(conn, &req, now))
        .await
        .map_err(err)?;

    // Hard daily-cap refusal, consistent with the bot's per-turn check:
    // refuse only once already at/over the cap (we can't know the exact
    // cost before the call).
    if ctx.cap_micros >= 0 && ctx.today_micros >= ctx.cap_micros {
        return Err(format!(
            "Daily AI cost cap reached ({}/{} used today). Raise it in Settings or try tomorrow.",
            crate::llm::pricing::format_micros_usd(ctx.today_micros),
            crate::llm::pricing::format_micros_usd(ctx.cap_micros),
        ));
    }

    let (provider_name, model) = report_model(&ctx.provider_choice, &ctx.ollama_model);

    let (report, usage) = if ctx.provider_choice == "ollama" {
        let provider =
            OllamaProvider::with_base_url(ctx.ollama_model.clone(), ctx.ollama_endpoint.clone())
                .map_err(err)?;
        let (r, u, _m) = report_writer::generate(&provider, &ctx.data, &input)
            .await
            .map_err(err)?;
        (r, u)
    } else {
        let key = secrets::retrieve(secrets::keys::ANTHROPIC_API_KEY)
            .map_err(err)?
            .ok_or_else(|| "no Anthropic API key saved".to_string())?;
        let provider = AnthropicProvider::with_options(
            key,
            REPORT_ANTHROPIC_MODEL,
            "https://api.anthropic.com",
        )
        .map_err(err)?;
        let (r, u, _m) = report_writer::generate(&provider, &ctx.data, &input)
            .await
            .map_err(err)?;
        (r, u)
    };

    let cost_micros = crate::llm::pricing::compute_cost_micros(&model, &usage).unwrap_or(0);

    // Log to llm_usage so report spend shows in Settings and counts
    // toward the daily cap, exactly like the bot's calls. Best-effort.
    {
        let pn = provider_name.clone();
        let m = model.clone();
        let _ = state
            .db_actor
            .run(move |conn| llm_usage::log(conn, &pn, &m, &usage, now))
            .await;
    }

    let generated_at = now
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| now.unix_timestamp().to_string());

    Ok(GeneratedReportResponse {
        figures: serde_json::to_value(&ctx.data).unwrap_or(serde_json::Value::Null),
        timeframe_label: ctx.data.timeframe_label.clone(),
        report,
        provider: provider_name,
        model,
        cost_micros,
        generated_at,
    })
}

/// Persist a client-rendered PDF (base64) to a user-chosen path. The
/// path comes from the dialog plugin's save picker; writing bytes
/// through a command keeps the only filesystem touch inside the typed
/// boundary rather than granting the webview a broad `fs` capability.
#[tauri::command]
pub async fn report_save_pdf(path: String, base64_pdf: String) -> Result<(), String> {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    if path.trim().is_empty() {
        return Err("no save path provided".into());
    }
    let bytes = B64.decode(base64_pdf.as_bytes()).map_err(err)?;
    std::fs::write(&path, bytes).map_err(|e| format!("writing {path}: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn get_ollama_allow_remote(state: State<'_, AppState>) -> Result<bool, String> {
    state
        .db_actor
        .run(|conn| {
            Ok(settings::get(conn, settings::keys::OLLAMA_ALLOW_REMOTE)?.as_deref() == Some("1"))
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_ollama_allow_remote(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db_actor
        .run(move |conn| {
            settings::set(
                conn,
                settings::keys::OLLAMA_ALLOW_REMOTE,
                if enabled { "1" } else { "0" },
            )
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(err)
}

#[tauri::command]
pub async fn set_autostart(
    enabled: bool,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(err)?;
    } else {
        manager.disable().map_err(err)?;
    }
    state
        .db_actor
        .run(move |conn| {
            settings::set(
                conn,
                settings::keys::AUTOSTART,
                if enabled { "1" } else { "0" },
            )
        })
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// Auto-update (tauri-plugin-updater).
// ---------------------------------------------------------------------

/// Result of `check_for_update`. `available = false` means the user is
/// already on the latest version. When available, the frontend uses
/// `version` + `notes` to populate the in-app banner.
#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub version: Option<String>,
    pub current_version: String,
    pub notes: Option<String>,
}

#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    use tauri_plugin_updater::UpdaterExt;
    let current_version = app.package_info().version.to_string();
    let updater = app.updater().map_err(err)?;
    match updater.check().await.map_err(err)? {
        Some(u) => Ok(UpdateInfo {
            available: true,
            version: Some(u.version.clone()),
            current_version,
            notes: u.body.clone(),
        }),
        None => Ok(UpdateInfo {
            available: false,
            version: None,
            current_version,
            notes: None,
        }),
    }
}

/// Returns the running app version, sourced from the Tauri package info.
/// Used by the Settings → About section to display "v0.X.Y" alongside the
/// AGPL source-offer notice.
#[tauri::command]
pub fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Download and install the pending update, then relaunch. Errors if no
/// update is currently available (call `check_for_update` first).
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(err)?;
    let update = updater
        .check()
        .await
        .map_err(err)?
        .ok_or_else(|| "no update available".to_string())?;
    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(err)?;
    app.restart();
}

#[tauri::command]
pub async fn get_check_updates_on_launch(state: State<'_, AppState>) -> Result<bool, String> {
    state
        .db_actor
        .run(|conn| {
            Ok(
                settings::get(conn, settings::keys::CHECK_UPDATES_ON_LAUNCH)?.as_deref()
                    != Some("0"),
            )
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_check_updates_on_launch(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db_actor
        .run(move |conn| {
            settings::set(
                conn,
                settings::keys::CHECK_UPDATES_ON_LAUNCH,
                if enabled { "1" } else { "0" },
            )
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn get_weekly_summary_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    state
        .db_actor
        .run(|conn| {
            Ok(
                settings::get(conn, settings::keys::WEEKLY_SUMMARY_ENABLED)?.as_deref()
                    != Some("0"),
            )
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_weekly_summary_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db_actor
        .run(move |conn| {
            settings::set(
                conn,
                settings::keys::WEEKLY_SUMMARY_ENABLED,
                if enabled { "1" } else { "0" },
            )
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn get_budget_alerts_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    state
        .db_actor
        .run(|conn| {
            Ok(settings::get(conn, settings::keys::BUDGET_ALERTS_ENABLED)?.as_deref() != Some("0"))
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_budget_alerts_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db_actor
        .run(move |conn| {
            settings::set(
                conn,
                settings::keys::BUDGET_ALERTS_ENABLED,
                if enabled { "1" } else { "0" },
            )
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn get_llm_usage_summary(
    state: State<'_, AppState>,
) -> Result<llm_usage::UsageSummary, String> {
    let now = OffsetDateTime::now_utc();
    state
        .db_actor
        .run(move |conn| llm_usage::summary(conn, now))
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// Forecast tools (v0.3.0).
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct CategoryStatsResponse {
    pub category_id: i64,
    pub months_back: u32,
    pub stats: Option<DescriptiveStats>,
    pub histogram: Option<Histogram>,
    pub monthly_totals_cents: Vec<i64>,
}

#[tauri::command]
pub async fn get_category_stats(
    category_id: i64,
    months_back: u32,
    state: State<'_, AppState>,
) -> Result<CategoryStatsResponse, String> {
    let now = OffsetDateTime::now_utc();
    let n = months_back.clamp(1, 120);
    let totals = state
        .db_actor
        .run(move |conn| expenses::monthly_totals_for_category(conn, category_id, now, n))
        .await
        .map_err(err)?;
    let stats = stats_mod::describe(&totals);
    let histogram = stats_mod::histogram(&totals, 10);
    Ok(CategoryStatsResponse {
        category_id,
        months_back: n,
        stats,
        histogram,
        monthly_totals_cents: totals,
    })
}

#[derive(Debug, Deserialize)]
pub struct ScenarioInput {
    pub cuts: Vec<ScenarioCut>,
}

#[tauri::command]
pub async fn run_scenario(
    input: ScenarioInput,
    state: State<'_, AppState>,
) -> Result<ScenarioResult, String> {
    let rows: Vec<(i64, i64)> = state
        .db_actor
        .run(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, COALESCE(monthly_target_cents, 0)
                 FROM categories
                 WHERE kind = 'variable' AND is_active = 1
                   AND monthly_target_cents IS NOT NULL
                   AND monthly_target_cents > 0",
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
        .map_err(err)?;
    Ok(forecast_mod::scenario_delta(&rows, &input.cuts))
}

#[derive(Debug, Deserialize)]
pub struct SetStartingBalanceInput {
    pub category_id: i64,
    /// In cents. None clears the existing value.
    pub starting_balance_cents: Option<i64>,
    /// ISO date YYYY-MM-DD.
    pub balance_as_of: Option<String>,
}

#[tauri::command]
pub async fn set_starting_balance(
    input: SetStartingBalanceInput,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db_actor
        .run(move |conn| {
            categories::set_starting_balance(
                conn,
                input.category_id,
                input.starting_balance_cents,
                input.balance_as_of.as_deref(),
            )?;
            Ok(())
        })
        .await
        .map_err(err)
}

#[derive(Debug, Serialize)]
pub struct InvestmentSummary {
    pub category_id: i64,
    pub name: String,
    pub starting_balance_cents: Option<i64>,
    pub balance_as_of: Option<String>,
    /// Average monthly contribution computed over the last 12 months
    /// of logged history. None when there are zero contributions.
    pub avg_monthly_contribution_cents: Option<i64>,
    /// Total contributed over the last 12 months.
    pub last_12mo_contribution_cents: i64,
}

#[tauri::command]
pub async fn list_investment_categories(
    state: State<'_, AppState>,
) -> Result<Vec<InvestmentSummary>, String> {
    let now = OffsetDateTime::now_utc();
    state
        .db_actor
        .run(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, starting_balance_cents, balance_as_of
                 FROM categories
                 WHERE kind = 'investing' AND is_active = 1
                 ORDER BY name ASC",
            )?;
            let cats = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            let mut out = Vec::with_capacity(cats.len());
            for (id, name, balance, as_of) in cats {
                let totals = expenses::monthly_totals_for_category(conn, id, now, 12)?;
                let total_12mo: i64 = totals.iter().sum();
                let avg = if totals.is_empty() {
                    None
                } else {
                    Some(total_12mo / totals.len() as i64)
                };
                out.push(InvestmentSummary {
                    category_id: id,
                    name,
                    starting_balance_cents: balance,
                    balance_as_of: as_of,
                    avg_monthly_contribution_cents: avg,
                    last_12mo_contribution_cents: total_12mo,
                });
            }
            Ok(out)
        })
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// Forecast wave 2 — Monte Carlo bands, simulator, category analyzer.
// (v0.3.3)
// ---------------------------------------------------------------------

use crate::insights::category_analyzer::{
    self as cat_analyzer_mod, AnalysisWindow, CategoryAnalysis,
};
use crate::insights::simulator::{
    self as simulator_mod, HeatmapInput, HeatmapResult, ProbabilityInput, ProbabilityResult,
    RequiredContributionInput, RequiredContributionResult,
};

#[tauri::command]
pub async fn simulator_solve_required_contribution(
    input: RequiredContributionInput,
) -> Result<RequiredContributionResult, String> {
    Ok(simulator_mod::solve_required_contribution(&input))
}

#[tauri::command]
pub async fn simulator_compute_probability(
    input: ProbabilityInput,
) -> Result<ProbabilityResult, String> {
    Ok(simulator_mod::compute_probability(&input))
}

#[tauri::command]
pub async fn simulator_heatmap(input: HeatmapInput) -> Result<HeatmapResult, String> {
    Ok(simulator_mod::heatmap(&input))
}

#[tauri::command]
pub async fn analyze_category(
    category_id: i64,
    window: AnalysisWindow,
    state: State<'_, AppState>,
) -> Result<CategoryAnalysis, String> {
    state
        .db_actor
        .run(move |conn| {
            cat_analyzer_mod::analyze(conn, category_id, window, OffsetDateTime::now_utc())
        })
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// Debt management (v0.3.7).
//
// Pure calculator: no DB reads. The frontend gathers all inputs and
// receives a deterministic month-by-month schedule plus summary stats.
// Goal-seek bisects the monthly payment; portfolio mode handles
// snowball / avalanche distribution of a fixed monthly budget.
// ---------------------------------------------------------------------

use crate::insights::debt::{
    self as debt_mod, GoalSeekInput, GoalSeekResult, PortfolioInput, PortfolioResult,
    ScheduleInput, ScheduleResult,
};

#[tauri::command]
pub async fn debt_simulate_schedule(input: ScheduleInput) -> Result<ScheduleResult, String> {
    Ok(debt_mod::simulate_schedule(&input))
}

#[tauri::command]
pub async fn debt_goal_seek(input: GoalSeekInput) -> Result<GoalSeekResult, String> {
    Ok(debt_mod::goal_seek(&input))
}

#[tauri::command]
pub async fn debt_simulate_portfolio(input: PortfolioInput) -> Result<PortfolioResult, String> {
    Ok(debt_mod::simulate_portfolio(&input))
}

// ---------------------------------------------------------------------
// CSV import (v0.3.2).
//
// All bulk-import paths from bank/CC CSV exports go through these
// handlers. The frontend wizard drives the flow; the backend hosts
// parsing, deduplication, rules-based categorization, and the optional
// batched LLM suggest call. See `Patches/v0.3.2.md` and
// `src/csv_import/` for the architecture.
// ---------------------------------------------------------------------

use crate::csv_import::{
    ai_suggest::{self, CategoryHint},
    categorize::{self as categorize_mod, Decision},
    dedupe::{self, DuplicateMatch},
    parser::{self as csv_parser, ParsedRow, PreviewResult},
};
use crate::repository::{
    csv_import_profiles::{self as csv_profiles, ColumnMapping, CsvImportProfile},
    merchant_rules::{self, MerchantRule},
};
/// Preview payload for the wizard's first screen.
#[derive(Debug, Serialize)]
pub struct CsvPreview {
    pub preview: PreviewResult,
    /// Profile auto-suggested via header_signature match, if any.
    pub suggested_profile: Option<CsvImportProfile>,
    /// All saved profiles for the manual dropdown.
    pub profiles: Vec<CsvImportProfile>,
}

#[tauri::command]
pub async fn csv_import_preview(
    content: String,
    state: State<'_, AppState>,
) -> Result<CsvPreview, String> {
    let preview = csv_parser::parse_preview(&content).map_err(err)?;
    let header_signature = preview.header_signature.clone();
    let (suggested, profiles) = state
        .db_actor
        .run(move |conn| {
            let suggested = csv_profiles::find_by_signature(conn, &header_signature)?;
            let profiles = csv_profiles::list(conn)?;
            Ok((suggested, profiles))
        })
        .await
        .map_err(err)?;
    Ok(CsvPreview {
        preview,
        suggested_profile: suggested,
        profiles,
    })
}

#[derive(Debug, Deserialize)]
pub struct SaveProfileInput {
    pub name: String,
    pub header_signature: Option<String>,
    pub mapping: ColumnMapping,
}

#[tauri::command]
pub async fn csv_import_save_profile(
    input: SaveProfileInput,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let now = OffsetDateTime::now_utc();
    state
        .db_actor
        .run(move |conn| {
            csv_profiles::create(
                conn,
                &input.name,
                input.header_signature.as_deref(),
                &input.mapping,
                now,
            )
        })
        .await
        .map_err(err)
}

#[derive(Debug, Deserialize)]
pub struct ParseInput {
    pub content: String,
    pub mapping: ColumnMapping,
}

/// Apply the user-confirmed mapping to the file and return the parsed
/// rows. The wizard then runs categorize / dedupe.
#[tauri::command]
pub async fn csv_import_parse(input: ParseInput) -> Result<Vec<ParsedRow>, String> {
    csv_parser::parse_with_mapping(&input.content, &input.mapping).map_err(err)
}

#[derive(Debug, Serialize)]
pub struct CategorizeAndDedupeResult {
    pub decisions: Vec<Decision>,
    pub duplicates: Vec<DuplicateMatch>,
}

#[derive(Debug, Deserialize)]
pub struct CategorizeInput {
    pub rows: Vec<ParsedRow>,
}

#[tauri::command]
pub async fn csv_import_categorize_and_dedupe(
    input: CategorizeInput,
    state: State<'_, AppState>,
) -> Result<CategorizeAndDedupeResult, String> {
    state
        .db_actor
        .run(move |conn| {
            let decisions = categorize_mod::categorize_all(conn, &input.rows)?;
            let duplicates = dedupe::find_probable_duplicates(conn, &input.rows)?;
            Ok(CategorizeAndDedupeResult {
                decisions,
                duplicates,
            })
        })
        .await
        .map_err(err)
}

#[derive(Debug, Deserialize)]
pub struct AiSuggestInput {
    /// Distinct merchant strings to categorize.
    pub merchants: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AiSuggestResponse {
    pub suggestions: std::collections::HashMap<String, i64>,
    pub cost_micros: i64,
}

#[tauri::command]
pub async fn csv_import_ai_suggest(
    input: AiSuggestInput,
    state: State<'_, AppState>,
) -> Result<AiSuggestResponse, String> {
    // Build a category-hint list from the user's active categories.
    let (hints, provider_choice, anth_model, ollama_endpoint, ollama_model): (
        Vec<CategoryHint>,
        String,
        String,
        String,
        String,
    ) = state
        .db_actor
        .run(|conn| {
            let cats = categories::list(conn, false)?;
            let hints: Vec<CategoryHint> = cats
                .into_iter()
                .filter(|c| c.is_active)
                .map(|c| CategoryHint {
                    id: c.id,
                    name: c.name,
                    hint: Some(format!("{:?}", c.kind).to_lowercase()),
                })
                .collect();
            let provider_choice = settings::get(conn, settings::keys::LLM_PROVIDER)?
                .unwrap_or_else(|| "anthropic".into());
            let anth_model = settings::get_or_default(
                conn,
                settings::keys::ANTHROPIC_MODEL,
                DEFAULT_ANTHROPIC_MODEL,
            )?;
            let ollama_endpoint = settings::get_or_default(
                conn,
                settings::keys::OLLAMA_ENDPOINT,
                "http://localhost:11434",
            )?;
            let ollama_model =
                settings::get_or_default(conn, settings::keys::OLLAMA_MODEL, "llama3:8b")?;
            Ok((
                hints,
                provider_choice,
                anth_model,
                ollama_endpoint,
                ollama_model,
            ))
        })
        .await
        .map_err(err)?;

    let result = if provider_choice == "ollama" {
        let provider = OllamaProvider::with_base_url(ollama_model, ollama_endpoint).map_err(err)?;
        ai_suggest::suggest_categories(&provider, &input.merchants, &hints)
            .await
            .map_err(err)?
    } else {
        let key = secrets::retrieve(secrets::keys::ANTHROPIC_API_KEY)
            .map_err(err)?
            .ok_or_else(|| "no Anthropic API key saved".to_string())?;
        let provider =
            AnthropicProvider::with_options(key, &anth_model, "https://api.anthropic.com")
                .map_err(err)?;
        ai_suggest::suggest_categories(&provider, &input.merchants, &hints)
            .await
            .map_err(err)?
    };
    // (Cost surfaces in the wizard summary; we don't double-write to
    // llm_usage from here because suggest_categories already returned
    // the cost_micros and the underlying provider call wasn't routed
    // through the bot's usage-logging path.)
    Ok(AiSuggestResponse {
        suggestions: result.suggestions,
        cost_micros: result.cost_micros,
    })
}

/// One row the user has decided to commit. The frontend assembles these
/// after the review screens; the backend trusts the categorization and
/// just inserts.
#[derive(Debug, Deserialize)]
pub struct CommittableRow {
    pub occurred_at: String,
    pub amount_cents: i64,
    pub category_id: Option<i64>,
    pub merchant: String,
    pub description: Option<String>,
    pub is_refund: bool,
}

/// Pattern-and-category pair to persist as a new merchant_rules row.
#[derive(Debug, Deserialize)]
pub struct RuleToSave {
    pub pattern: String,
    pub category_id: i64,
    pub default_is_refund: bool,
}

#[derive(Debug, Deserialize)]
pub struct CommitInput {
    pub rows: Vec<CommittableRow>,
    pub rules_to_save: Vec<RuleToSave>,
    pub profile_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CommitResult {
    pub inserted: usize,
    pub rules_added: usize,
}

/// Plain helper version of `csv_import_commit` so tests can exercise
/// the transaction logic (insert loop + rule writes + profile touch)
/// without going through Tauri's State<> wrapper. Audit T-1.
pub fn csv_import_commit_inner(
    conn: &mut Connection,
    input: &CommitInput,
    now: OffsetDateTime,
) -> anyhow::Result<CommitResult> {
    let currency = settings::get_or_default(conn, settings::keys::DEFAULT_CURRENCY, "USD")?;
    // Wrap the entire commit (expense inserts + merchant-rule writes
    // + profile touch) in a single transaction so a mid-batch failure
    // (bogus date parse, FK violation, rusqlite Busy on WAL
    // contention) rolls back cleanly instead of leaving a partial
    // import on disk.
    let tx = conn.unchecked_transaction()?;

    let mut inserted = 0usize;
    for row in &input.rows {
        let occurred_at = OffsetDateTime::parse(
            &row.occurred_at,
            &time::format_description::well_known::Rfc3339,
        )?;
        expenses::insert(
            &tx,
            &crate::domain::NewExpense {
                amount_cents: row.amount_cents,
                currency: currency.clone(),
                category_id: row.category_id,
                description: row
                    .description
                    .clone()
                    .or_else(|| Some(row.merchant.clone())),
                occurred_at,
                source: ExpenseSource::Csv,
                raw_message: Some(row.merchant.clone()),
                llm_confidence: None,
                logged_by_chat_id: None,
                is_refund: row.is_refund,
                refund_for_expense_id: None,
            },
        )?;
        inserted += 1;
    }

    let mut rules_added = 0usize;
    for rule in &input.rules_to_save {
        merchant_rules::create(
            &tx,
            &rule.pattern,
            rule.category_id,
            rule.default_is_refund,
            0,
            now,
        )?;
        rules_added += 1;
    }

    if let Some(id) = input.profile_id {
        let _ = csv_profiles::touch(&tx, id, now);
    }

    tx.commit()?;
    Ok(CommitResult {
        inserted,
        rules_added,
    })
}

#[tauri::command]
pub async fn csv_import_commit(
    input: CommitInput,
    state: State<'_, AppState>,
) -> Result<CommitResult, String> {
    let now = OffsetDateTime::now_utc();
    state
        .db_actor
        .run(move |conn| csv_import_commit_inner(conn, &input, now))
        .await
        .map_err(err)
}

// --- Settings management for profiles + rules ----------------------

#[tauri::command]
pub async fn list_csv_import_profiles(
    state: State<'_, AppState>,
) -> Result<Vec<CsvImportProfile>, String> {
    state
        .db_actor
        .run(|conn| csv_profiles::list(conn))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn delete_csv_import_profile(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    state
        .db_actor
        .run(move |conn| {
            csv_profiles::delete(conn, id)?;
            Ok(())
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn list_merchant_rules(state: State<'_, AppState>) -> Result<Vec<MerchantRule>, String> {
    state
        .db_actor
        .run(|conn| merchant_rules::list(conn))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn delete_merchant_rule(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    state
        .db_actor
        .run(move |conn| {
            merchant_rules::delete(conn, id)?;
            Ok(())
        })
        .await
        .map_err(err)
}

// ---------------------------------------------------------------------
// Data export — the "open hub" (v0.5.0).
//
// A complete, faithful one-way photocopy of the ledger. Reads only the
// stable `v_ledger_v1` view; snapshot, never a live feed; desktop-only,
// no bot path. The frontend's save dialog supplies `path`; we write the
// bytes through this command so the only filesystem touch stays inside
// the typed boundary (same pattern as `report_save_pdf`).
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ExportDataInput {
    pub path: String,
    /// "csv" | "jsonl" | "beancount"
    pub format: String,
    /// "all" | "month" | "quarter" | "year"
    pub timeframe: String,
}

#[derive(Debug, Serialize)]
pub struct ExportSummary {
    pub row_count: usize,
    pub byte_count: usize,
    pub path: String,
}

#[tauri::command]
pub async fn export_data(
    input: ExportDataInput,
    state: State<'_, AppState>,
) -> Result<ExportSummary, String> {
    use crate::export::{self, ExportFormat, ExportTimeframe};

    if input.path.trim().is_empty() {
        return Err("no save path provided".into());
    }
    let format: ExportFormat = input.format.parse().map_err(err)?;
    let timeframe: ExportTimeframe = input.timeframe.parse().map_err(err)?;
    let now = OffsetDateTime::now_utc();

    let result = state
        .db_actor
        .run(move |conn| export::export(conn, format, timeframe, now))
        .await
        .map_err(err)?;

    std::fs::write(&input.path, &result.bytes)
        .map_err(|e| format!("writing {}: {e}", input.path))?;

    Ok(ExportSummary {
        row_count: result.row_count,
        byte_count: result.bytes.len(),
        path: input.path,
    })
}

// ---------------------------------------------------------------------
// Misc.
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn ping() -> Result<String, String> {
    Ok("pong".into())
}
