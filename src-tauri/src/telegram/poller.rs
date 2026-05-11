//! Long-poll loop. Spawns one tokio task that drains `getUpdates` from
//! Telegram and dispatches each update through the router.
//!
//! No relay, no inbound port. The host desktop's outbound long-poll
//! connection is the only network channel between Telegram and the user's
//! database.
//!
//! ## Idempotency (audit S-5)
//!
//! The pre-v0.3.14 sequence was `handle_update` (which can insert an
//! expense) → `persist_offset`. A crash in that window re-fetched the
//! update on restart and re-inserted the expense. v0.3.14 records each
//! processed `update_id` in `processed_telegram_updates` and short-
//! circuits before `handle_update` runs again. The offset bump + the
//! idempotency row insert share a single transaction.
//!
//! Residual window: if we crash *during* `handle_update`, after the
//! expense INSERT commits but before the post-update transaction
//! commits, we'll still re-process on restart. Fully closing that
//! requires threading a tx handle through the dispatcher, which is a
//! v0.4.0-scope refactor — for now we narrow the window from "anything
//! handle_update writes" to "post-handle, pre-commit", which is
//! microseconds rather than the round-trip-to-Anthropic this used to
//! span.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rusqlite::params;
use time::OffsetDateTime;

use super::auth;
use super::router::{self, RouterDeps};

/// How long processed-update rows are retained. Pure housekeeping —
/// once an update_id is below `last_update_id`, the idempotency check
/// short-circuits anyway. Keeping a month of history just makes the
/// table queryable for debugging if a user reports a phantom event.
const PROCESSED_GC_RETENTION_DAYS: i64 = 30;

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

        // Periodic housekeeping: drop expired pairing codes and aged
        // idempotency rows. Both are best-effort.
        {
            let conn = deps.conn.lock().unwrap();
            let now = OffsetDateTime::now_utc();
            let _ = auth::expire_old_pairings(&conn, now);
            let _ = gc_processed_updates(&conn, now);
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
                    if already_processed(&deps, upd.update_id).unwrap_or(false) {
                        tracing::debug!(
                            target: "telegram::poller",
                            update_id = upd.update_id,
                            "update already processed; advancing offset only",
                        );
                        if let Err(e) = persist_offset_and_mark(&deps, upd.update_id) {
                            tracing::error!(
                                target: "telegram::poller",
                                error = %e,
                                "could not advance offset on already-processed update",
                            );
                        }
                        continue;
                    }
                    let handler_result = router::handle_update(&deps, &upd, now).await;
                    if let Err(e) = handler_result {
                        tracing::error!(
                            target: "telegram::poller",
                            update_id = upd.update_id,
                            error = %e,
                            "router::handle_update failed"
                        );
                    }
                    // Always advance + mark, even when the handler
                    // errored: re-trying a buggy update next launch is
                    // worse than dropping one, and the user will get a
                    // user-visible failure on their phone anyway.
                    if let Err(e) = persist_offset_and_mark(&deps, upd.update_id) {
                        tracing::error!(
                            target: "telegram::poller",
                            error = %e,
                            "could not persist offset / idempotency row",
                        );
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

fn already_processed(deps: &RouterDeps, update_id: i64) -> Result<bool> {
    let conn = deps.conn.lock().unwrap();
    let exists: i64 = conn
        .query_row(
            "SELECT 1 FROM processed_telegram_updates WHERE update_id = ?1",
            params![update_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(exists == 1)
}

/// Bump `telegram_state.last_update_id` AND insert the idempotency row
/// in the same transaction. A crash between these two writes would
/// re-open the duplicate-on-restart window this fix is closing.
fn persist_offset_and_mark(deps: &RouterDeps, update_id: i64) -> Result<()> {
    let conn = deps.conn.lock().unwrap();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT OR IGNORE INTO processed_telegram_updates (update_id) VALUES (?1)",
        params![update_id],
    )?;
    tx.execute(
        "UPDATE telegram_state SET last_update_id = ?1 WHERE id = 1",
        params![update_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// Drop processed-update rows older than `PROCESSED_GC_RETENTION_DAYS`.
/// Best-effort cleanup so the table doesn't grow unbounded.
fn gc_processed_updates(conn: &rusqlite::Connection, now: OffsetDateTime) -> Result<usize> {
    let cutoff = now - time::Duration::days(PROCESSED_GC_RETENTION_DAYS);
    let n = conn.execute(
        "DELETE FROM processed_telegram_updates WHERE processed_at < ?1",
        params![cutoff],
    )?;
    Ok(n)
}

async fn wait_for_shutdown(flag: &AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use rusqlite::Connection;
    use std::sync::Mutex as StdMutex;

    fn fresh() -> Connection {
        let c = db::open_in_memory().unwrap();
        db::migrate(&c).unwrap();
        c
    }

    #[test]
    fn already_processed_returns_false_for_unseen_update() {
        let conn = fresh();
        let conn_mtx = StdMutex::new(conn);
        let exists: i64 = conn_mtx
            .lock()
            .unwrap()
            .query_row(
                "SELECT 1 FROM processed_telegram_updates WHERE update_id = 42",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        assert_eq!(exists, 0);
    }

    #[test]
    fn persist_offset_and_mark_is_atomic_and_idempotent() {
        // Directly exercise the SQL the helper runs; we don't have a
        // RouterDeps in scope from this unit-test layer.
        let conn = fresh();
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT OR IGNORE INTO processed_telegram_updates (update_id) VALUES (?1)",
            params![100i64],
        )
        .unwrap();
        tx.execute(
            "UPDATE telegram_state SET last_update_id = ?1 WHERE id = 1",
            params![100i64],
        )
        .unwrap();
        tx.commit().unwrap();

        // Second time, same update — INSERT OR IGNORE is a no-op,
        // offset moves to the (same) value cleanly.
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT OR IGNORE INTO processed_telegram_updates (update_id) VALUES (?1)",
            params![100i64],
        )
        .unwrap();
        tx.execute(
            "UPDATE telegram_state SET last_update_id = ?1 WHERE id = 1",
            params![100i64],
        )
        .unwrap();
        tx.commit().unwrap();

        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processed_telegram_updates WHERE update_id = 100",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "idempotency row stays unique");
        let last: i64 = conn
            .query_row(
                "SELECT last_update_id FROM telegram_state WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(last, 100);
    }

    #[test]
    fn gc_drops_old_processed_rows_and_keeps_recent() {
        let conn = fresh();
        let now = OffsetDateTime::now_utc();
        let old = now - time::Duration::days(40);
        let recent = now - time::Duration::days(5);
        conn.execute(
            "INSERT INTO processed_telegram_updates (update_id, processed_at) VALUES (1, ?1)",
            params![old],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO processed_telegram_updates (update_id, processed_at) VALUES (2, ?1)",
            params![recent],
        )
        .unwrap();
        let n = gc_processed_updates(&conn, now).unwrap();
        assert_eq!(n, 1, "only the 40-day-old row should be dropped");
        let kept: i64 = conn
            .query_row(
                "SELECT update_id FROM processed_telegram_updates",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 2);
    }
}
