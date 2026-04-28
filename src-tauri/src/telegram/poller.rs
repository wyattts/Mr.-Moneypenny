//! Long-poll loop. Spawns one tokio task that drains `getUpdates` from
//! Telegram and dispatches each update through the router.
//!
//! No relay, no inbound port. The host desktop's outbound long-poll
//! connection is the only network channel between Telegram and the user's
//! database.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rusqlite::params;
use time::OffsetDateTime;

use super::auth;
use super::router::{self, RouterDeps};

/// Long-poll timeout in seconds. Telegram holds the connection for up to
/// this long if no updates are pending. 30s is the conventional value:
/// long enough to keep the radio idle, short enough that ungraceful
/// connection drops recover quickly.
const LONG_POLL_TIMEOUT_SECS: u32 = 30;

const INITIAL_BACKOFF_SECS: u64 = 1;
const MAX_BACKOFF_SECS: u64 = 60;

/// Run the poll loop until `shutdown` is set. Returns on graceful exit
/// or on a fatal database error.
pub async fn run(deps: RouterDeps, shutdown: Arc<AtomicBool>) -> Result<()> {
    // Defensive: ensure no leftover webhook is set, otherwise getUpdates
    // returns 409 Conflict. Cheap to call repeatedly; idempotent.
    if let Err(e) = deps.client.delete_webhook().await {
        tracing::warn!(target: "telegram::poller", error=%e, "deleteWebhook failed; continuing");
    }

    let mut backoff_secs = INITIAL_BACKOFF_SECS;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            tracing::info!(target: "telegram::poller", "shutdown requested");
            break;
        }

        // Periodic housekeeping: drop expired pairing codes.
        {
            let conn = deps.conn.lock().unwrap();
            let _ = auth::expire_old_pairings(&conn, OffsetDateTime::now_utc());
        }

        let offset = match read_offset(&deps) {
            Ok(o) => o,
            Err(e) => {
                tracing::error!(target: "telegram::poller", error=%e, "could not read last_update_id; aborting");
                return Err(e);
            }
        };

        match deps
            .client
            .get_updates(offset, LONG_POLL_TIMEOUT_SECS)
            .await
        {
            Ok(updates) => {
                backoff_secs = INITIAL_BACKOFF_SECS;
                for upd in updates {
                    let now = OffsetDateTime::now_utc();
                    if let Err(e) = router::handle_update(&deps, &upd, now).await {
                        tracing::error!(
                            target: "telegram::poller",
                            update_id = upd.update_id,
                            error = %e,
                            "router::handle_update failed"
                        );
                    }
                    if let Err(e) = persist_offset(&deps, upd.update_id) {
                        tracing::error!(target: "telegram::poller", error=%e, "could not persist offset");
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "telegram::poller",
                    error = %e,
                    backoff_secs,
                    "get_updates failed; backing off"
                );
                // Sleep, but break out promptly on shutdown.
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(backoff_secs)) => {}
                    _ = wait_for_shutdown(&shutdown) => {}
                }
                backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
            }
        }
    }
    Ok(())
}

fn read_offset(deps: &RouterDeps) -> Result<i64> {
    let conn = deps.conn.lock().unwrap();
    let last: i64 = conn.query_row(
        "SELECT last_update_id FROM telegram_state WHERE id = 1",
        [],
        |r| r.get(0),
    )?;
    // getUpdates uses `offset = last_processed + 1`.
    Ok(last + 1)
}

fn persist_offset(deps: &RouterDeps, update_id: i64) -> Result<()> {
    let conn = deps.conn.lock().unwrap();
    conn.execute(
        "UPDATE telegram_state SET last_update_id = ?1 WHERE id = 1",
        params![update_id],
    )?;
    Ok(())
}

async fn wait_for_shutdown(flag: &AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
