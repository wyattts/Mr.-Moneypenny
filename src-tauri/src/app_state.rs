//! Tauri-side application state. Holds:
//!   - The shared SQLite connection (single-writer, mutex-guarded).
//!   - The in-memory bot state (conversation history per chat).
//!   - The optional poller task handle and its shutdown flag.
//!
//! The poller is started lazily (after the user has saved a Telegram
//! bot token) and kept running for the lifetime of the app. It can be
//! stopped via `shutdown()` for graceful exit / unpair flows.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::Connection;
use tokio::task::JoinHandle;

use crate::llm::anthropic::{AnthropicProvider, DEFAULT_MODEL as DEFAULT_ANTHROPIC_MODEL};
use crate::llm::ollama::OllamaProvider;
use crate::llm::LLMProvider;
use crate::repository::settings;
use crate::secrets;
use crate::telegram::client::TelegramClient;
use crate::telegram::poller;
use crate::telegram::router::RouterDeps;
use crate::telegram::state::BotState;

pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub bot: Arc<BotState>,
    inner: Mutex<Inner>,
}

struct Inner {
    poller: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(db: Connection) -> Self {
        Self {
            db: Arc::new(Mutex::new(db)),
            bot: Arc::new(BotState::new()),
            inner: Mutex::new(Inner {
                poller: None,
                shutdown: Arc::new(AtomicBool::new(false)),
            }),
        }
    }

    /// Spawn the long-poll task using the currently saved Telegram token
    /// and LLM provider. Idempotent: no-op if a task is already running.
    /// Returns an error if either secret/setting is missing.
    pub fn ensure_poller_running(&self) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner
            .poller
            .as_ref()
            .map(|h| !h.is_finished())
            .unwrap_or(false)
        {
            return Ok(());
        }

        let token = secrets::retrieve(secrets::keys::TELEGRAM_BOT_TOKEN)?
            .context("telegram bot token not configured")?;
        let client = TelegramClient::new(token).context("building telegram client")?;
        let llm = build_llm_provider(&self.db.lock().unwrap())?;
        let default_currency = settings::get_or_default(
            &self.db.lock().unwrap(),
            settings::keys::DEFAULT_CURRENCY,
            "USD",
        )?;

        let deps = RouterDeps {
            conn: Arc::clone(&self.db),
            llm,
            client: Arc::new(client),
            state: Arc::clone(&self.bot),
            default_currency,
        };
        let shutdown = Arc::clone(&inner.shutdown);
        shutdown.store(false, Ordering::Relaxed);

        let handle = tokio::spawn(async move {
            if let Err(e) = poller::run(deps, shutdown).await {
                tracing::error!(target: "app_state", error=%e, "poller exited with error");
            }
        });
        inner.poller = Some(handle);
        Ok(())
    }

    /// Signal the poller to stop and wait briefly for graceful exit.
    /// Used on unpair / app shutdown.
    pub async fn shutdown_poller(&self) {
        let (handle, shutdown) = {
            let mut inner = self.inner.lock().unwrap();
            (inner.poller.take(), Arc::clone(&inner.shutdown))
        };
        shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = handle {
            // 5-second grace; if it doesn't exit by then, abort.
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), h).await;
        }
    }
}

/// Build the configured LLM provider based on `llm_provider` setting.
fn build_llm_provider(conn: &Connection) -> Result<Arc<dyn LLMProvider>> {
    let provider = settings::get_or_default(conn, settings::keys::LLM_PROVIDER, "")?;
    match provider.as_str() {
        "anthropic" => {
            let key = secrets::retrieve(secrets::keys::ANTHROPIC_API_KEY)?
                .context("anthropic api key not configured")?;
            let model = settings::get_or_default(
                conn,
                settings::keys::ANTHROPIC_MODEL,
                DEFAULT_ANTHROPIC_MODEL,
            )?;
            let p = AnthropicProvider::with_options(key, model, "https://api.anthropic.com")?;
            Ok(Arc::new(p))
        }
        "ollama" => {
            let endpoint = settings::get_or_default(
                conn,
                settings::keys::OLLAMA_ENDPOINT,
                "http://localhost:11434",
            )?;
            let model = settings::get_or_default(conn, settings::keys::OLLAMA_MODEL, "llama3:8b")?;
            let p = OllamaProvider::with_base_url(model, endpoint)?;
            Ok(Arc::new(p))
        }
        other => anyhow::bail!(
            "llm_provider not configured (got {other:?}); pick anthropic or ollama in setup"
        ),
    }
}
