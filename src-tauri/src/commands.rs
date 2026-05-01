//! Tauri IPC commands exposed to the React frontend.
//!
//! Each `#[tauri::command]` is the only way the frontend can touch the
//! database, secrets, or network — keep these handlers small and route
//! into the typed library code, never inline business logic here.

use serde::{Deserialize, Serialize};
use tauri::State;
use time::{Date, Duration, OffsetDateTime, Time};

use crate::app_state::AppState;
use crate::domain::{CategoryKind, ExpenseSource, NewCategory};
use crate::insights::{
    dashboard,
    forecast::{
        self as forecast_mod, GoalSeekInput, GoalSeekResult, InvestmentProjection,
        ProjectInvestmentInput, ScenarioCut, ScenarioResult,
    },
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
    let conn = state.db.lock().unwrap();
    let setup_complete = settings::get(&conn, settings::keys::SETUP_COMPLETE)
        .map_err(err)?
        .as_deref()
        == Some("1");
    let last_completed_step = settings::get(&conn, settings::keys::SETUP_STEP)
        .map_err(err)?
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(0);
    let llm_provider = settings::get(&conn, settings::keys::LLM_PROVIDER).map_err(err)?;
    let default_currency =
        settings::get_or_default(&conn, settings::keys::DEFAULT_CURRENCY, "USD").map_err(err)?;
    let locale = settings::get(&conn, settings::keys::LOCALE).map_err(err)?;
    let ollama_endpoint = settings::get(&conn, settings::keys::OLLAMA_ENDPOINT).map_err(err)?;
    let ollama_model = settings::get(&conn, settings::keys::OLLAMA_MODEL).map_err(err)?;

    let authorized_chat_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM telegram_authorized_chats", [], |r| {
            r.get(0)
        })
        .map_err(err)?;

    let anthropic_key_set = secrets::exists(secrets::keys::ANTHROPIC_API_KEY).map_err(err)?;
    let telegram_token_set = secrets::exists(secrets::keys::TELEGRAM_BOT_TOKEN).map_err(err)?;

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
}

#[tauri::command]
pub async fn set_setup_step(step: u8, state: State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    settings::set(&conn, settings::keys::SETUP_STEP, &step.to_string()).map_err(err)
}

// ---------------------------------------------------------------------
// LLM provider selection + config.
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn save_llm_provider(provider: String, state: State<'_, AppState>) -> Result<(), String> {
    if provider != "anthropic" && provider != "ollama" {
        return Err(format!("invalid provider: {provider}"));
    }
    let conn = state.db.lock().unwrap();
    settings::set(&conn, settings::keys::LLM_PROVIDER, &provider).map_err(err)
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
    let model = {
        let conn = state.db.lock().unwrap();
        settings::get_or_default(
            &conn,
            settings::keys::ANTHROPIC_MODEL,
            DEFAULT_ANTHROPIC_MODEL,
        )
        .map_err(err)?
    };
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
    let conn = state.db.lock().unwrap();
    settings::set(&conn, settings::keys::OLLAMA_ENDPOINT, &endpoint).map_err(err)?;
    settings::set(&conn, settings::keys::OLLAMA_MODEL, &model).map_err(err)?;
    Ok(())
}

#[tauri::command]
pub async fn list_ollama_models(endpoint: String) -> Result<Vec<String>, String> {
    let url = format!("{}/api/tags", endpoint.trim_end_matches('/'));
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
    let (endpoint, model) = {
        let conn = state.db.lock().unwrap();
        let endpoint = settings::get_or_default(
            &conn,
            settings::keys::OLLAMA_ENDPOINT,
            "http://localhost:11434",
        )
        .map_err(err)?;
        let model = settings::get_or_default(&conn, settings::keys::OLLAMA_MODEL, "llama3:8b")
            .map_err(err)?;
        (endpoint, model)
    };
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
    state: State<'_, AppState>,
) -> Result<String, String> {
    let conn = state.db.lock().unwrap();
    auth::generate_pairing_code(&conn, &display_name, time::OffsetDateTime::now_utc()).map_err(err)
}

