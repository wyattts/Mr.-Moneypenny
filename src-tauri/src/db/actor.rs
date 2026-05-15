// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//! Single-writer SQLite actor.
//!
//! One dedicated OS thread owns the `rusqlite::Connection`. Async
//! callers submit a closure plus a `oneshot::Sender` over a bounded
//! `tokio::sync::mpsc`; the actor pulls them off in FIFO order, runs
//! the closure synchronously, and ships the result back. Tokio worker
//! threads never block on SQL — they just await a oneshot.
//!
//! This is the audit's recommended fix for **CC-1** (sync rusqlite on
//! Tokio workers), **CC-4** (`Mutex<Connection>` held across `.await`
//! is fragile), and **CC-6** (scheduler `unwrap`s on a poisoned lock).
//! See docs/audit-v0.3.7.md.
//!
//! ## Migration phase 1
//!
//! This module ships alongside the existing `Arc<Mutex<Connection>>`
//! pattern; both APIs are wired up in `AppState` and they coexist on a
//! per-handler basis. Phase 2 migrates the scheduler, phase 3 migrates
//! the `commands.rs` handlers, and phase 4 removes the old Mutex API.
//! During the migration the database file is opened twice (once for the
//! Mutex path, once for the actor), which SQLite handles cleanly under
//! WAL.
//!
//! ## Design notes
//!
//! Closures are boxed `FnOnce(&mut Connection) -> anyhow::Result<T>`, so
//! any caller can ship any query pattern without needing a request enum.
//! The boxed-dispatch overhead is negligible vs. the SQL.
//!
//! The actor catches panics in the closure (`std::panic::catch_unwind` +
//! `AssertUnwindSafe`) and surfaces them as `Err`. A buggy handler does
//! not kill the thread; subsequent calls continue to work.
//!
//! Shutdown is automatic: when the last clone of `DbHandle` drops, the
//! channel closes, `blocking_recv` returns `None`, and the actor thread
//! exits — dropping the Connection cleanly.
//!
//! The channel is bounded (`CHANNEL_CAPACITY`) so a flood of callers
//! provides natural backpressure instead of growing an unbounded queue.

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::thread;

use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;
use tokio::sync::{mpsc, oneshot};

/// Bounded inflight-job queue. Sized so two concurrent heavy handlers
/// plus the scheduler tick plus the poller never block trivially, but
/// a runaway producer is throttled instead of OOM'ing the process.
const CHANNEL_CAPACITY: usize = 64;

/// Boxed job. The actor calls the inner function with `&mut Connection`;
/// the function itself owns the matching `oneshot::Sender` and is
/// responsible for shipping the result. Box-and-forget keeps the
/// boundary one type wide regardless of `T`.
type DbJob = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

/// Async handle to a DB actor running on a dedicated OS thread. Cheap
/// to clone (`Sender` is internally `Arc`); pass clones to whichever
/// subsystems need DB access.
#[derive(Clone)]
pub struct DbHandle {
    sender: mpsc::Sender<DbJob>,
}

impl DbHandle {
    /// Take ownership of a pre-opened, pre-migrated `Connection` and
    /// hand it off to a freshly spawned actor thread. The thread name
    /// is set so it shows up in OS-level process listings.
    pub fn spawn(conn: Connection) -> Self {
        let (sender, mut receiver) = mpsc::channel::<DbJob>(CHANNEL_CAPACITY);
        thread::Builder::new()
            .name("moneypenny-db".into())
            .spawn(move || {
                let mut conn = conn;
                while let Some(job) = receiver.blocking_recv() {
                    job(&mut conn);
                }
                tracing::debug!(
                    target: "db::actor",
                    "channel closed; db actor thread exiting"
                );
            })
            .expect("failed to spawn db actor thread");
        Self { sender }
    }

    /// Convenience: open + migrate the DB at `path`, then hand off to
    /// `spawn`. Useful so callers don't have to repeat the
    /// open-then-migrate dance.
    pub fn spawn_from_path(path: &Path) -> Result<Self> {
        let conn = super::open(path).context("opening db for actor")?;
        super::migrate(&conn).context("migrating db for actor")?;
        Ok(Self::spawn(conn))
    }

    /// Synchronous version of [`run`] for callers that aren't on the
    /// Tokio runtime — Tauri's `setup()` hook, the `ExitRequested`
    /// run-loop callback, and the `ensure_poller_running` /
    /// `ensure_scheduler_running` startup helpers all live there.
    /// Blocks the calling thread until the actor finishes the closure.
    ///
    /// Don't call this from an async context — it would block the
    /// Tokio worker. Use [`run`] there.
    pub fn blocking_run<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let (otx, orx) = std::sync::mpsc::sync_channel::<Result<T>>(1);
        let job: DbJob = Box::new(move |conn: &mut Connection| {
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| f(conn)));
            let result = match outcome {
                Ok(r) => r,
                Err(panic) => Err(panic_to_err(panic)),
            };
            let _ = otx.send(result);
        });
        self.sender
            .blocking_send(job)
            .map_err(|_| anyhow!("db actor shut down"))?;
        orx.recv()
            .map_err(|_| anyhow!("db job dropped before completion"))?
    }

    /// Run a synchronous closure on the actor thread with exclusive
    /// access to the connection. Returns the closure's result as a
    /// future the caller awaits.
    ///
    /// Errors:
    /// - The closure returned `Err(_)` — surfaced verbatim.
    /// - The closure panicked — wrapped in `Err("db closure panicked: …")`.
    /// - The actor has already shut down (handle dropped, channel
    ///   closed) — `Err("db actor shut down")`.
    pub async fn run<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let (otx, orx) = oneshot::channel::<Result<T>>();
        let job: DbJob = Box::new(move |conn: &mut Connection| {
            // The closure may panic; we want the actor to stay alive
            // and the caller to get a clear error. AssertUnwindSafe is
            // sound because we don't observe `conn` after the unwind —
            // we hand it back to the actor loop, which is robust to a
            // mid-query connection (rusqlite resets prepared statements
            // on next use).
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| f(conn)));
            let result = match outcome {
                Ok(r) => r,
                Err(panic) => Err(panic_to_err(panic)),
            };
            // The caller may have been dropped (cancelled future).
            // That's fine; ignore the SendError.
            let _ = otx.send(result);
        });

        self.sender
            .send(job)
            .await
            .map_err(|_| anyhow!("db actor shut down"))?;
        orx.await
            .map_err(|_| anyhow!("db job dropped before completion"))?
    }
}

