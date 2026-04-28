//! Tauri-side application state. Holds:
//!   - The shared SQLite connection (single-writer, mutex-guarded).
//!   - The in-memory bot state (conversation history per chat).
//!   - The poller's shutdown flag and a "started" sentinel.
//!
//! The poller is started lazily (after the user has saved a Telegram bot
//! token). Spawning happens via `tauri::async_runtime::spawn`, which uses
//! Tauri's managed runtime and works from any caller context — including
//! Tauri's `setup()` callback, where Tokio's `tokio::spawn` would panic
//! because the caller thread isn't inside a Tokio runtime context.

use std::sync::atomic::AtomicBool;
#[cfg(feature = "desktop")]
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

#[cfg(feature = "desktop")]
use anyhow::{Context, Result};
use rusqlite::Connection;

#[cfg(feature = "desktop")]
use crate::llm::anthropic::{AnthropicProvider, DEFAULT_MODEL as DEFAULT_ANTHROPIC_MODEL};
#[cfg(feature = "desktop")]
use crate::llm::ollama::OllamaProvider;
#[cfg(feature = "desktop")]
use crate::llm::LLMProvider;
#[cfg(feature = "desktop")]
use crate::repository::settings;
#[cfg(feature = "desktop")]
use crate::secrets;
#[cfg(feature = "desktop")]
use crate::telegram::client::TelegramClient;
#[cfg(feature = "desktop")]
use crate::telegram::poller;
#[cfg(feature = "desktop")]
use crate::telegram::router::RouterDeps;
use crate::telegram::state::BotState;

pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub bot: Arc<BotState>,
    #[allow(dead_code)] // only read by the desktop poller path
    inner: Mutex<Inner>,
}

#[derive(Default)]
#[allow(dead_code)] // only read by the desktop poller path
struct Inner {
    /// True once the poller task has been spawned. We don't track the
    /// JoinHandle — the OS reaps the task on process exit, and graceful
    /// shutdown happens via the `shutdown` flag.
    started: bool,
    shutdown: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(db: Connection) -> Self {
        Self {
            db: Arc::new(Mutex::new(db)),
            bot: Arc::new(BotState::new()),
            inner: Mutex::new(Inner {
                started: false,
                shutdown: Arc::new(AtomicBool::new(false)),
            }),
        }
    }

    /// Spawn the long-poll task using the currently saved Telegram token
    /// and LLM provider. Idempotent: no-op if already running. Returns
    /// an error if either secret/setting is missing.
    #[cfg(feature = "desktop")]
    pub fn ensure_poller_running(&self) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.started {
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

        // Tauri's spawn uses its managed runtime; safe to call from setup()
        // or from a Tauri command handler regardless of current context.
        tauri::async_runtime::spawn(async move {
            if let Err(e) = poller::run(deps, shutdown).await {
                tracing::error!(target: "app_state", error=%e, "poller exited with error");
            }
        });
        inner.started = true;
        Ok(())
    }

    /// Signal the poller to stop. Returns immediately; the task will exit
    /// when it next checks the flag (at most one long-poll timeout away).
    #[cfg(feature = "desktop")]
    pub fn shutdown_poller(&self) {
        let inner = self.inner.lock().unwrap();
        inner.shutdown.store(true, Ordering::Relaxed);
    }
}

/// Build the configured LLM provider based on `llm_provider` setting.
#[cfg(feature = "desktop")]
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