#[tauri::command]
pub async fn list_authorized_chats(
    state: State<'_, AppState>,
) -> Result<Vec<AuthorizedChat>, String> {
    let conn = state.db.lock().unwrap();
    auth::list_members(&conn).map_err(err)
}

/// Wipe every authorized chat and pending pairing code. Used by the
/// Settings UI's "factory reset" toggle when rotating the bot token.
/// Returns the number of authorized chats that were removed.
#[tauri::command]
pub async fn clear_authorized_chats(state: State<'_, AppState>) -> Result<usize, String> {
    let conn = state.db.lock().unwrap();
    auth::clear_all(&conn).map_err(err)
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
    let conn = state.db.lock().unwrap();
    settings::set(&conn, settings::keys::DEFAULT_CURRENCY, &currency).map_err(err)?;
    settings::set(&conn, settings::keys::LOCALE, &locale).map_err(err)?;
    Ok(())
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
    let conn = state.db.lock().unwrap();
    let cats = categories::list(&conn, include_inactive).map_err(err)?;
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
    let conn = state.db.lock().unwrap();
    categories::set_monthly_target(&conn, id, monthly_target_cents).map_err(err)?;
    Ok(())
}

#[tauri::command]
pub async fn set_category_active(
    id: i64,
    is_active: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    categories::set_active(&conn, id, is_active).map_err(err)?;
    Ok(())
}

// ---------------------------------------------------------------------
// Finalize.
// ---------------------------------------------------------------------

#[tauri::command]
#[allow(clippy::collapsible_match)]
pub async fn finalize_setup(state: State<'_, AppState>) -> Result<(), String> {
    {
        let conn = state.db.lock().unwrap();
        // Sanity: refuse to finalize if prerequisites are missing.
        let provider = settings::get(&conn, settings::keys::LLM_PROVIDER)
            .map_err(err)?
            .ok_or_else(|| "pick an LLM provider first".to_string())?;
        match provider.as_str() {
            "anthropic" => {
                if !secrets::exists(secrets::keys::ANTHROPIC_API_KEY).map_err(err)? {
                    return Err("save your Anthropic API key first".into());
                }
            }
            "ollama" => {
                if settings::get(&conn, settings::keys::OLLAMA_ENDPOINT)
                    .map_err(err)?
                    .is_none()
                {
                    return Err("save your Ollama endpoint first".into());
                }
            }
            _ => {}
        }
        if !secrets::exists(secrets::keys::TELEGRAM_BOT_TOKEN).map_err(err)? {
            return Err("save your Telegram bot token first".into());
        }
        let auth_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM telegram_authorized_chats", [], |r| {
                r.get(0)
            })
            .map_err(err)?;
        if auth_count == 0 {
            return Err("pair a chat first (send /start <code> to your bot)".into());
        }
        settings::set(&conn, settings::keys::SETUP_COMPLETE, "1").map_err(err)?;
    }
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
    let conn = state.db.lock().unwrap();
    dashboard(&conn, range.into(), OffsetDateTime::now_utc()).map_err(err)
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

#[tauri::command]
pub async fn list_expenses(
    filters: ExpenseFilters,
    state: State<'_, AppState>,
) -> Result<Vec<LedgerRow>, String> {
    let limit = filters.limit.unwrap_or(100).min(500);
    let offset = filters.offset.unwrap_or(0);
    let now = OffsetDateTime::now_utc();
    let tz = now.offset();

    let conn = state.db.lock().unwrap();
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

    let mut stmt = conn.prepare(&sql).map_err(err)?;
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
        })
        .map_err(err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(err)?;
    Ok(rows)
}