fn panic_to_err(p: Box<dyn std::any::Any + Send>) -> anyhow::Error {
    let msg = if let Some(s) = p.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    };
    anyhow!("db closure panicked: {msg}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn fresh_handle() -> DbHandle {
        let conn = crate::db::open_in_memory().unwrap();
        crate::db::migrate(&conn).unwrap();
        DbHandle::spawn(conn)
    }

    #[tokio::test]
    async fn round_trip_returns_closure_value() {
        let h = fresh_handle();
        let v: i64 = h
            .run(|conn| {
                let n: i64 = conn.query_row("SELECT 42", [], |r| r.get(0))?;
                Ok(n)
            })
            .await
            .unwrap();
        assert_eq!(v, 42);
    }

    #[tokio::test]
    async fn closure_can_mutate_db_and_subsequent_read_sees_it() {
        let h = fresh_handle();
        // Insert a category.
        let new_id: i64 = h
            .run(|conn| {
                conn.execute(
                    "INSERT INTO categories (name, kind, is_recurring, is_active, is_seed) \
                     VALUES ('Test Cat', 'variable', 0, 1, 0)",
                    [],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .unwrap();
        // Read it back via a separate `run`.
        let name: String = h
            .run(move |conn| {
                let n: String = conn.query_row(
                    "SELECT name FROM categories WHERE id = ?1",
                    params![new_id],
                    |r| r.get(0),
                )?;
                Ok(n)
            })
            .await
            .unwrap();
        assert_eq!(name, "Test Cat");
    }

    #[tokio::test]
    async fn concurrent_submissions_are_serialized_and_all_return() {
        let h = fresh_handle();
        // Fire off ten concurrent reads; the actor processes them one
        // at a time but each future resolves with the correct value.
        let futures = (0..10).map(|i| {
            let h = h.clone();
            tokio::spawn(async move {
                h.run(move |conn| {
                    let v: i64 = conn.query_row(&format!("SELECT {i}"), [], |r| r.get(0))?;
                    Ok(v)
                })
                .await
            })
        });
        let mut results: Vec<i64> = Vec::new();
        for f in futures {
            results.push(f.await.unwrap().unwrap());
        }
        results.sort();
        assert_eq!(results, (0..10).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn panic_inside_closure_returns_err_and_actor_keeps_running() {
        let h = fresh_handle();
        let err = h
            .run(|_conn| -> Result<()> {
                panic!("kaboom");
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("kaboom"));
        // Actor is still alive — a follow-up call succeeds.
        let n: i64 = h
            .run(|conn| Ok(conn.query_row("SELECT 7", [], |r| r.get::<_, i64>(0))?))
            .await
            .unwrap();
        assert_eq!(n, 7);
    }

    #[tokio::test]
    async fn err_from_closure_propagates_verbatim() {
        let h = fresh_handle();
        let err = h
            .run(|_conn| -> Result<()> { Err(anyhow!("specific failure")) })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("specific failure"));
    }

    #[tokio::test]
    async fn dropping_the_handle_shuts_the_actor_down() {
        let h = fresh_handle();
        // Confirm the actor responds before drop.
        let _: i64 = h
            .run(|conn| Ok(conn.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))?))
            .await
            .unwrap();
        drop(h);
        // Nothing observable from this side; the thread exits because
        // its channel receiver returns None. The fact that we got here
        // without a hang or panic is the test.
    }

    #[tokio::test]
    async fn cancelled_caller_does_not_break_subsequent_calls() {
        let h = fresh_handle();
        // Submit a job whose oneshot receiver we drop before it
        // completes. The actor still runs the closure but its
        // `oneshot::send` returns Err(SendError); we ignore that.
        {
            let h2 = h.clone();
            let _ = tokio::time::timeout(
                std::time::Duration::from_nanos(1),
                h2.run(|conn| Ok(conn.query_row("SELECT 2", [], |r| r.get::<_, i64>(0))?)),
            )
            .await; // ignore timeout result; the future is dropped here
        }
        // Subsequent calls still resolve.
        let n: i64 = h
            .run(|conn| Ok(conn.query_row("SELECT 99", [], |r| r.get::<_, i64>(0))?))
            .await
            .unwrap();
        assert_eq!(n, 99);
    }

    // `blocking_run` doesn't use #[tokio::test] — it must work from a
    // plain sync context (Tauri's setup() hook is one).
    #[test]
    fn blocking_run_returns_closure_value_without_a_tokio_runtime() {
        let h = fresh_handle();
        let v: i64 = h
            .blocking_run(|conn| Ok(conn.query_row("SELECT 13", [], |r| r.get::<_, i64>(0))?))
            .unwrap();
        assert_eq!(v, 13);
    }
}
