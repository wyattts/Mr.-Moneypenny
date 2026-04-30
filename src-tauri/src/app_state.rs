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
    /// Companion flag for the scheduler task. Same lifecycle pattern.
    scheduler_started: bool,
    scheduler_shutdown: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(db: Connection) -> Self {
        Self {
            db: Arc::new(Mutex::new(db)),
            bot: Arc::new(BotState::new()),
            inner: Mutex::new(Inner {
                started: false,
                shutdown: Arc::new(AtomicBool::new(false)),
                scheduler_started: false,
                scheduler_shutdown: Arc::new(AtomicBool::new(false)),
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

    /// Spawn the scheduler task. Idempotent. Independent of the TG poller —
    /// the scheduler can run without a bot token (its handlers will just
    /// no-op when there's nothing to send), so we start it as soon as the
    /// app comes up.
    #[cfg(feature = "desktop")]
    pub fn ensure_scheduler_running(&self) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.scheduler_started {
            return Ok(());
        }
        // Try to build the same RouterDeps the poller uses so handlers
        // share the LLM + Telegram client. If the user hasn't completed
        // setup yet, we skip until ensure_poller_running succeeds, since
        // both rely on the same secrets.
        let token = match secrets::retrieve(secrets::keys::TELEGRAM_BOT_TOKEN)? {
            Some(t) => t,
            None => return Ok(()), // setup incomplete; poller path will retry later
        };
        let client = TelegramClient::new(token).context("building telegram client")?;
        let llm = build_llm_provider(&self.db.lock().unwrap())?;
        let default_currency = settings::get_or_default(
            &self.db.lock().unwrap(),
            settings::keys::DEFAULT_CURRENCY,
            "USD",
        )?;

        let deps = crate::telegram::router::RouterDeps {
            conn: Arc::clone(&self.db),
            llm,
            client: Arc::new(client),
            state: Arc::clone(&self.bot),
            default_currency,
        };

        // Ensure the singleton jobs exist (weekly summary + budget alert
        // sweep). Idempotent — re-launching doesn't duplicate.
        {
            let conn = self.db.lock().unwrap();
            let now = time::OffsetDateTime::now_utc();
            // Fire the first weekly summary one week from now (so the
            // user doesn't get a "$0 in 0 expenses" DM immediately
            // after pairing).
            let _ = crate::scheduler::ensure_singleton(
                &conn,
                crate::scheduler::JobKind::WeeklySummary,
                now + time::Duration::days(7),
            );
            // Budget alert sweep starts firing within the next hour so
            // it picks up new spend promptly.
            let _ = crate::scheduler::ensure_singleton(
                &conn,
                crate::scheduler::JobKind::BudgetAlertSweep,
                now + time::Duration::minutes(5),
            );
        }

        let shutdown = Arc::clone(&inner.scheduler_shutdown);
        shutdown.store(false, Ordering::Relaxed);
        tauri::async_runtime::spawn(async move {
            crate::scheduler::run(deps, shutdown).await;
        });
        inner.scheduler_started = true;
        Ok(())
    }

    /// Tear down the running poller (if any) and spawn a fresh one with
    /// whatever Telegram token + LLM provider is currently saved. Used
    /// after a token rotation so the in-memory `TelegramClient` (which
    /// captured the old token at spawn time) is replaced.
    ///
    /// The previous task exits at its next loop tick — at most one
    /// long-poll timeout (~30s) later. The new task targets a different
    /// Telegram endpoint when the bot is genuinely new, so the overlap
    /// is harmless. Even when the user re-saves the same token, the
    /// resulting 409 Conflict on the new poller's first `getUpdates`
    /// call is recovered via the existing exponential-backoff path
    /// once the old poller exits.
    #[cfg(feature = "desktop")]
    pub fn restart_poller(&self) -> Result<()> {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.shutdown.store(true, Ordering::Relaxed);
            inner.started = false;
            inner.shutdown = Arc::new(AtomicBool::new(false));
        }
        self.ensure_poller_running()
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