#[tauri::command]
pub async fn delete_expense(id: i64, state: State<'_, AppState>) -> Result<bool, String> {
    let conn = state.db.lock().unwrap();
    expenses::delete(&conn, id).map_err(err)
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
    let conn = state.db.lock().unwrap();
    categories::insert(
        &conn,
        &NewCategory {
            name: arg.name,
            kind,
            monthly_target_cents: arg.monthly_target_cents,
            is_recurring: arg.is_recurring,
            recurrence_day_of_month: arg.recurrence_day_of_month,
        },
    )
    .map_err(err)
}

#[tauri::command]
pub async fn delete_category(id: i64, state: State<'_, AppState>) -> Result<bool, String> {
    let conn = state.db.lock().unwrap();
    categories::delete(&conn, id).map_err(err)
}

// ---------------------------------------------------------------------
// Household.
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn remove_household_member(
    chat_id: i64,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let conn = state.db.lock().unwrap();
    auth::remove_member(&conn, chat_id).map_err(err)
}

// ---------------------------------------------------------------------
// Background mode + autostart.
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn get_run_in_background(state: State<'_, AppState>) -> Result<bool, String> {
    let conn = state.db.lock().unwrap();
    Ok(settings::get(&conn, settings::keys::RUN_IN_BACKGROUND)
        .map_err(err)?
        .as_deref()
        != Some("0"))
}

#[tauri::command]
pub async fn set_run_in_background(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    settings::set(
        &conn,
        settings::keys::RUN_IN_BACKGROUND,
        if enabled { "1" } else { "0" },
    )
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
    let conn = state.db.lock().unwrap();
    settings::set(
        &conn,
        settings::keys::AUTOSTART,
        if enabled { "1" } else { "0" },
    )
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
    let conn = state.db.lock().unwrap();
    Ok(
        settings::get(&conn, settings::keys::CHECK_UPDATES_ON_LAUNCH)
            .map_err(err)?
            .as_deref()
            != Some("0"),
    )
}

#[tauri::command]
pub async fn set_check_updates_on_launch(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    settings::set(
        &conn,
        settings::keys::CHECK_UPDATES_ON_LAUNCH,
        if enabled { "1" } else { "0" },
    )
    .map_err(err)
}

#[tauri::command]
pub async fn get_weekly_summary_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    let conn = state.db.lock().unwrap();
    Ok(settings::get(&conn, settings::keys::WEEKLY_SUMMARY_ENABLED)
        .map_err(err)?
        .as_deref()
        != Some("0"))
}

#[tauri::command]
pub async fn set_weekly_summary_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    settings::set(
        &conn,
        settings::keys::WEEKLY_SUMMARY_ENABLED,
        if enabled { "1" } else { "0" },
    )
    .map_err(err)
}

#[tauri::command]
pub async fn get_budget_alerts_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    let conn = state.db.lock().unwrap();
    Ok(settings::get(&conn, settings::keys::BUDGET_ALERTS_ENABLED)
        .map_err(err)?
        .as_deref()
        != Some("0"))
}

#[tauri::command]
pub async fn set_budget_alerts_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    settings::set(
        &conn,
        settings::keys::BUDGET_ALERTS_ENABLED,
        if enabled { "1" } else { "0" },
    )
    .map_err(err)
}

