//! Tauri IPC commands exposed to the React frontend.
//!
//! Each `#[tauri::command]` is the only way the frontend can touch the
//! database, secrets, or network — keep these handlers small and route
//! into the typed library code, never inline business logic here.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::app_state::AppState;
use crate::domain::CategoryKind;
use crate::llm::anthropic::{AnthropicProvider, DEFAULT_MODEL as DEFAULT_ANTHROPIC_MODEL};
use crate::llm::ollama::OllamaProvider;
use crate::llm::system_prompt::SystemPrompt;
use crate::llm::{ChatRequest, LLMProvider, Message};
use crate::repository::{categories, settings};
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

    // Spawn the poller now so /start <code> can be received.
    state.ensure_poller_running().map_err(err)?;

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
            kind: match c.kind {
                CategoryKind::Fixed => "fixed".into(),
                CategoryKind::Variable => "variable".into(),
            },
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
// Misc.
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn ping() -> Result<String, String> {
    Ok("pong".into())
}