#[tauri::command]
pub async fn get_llm_usage_summary(
    state: State<'_, AppState>,
) -> Result<llm_usage::UsageSummary, String> {
    let conn = state.db.lock().unwrap();
    let now = OffsetDateTime::now_utc();
    llm_usage::summary(&conn, now).map_err(err)
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
    let conn = state.db.lock().unwrap();
    let now = OffsetDateTime::now_utc();
    let n = months_back.clamp(1, 120);
    let totals = expenses::monthly_totals_for_category(&conn, category_id, now, n).map_err(err)?;
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

#[tauri::command]
pub async fn project_investment(
    input: ProjectInvestmentInput,
) -> Result<InvestmentProjection, String> {
    Ok(forecast_mod::project_investment(&input))
}

#[tauri::command]
pub async fn solve_goal_seek(input: GoalSeekInput) -> Result<GoalSeekResult, String> {
    Ok(forecast_mod::solve_goal_seek(&input))
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
    let conn = state.db.lock().unwrap();
    // Pull active variable categories with monthly_target_cents set.
    let mut stmt = conn
        .prepare(
            "SELECT id, COALESCE(monthly_target_cents, 0)
             FROM categories
             WHERE kind = 'variable' AND is_active = 1
               AND monthly_target_cents IS NOT NULL
               AND monthly_target_cents > 0",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
        .map_err(err)?
        .collect::<rusqlite::Result<Vec<_>>>()
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
    let conn = state.db.lock().unwrap();
    categories::set_starting_balance(
        &conn,
        input.category_id,
        input.starting_balance_cents,
        input.balance_as_of.as_deref(),
    )
    .map_err(err)?;
    Ok(())
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
    let conn = state.db.lock().unwrap();
    let now = OffsetDateTime::now_utc();
    let mut stmt = conn
        .prepare(
            "SELECT id, name, starting_balance_cents, balance_as_of
             FROM categories
             WHERE kind = 'investing' AND is_active = 1
             ORDER BY name ASC",
        )
        .map_err(err)?;
    let cats = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(err)?;
    drop(stmt);
    let mut out = Vec::with_capacity(cats.len());
    for (id, name, balance, as_of) in cats {
        let totals = expenses::monthly_totals_for_category(&conn, id, now, 12).map_err(err)?;
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
}

// ---------------------------------------------------------------------
// Forecast wave 2 — Monte Carlo, runway, trend, goal probability.
// (v0.3.3)
// ---------------------------------------------------------------------

use crate::insights::monte_carlo::{
    self, goal_probability as mc_goal_probability, simulate as mc_simulate, PathBands, PathInput,
};
use crate::insights::runway::{self as runway_mod, RunwayResult};
use crate::insights::trend::{self as trend_mod, TrendResult};

#[tauri::command]
pub async fn monte_carlo_investment(input: PathInput) -> Result<PathBands, String> {
    Ok(mc_simulate(&input))
}

#[derive(Debug, Deserialize)]
pub struct GoalProbabilityInput {
    pub starting_balance_cents: i64,
    pub monthly_contribution_cents: i64,
    pub annual_return_pct: f64,
    pub annual_volatility_pct: f64,
    pub horizon_years: u32,
    pub target_cents: i64,
    pub n_paths: u32,
    pub seed: Option<u64>,
}

#[tauri::command]
pub async fn compute_goal_probability(input: GoalProbabilityInput) -> Result<f64, String> {
    let path = PathInput {
        starting_balance_cents: input.starting_balance_cents,
        monthly_contribution_cents: input.monthly_contribution_cents,
        annual_return_pct: input.annual_return_pct,
        annual_volatility_pct: input.annual_volatility_pct,
        horizon_years: input.horizon_years,
        n_paths: input.n_paths,
        time_points: 2,
        seed: input.seed,
    };
    Ok(mc_goal_probability(&path, input.target_cents))
}

#[tauri::command]
pub async fn compute_runway(state: State<'_, AppState>) -> Result<RunwayResult, String> {
    let conn = state.db.lock().unwrap();
    runway_mod::compute(&conn, OffsetDateTime::now_utc()).map_err(err)
}

#[tauri::command]
pub async fn compute_category_trend(
    category_id: i64,
    months_back: u32,
    state: State<'_, AppState>,
) -> Result<TrendResult, String> {
    let conn = state.db.lock().unwrap();
    let n = if months_back == 0 { 24 } else { months_back };
    trend_mod::compute(&conn, category_id, OffsetDateTime::now_utc(), n).map_err(err)
}

// silence unused-import warning when only the re-export above is used
#[allow(dead_code)]
fn _force_use_monte_carlo(_: &monte_carlo::PathBands) {}

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
    let conn = state.db.lock().unwrap();
    let suggested =
        csv_profiles::find_by_signature(&conn, &preview.header_signature).map_err(err)?;
    let profiles = csv_profiles::list(&conn).map_err(err)?;
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
    let conn = state.db.lock().unwrap();
    let now = OffsetDateTime::now_utc();
    csv_profiles::create(
        &conn,
        &input.name,
        input.header_signature.as_deref(),
        &input.mapping,
        now,
    )
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
    let conn = state.db.lock().unwrap();
    let decisions = categorize_mod::categorize_all(&conn, &input.rows).map_err(err)?;
    let duplicates = dedupe::find_probable_duplicates(&conn, &input.rows).map_err(err)?;
    Ok(CategorizeAndDedupeResult {
        decisions,
        duplicates,
    })
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
    let (hints, provider_choice, anth_model, ollama_endpoint, ollama_model) = {
        let conn = state.db.lock().unwrap();
        let cats = categories::list(&conn, false).map_err(err)?;
        let hints: Vec<CategoryHint> = cats
            .into_iter()
            .filter(|c| c.is_active)
            .map(|c| CategoryHint {
                id: c.id,
                name: c.name,
                hint: Some(format!("{:?}", c.kind).to_lowercase()),
            })
            .collect();
        let provider_choice = settings::get(&conn, settings::keys::LLM_PROVIDER)
            .map_err(err)?
            .unwrap_or_else(|| "anthropic".into());
        let anth_model = settings::get_or_default(
            &conn,
            settings::keys::ANTHROPIC_MODEL,
            DEFAULT_ANTHROPIC_MODEL,
        )
        .map_err(err)?;
        let ollama_endpoint = settings::get_or_default(
            &conn,
            settings::keys::OLLAMA_ENDPOINT,
            "http://localhost:11434",
        )
        .map_err(err)?;
        let ollama_model =
            settings::get_or_default(&conn, settings::keys::OLLAMA_MODEL, "llama3:8b")
                .map_err(err)?;
        (
            hints,
            provider_choice,
            anth_model,
            ollama_endpoint,
            ollama_model,
        )
    };

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

#[tauri::command]
pub async fn csv_import_commit(
    input: CommitInput,
    state: State<'_, AppState>,
) -> Result<CommitResult, String> {
    let conn = state.db.lock().unwrap();
    let now = OffsetDateTime::now_utc();
    let currency =
        settings::get_or_default(&conn, settings::keys::DEFAULT_CURRENCY, "USD").map_err(err)?;

    let mut inserted = 0usize;
    for row in &input.rows {
        let occurred_at = OffsetDateTime::parse(
            &row.occurred_at,
            &time::format_description::well_known::Rfc3339,
        )
        .map_err(err)?;
        expenses::insert(
            &conn,
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
        )
        .map_err(err)?;
        inserted += 1;
    }

    let mut rules_added = 0usize;
    for rule in &input.rules_to_save {
        merchant_rules::create(
            &conn,
            &rule.pattern,
            rule.category_id,
            rule.default_is_refund,
            0,
            now,
        )
        .map_err(err)?;
        rules_added += 1;
    }

    if let Some(id) = input.profile_id {
        let _ = csv_profiles::touch(&conn, id, now);
    }

    Ok(CommitResult {
        inserted,
        rules_added,
    })
}

// --- Settings management for profiles + rules ----------------------

#[tauri::command]
pub async fn list_csv_import_profiles(
    state: State<'_, AppState>,
) -> Result<Vec<CsvImportProfile>, String> {
    let conn = state.db.lock().unwrap();
    csv_profiles::list(&conn).map_err(err)
}

#[tauri::command]
pub async fn delete_csv_import_profile(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    csv_profiles::delete(&conn, id).map_err(err)
}

#[tauri::command]
pub async fn list_merchant_rules(state: State<'_, AppState>) -> Result<Vec<MerchantRule>, String> {
    let conn = state.db.lock().unwrap();
    merchant_rules::list(&conn).map_err(err)
}

#[tauri::command]
pub async fn delete_merchant_rule(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    merchant_rules::delete(&conn, id).map_err(err)
}

// ---------------------------------------------------------------------
// Misc.
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn ping() -> Result<String, String> {
    Ok("pong".into())
}
