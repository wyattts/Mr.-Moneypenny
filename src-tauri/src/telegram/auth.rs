// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//! Pairing-code authentication and chat whitelisting.
//!
//! Bots are publicly addressable, so without an explicit allow-list anyone
//! who guesses or learns the bot's username could send messages to it.
//! The pairing-code flow is:
//!
//! 1. Desktop GUI calls `generate_pairing_code(name, kind)` → 8-digit
//!    code shown to the user with a 10-minute TTL. `kind` is one of
//!    `Owner` (granted only by codes the wizard issues during first-time
//!    setup) or `Member`.
//! 2. The user opens the bot in Telegram and sends `/start <code>`.
//! 3. Router calls `redeem_pairing_code(chat_id, code, now)`. The code's
//!    `kind` decides the resulting role; an attacker who brute-forces a
//!    member code cannot escalate to owner.
//! 4. `is_authorized(chat_id)` thereafter gates every incoming message.
//!
//! Sensitive operations (invite, remove member) require `is_owner`.
//!
//! ## Brute-force defense (v0.3.14)
//!
//! Two layers of rate-limiting back the 8-digit code:
//!
//! - **Per-chat**: once a chat submits `MAX_PER_CHAT_FAILS` wrong codes
//!   inside `PER_CHAT_WINDOW`, it enters an exponential cooldown
//!   (`PER_CHAT_COOLDOWNS`). Successful redemption clears the row.
//! - **Global**: across all chats, more than `MAX_GLOBAL_FAILS_PER_MIN`
//!   failures in a rolling minute pause every redemption for
//!   `GLOBAL_PAUSE`. Catches Sybil/multi-chat attackers that the
//!   per-chat counter can't see.

use anyhow::{anyhow, Result};
use rand::Rng;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

const PAIRING_TTL: Duration = Duration::minutes(10);

const PER_CHAT_WINDOW: Duration = Duration::seconds(60);
const MAX_PER_CHAT_FAILS: i64 = 5;
/// Exponential cooldown ladder applied each time a chat passes the
/// per-window threshold while still blocked. Index = how many cooldowns
/// the chat has already burned through.
const PER_CHAT_COOLDOWNS: &[Duration] = &[
    Duration::seconds(30),
    Duration::minutes(1),
    Duration::minutes(5),
    Duration::minutes(30),
    Duration::hours(24),
];

const GLOBAL_WINDOW: Duration = Duration::seconds(60);
const MAX_GLOBAL_FAILS_PER_MIN: i64 = 10;
const GLOBAL_PAUSE: Duration = Duration::seconds(30);

/// User-facing error string when a redemption is refused. Single string
/// for all "you can't pair right now" cases so an attacker can't tell
/// "wrong code", "rate-limited", "expired", or "no such code" apart.
/// (S-1 + S-3 in docs/audit-v0.3.7.md.)
const REJECT_MSG: &str = "invalid or expired pairing code";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Member,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Member => "member",
        }
    }
}

impl std::str::FromStr for Role {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "owner" => Role::Owner,
            "member" => Role::Member,
            other => anyhow::bail!("invalid role: {other}"),
        })
    }
}

/// The two kinds of pairing code. Mirrors `Role` semantically but lives
/// on the pending row so the redemption path doesn't have to derive
/// ownership from "first redeemer wins" — see audit S-1's ownership
/// hardening recommendation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PairingKind {
    Owner,
    Member,
}

impl PairingKind {
    fn as_str(&self) -> &'static str {
        match self {
            PairingKind::Owner => "owner",
            PairingKind::Member => "member",
        }
    }

    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "owner" => PairingKind::Owner,
            "member" => PairingKind::Member,
            other => anyhow::bail!("invalid pairing kind: {other}"),
        })
    }
}

impl From<PairingKind> for Role {
    fn from(k: PairingKind) -> Role {
        match k {
            PairingKind::Owner => Role::Owner,
            PairingKind::Member => Role::Member,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizedChat {
    pub chat_id: i64,
    pub display_name: String,
    pub role: Role,
}

/// Insert a new pairing code for a member-to-be (or owner-to-be).
/// Returns the 8-digit code. The display name is what the redeeming chat
/// will be recorded as. `kind` decides the redeemer's role.
pub fn generate_pairing_code(
    conn: &Connection,
    display_name: &str,
    kind: PairingKind,
    now: OffsetDateTime,
) -> Result<String> {
    let display_name = display_name.trim();
    if display_name.is_empty() {
        return Err(anyhow!("display_name cannot be empty"));
    }
    let mut rng = rand::thread_rng();
    // 8 digits, leading-zero-padded. 10^8 code space pairs with the
    // brute-force rate-limiting below; mobile-friendly to type back.
    let code = format!("{:08}", rng.gen_range(0..100_000_000));
    let expires_at = now + PAIRING_TTL;

    // First clear any expired rows so the table doesn't grow unbounded.
    conn.execute(
        "DELETE FROM telegram_pending_pairings WHERE expires_at < ?1",
        params![now],
    )?;

    conn.execute(
        "INSERT INTO telegram_pending_pairings (pairing_code, display_name, expires_at, kind)
         VALUES (?1, ?2, ?3, ?4)",
        params![code, display_name, expires_at, kind.as_str()],
    )?;
    Ok(code)
}

/// Consume a pairing code: enforce rate limits, validate it exists and
/// isn't expired, then insert the chat into the authorized list with the
/// role baked into the code's `kind`.
pub fn redeem_pairing_code(
    conn: &Connection,
    chat_id: i64,
    code: &str,
    now: OffsetDateTime,
) -> Result<AuthorizedChat> {
    let code = code.trim();
    if code.is_empty() {
        // Trip the same counters a wrong code would so an attacker can't
        // use empty arguments as a free probe.
        record_failure(conn, chat_id, now)?;
        return Err(anyhow!(REJECT_MSG));
    }

    // 1. Global pause? Most-restrictive gate first; cheap to check.
    if let Some(blocked_until) = global_blocked_until(conn)? {
        if blocked_until > now {
            return Err(anyhow!(REJECT_MSG));
        }
    }

    // 2. Per-chat cooldown?
    if let Some(blocked_until) = chat_blocked_until(conn, chat_id)? {
        if blocked_until > now {
            return Err(anyhow!(REJECT_MSG));
        }
    }

    // 3. If chat is already authorized, refuse — preserves their existing
    //    role. (Not a brute-force concern, just hygiene.)
    if let Some(existing) = is_authorized(conn, chat_id)? {
        return Err(anyhow!(
            "chat {} is already authorized as {}",
            chat_id,
            existing.role.as_str()
        ));
    }

    let tx = conn.unchecked_transaction()?;

    // Look up + atomically consume the code.
    let row: Option<(String, OffsetDateTime, String)> = tx
        .query_row(
            "SELECT display_name, expires_at, kind FROM telegram_pending_pairings WHERE pairing_code = ?1",
            params![code],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((display_name, expires_at, kind_str)) = row else {
        // No-such-code: trip both counters. Drop the tx (no writes yet).
        drop(tx);
        record_failure(conn, chat_id, now)?;
        tracing::debug!(target: "telegram::auth", chat_id, "redeem: no matching code");
        return Err(anyhow!(REJECT_MSG));
    };
    if expires_at < now {
        // Clean up the expired row inside this tx, then count as failure.
        tx.execute(
            "DELETE FROM telegram_pending_pairings WHERE pairing_code = ?1",
            params![code],
        )?;
        tx.commit()?;
        record_failure(conn, chat_id, now)?;
        tracing::debug!(target: "telegram::auth", chat_id, "redeem: code expired");
        return Err(anyhow!(REJECT_MSG));
    }

    let kind = PairingKind::parse(&kind_str)?;
    let role: Role = kind.into();

    tx.execute(
        "DELETE FROM telegram_pending_pairings WHERE pairing_code = ?1",
        params![code],
    )?;
    tx.execute(
        "INSERT INTO telegram_authorized_chats (chat_id, display_name, role) VALUES (?1, ?2, ?3)",
        params![chat_id, display_name, role.as_str()],
    )?;
    tx.commit()?;

    // Clear any cooldown state for this chat — they got it right.
    let _ = conn.execute(
        "DELETE FROM telegram_redemption_attempts WHERE chat_id = ?1",
        params![chat_id],
    );

    Ok(AuthorizedChat {
        chat_id,
        display_name,
        role,
    })
}

// --- rate-limit helpers ---------------------------------------------------

fn chat_blocked_until(conn: &Connection, chat_id: i64) -> Result<Option<OffsetDateTime>> {
    let row: Option<Option<OffsetDateTime>> = conn
        .query_row(
            "SELECT blocked_until FROM telegram_redemption_attempts WHERE chat_id = ?1",
            params![chat_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(row.flatten())
}

fn global_blocked_until(conn: &Connection) -> Result<Option<OffsetDateTime>> {
    let row: Option<Option<OffsetDateTime>> = conn
        .query_row(
            "SELECT blocked_until FROM telegram_redemption_global WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(row.flatten())
}

/// Record one redemption failure: bump the per-chat window counter
/// (resetting if the previous window elapsed) and the global counter,
/// applying cooldowns when either threshold trips.
fn record_failure(conn: &Connection, chat_id: i64, now: OffsetDateTime) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    record_per_chat_failure(&tx, chat_id, now)?;
    record_global_failure(&tx, now)?;
    tx.commit()?;
    Ok(())
}

fn record_per_chat_failure(
    tx: &rusqlite::Transaction,
    chat_id: i64,
    now: OffsetDateTime,
) -> Result<()> {
    let existing: Option<(i64, OffsetDateTime, Option<OffsetDateTime>, i64)> = tx
        .query_row(
            "SELECT attempts, window_start, blocked_until, cooldown_level
             FROM telegram_redemption_attempts WHERE chat_id = ?1",
            params![chat_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;

    match existing {
        None => {
            // First failure for this chat.
            tx.execute(
                "INSERT INTO telegram_redemption_attempts
                 (chat_id, attempts, window_start, blocked_until, cooldown_level)
                 VALUES (?1, 1, ?2, NULL, 0)",
                params![chat_id, now],
            )?;
        }
        Some((attempts, window_start, blocked_until, cooldown_level)) => {
            // If we're still inside a cooldown, leave it alone — every
            // request during the cooldown was already rejected at the
            // gate.
            if let Some(b) = blocked_until {
                if b > now {
                    return Ok(());
                }
            }
            let in_window = now - window_start < PER_CHAT_WINDOW;
            let new_attempts = if in_window { attempts + 1 } else { 1 };
            let new_window_start = if in_window { window_start } else { now };

            if new_attempts >= MAX_PER_CHAT_FAILS {
                // Trip the cooldown. Pick the next step from the ladder;
                // never go down, cap at the longest step.
                let idx = (cooldown_level as usize).min(PER_CHAT_COOLDOWNS.len() - 1);
                let step = PER_CHAT_COOLDOWNS[idx];
                let until = now + step;
                let next_level = (cooldown_level + 1).min(PER_CHAT_COOLDOWNS.len() as i64);
                tx.execute(
                    "UPDATE telegram_redemption_attempts
                     SET attempts = 0, window_start = ?2, blocked_until = ?3,
                         cooldown_level = ?4
                     WHERE chat_id = ?1",
                    params![chat_id, now, until, next_level],
                )?;
            } else {
                tx.execute(
                    "UPDATE telegram_redemption_attempts
                     SET attempts = ?2, window_start = ?3, blocked_until = NULL
                     WHERE chat_id = ?1",
                    params![chat_id, new_attempts, new_window_start],
                )?;
            }
        }
    }
    Ok(())
}

fn record_global_failure(tx: &rusqlite::Transaction, now: OffsetDateTime) -> Result<()> {
    let (failures, window_start, blocked_until): (i64, OffsetDateTime, Option<OffsetDateTime>) = tx
        .query_row(
            "SELECT failures_in_window, window_start, blocked_until
             FROM telegram_redemption_global WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;

    // If we're inside the global pause, nothing to update — the gate
    // already rejected the call and we don't want to extend the pause
    // for failed attempts during the pause window.
    if let Some(b) = blocked_until {
        if b > now {
            return Ok(());
        }
    }

    let in_window = now - window_start < GLOBAL_WINDOW;
    let new_failures = if in_window { failures + 1 } else { 1 };
    let new_window_start = if in_window { window_start } else { now };

    if new_failures >= MAX_GLOBAL_FAILS_PER_MIN {
        let until = now + GLOBAL_PAUSE;
        tx.execute(
            "UPDATE telegram_redemption_global
             SET failures_in_window = 0, window_start = ?1, blocked_until = ?2
             WHERE id = 1",
            params![now, until],
        )?;
    } else {
        tx.execute(
            "UPDATE telegram_redemption_global
             SET failures_in_window = ?1, window_start = ?2, blocked_until = NULL
             WHERE id = 1",
            params![new_failures, new_window_start],
        )?;
    }
    Ok(())
}

// --- read-side helpers (unchanged) ----------------------------------------

/// Look up an authorized chat by id. Returns `None` if not authorized.
pub fn is_authorized(conn: &Connection, chat_id: i64) -> Result<Option<AuthorizedChat>> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT display_name, role FROM telegram_authorized_chats WHERE chat_id = ?1",
            params![chat_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match row {
        Some((display_name, role_str)) => Some(AuthorizedChat {
            chat_id,
            display_name,
            role: role_str.parse()?,
        }),
        None => None,
    })
}

pub fn is_owner(conn: &Connection, chat_id: i64) -> Result<bool> {
    Ok(matches!(
        is_authorized(conn, chat_id)?,
        Some(c) if c.role == Role::Owner
    ))
}

pub fn list_members(conn: &Connection) -> Result<Vec<AuthorizedChat>> {
    let mut stmt = conn.prepare_cached(
        "SELECT chat_id, display_name, role FROM telegram_authorized_chats
         ORDER BY role DESC, display_name ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let chat_id: i64 = r.get(0)?;
            let display_name: String = r.get(1)?;
            let role_str: String = r.get(2)?;
            Ok((chat_id, display_name, role_str))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(chat_id, display_name, role_str)| {
            Ok(AuthorizedChat {
                chat_id,
                display_name,
                role: role_str.parse()?,
            })
        })
        .collect()
}

/// Remove a member from the whitelist. Refuses to remove the owner —
/// the owner role can only be transferred, not deleted, to avoid an
/// orphaned database with no admin.
pub fn remove_member(conn: &Connection, chat_id: i64) -> Result<bool> {
    let role_str: Option<String> = conn
        .query_row(
            "SELECT role FROM telegram_authorized_chats WHERE chat_id = ?1",
            params![chat_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(s) = role_str {
        if s == "owner" {
            return Err(anyhow!("cannot remove the owner; transfer ownership first"));
        }
    }
    let n = conn.execute(
        "DELETE FROM telegram_authorized_chats WHERE chat_id = ?1",
        params![chat_id],
    )?;
    Ok(n > 0)
}

/// Garbage-collect expired pairing rows. Safe to call periodically.
pub fn expire_old_pairings(conn: &Connection, now: OffsetDateTime) -> Result<usize> {
    let n = conn.execute(
        "DELETE FROM telegram_pending_pairings WHERE expires_at < ?1",
        params![now],
    )?;
    Ok(n)
}

/// Wipe every authorized chat and any pending pairing codes. Called from
/// the Settings UI's "factory reset" path after rotating the bot token,
/// when the user wants to start over with a clean whitelist. Returns the
/// number of authorized chats removed (pairings are also cleared but
/// are not counted).
///
/// Note: this deletes the owner row too, by design — the user is
/// explicitly asking to reset, and the next /start <code> redemption
/// (issued as kind='owner') becomes the new owner.
pub fn clear_all(conn: &Connection) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let n = tx.execute("DELETE FROM telegram_authorized_chats", [])?;
    tx.execute("DELETE FROM telegram_pending_pairings", [])?;
    tx.execute("DELETE FROM telegram_redemption_attempts", [])?;
    tx.execute(
        "UPDATE telegram_redemption_global
         SET failures_in_window = 0, window_start = ?1, blocked_until = NULL
         WHERE id = 1",
        params![OffsetDateTime::now_utc()],
    )?;
    tx.commit()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use time::macros::datetime;

    fn fresh() -> Connection {
        let c = db::open_in_memory().unwrap();
        db::migrate(&c).unwrap();
        c
    }

    #[test]
    fn owner_code_grants_owner_role() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let code = generate_pairing_code(&conn, "Wyatt", PairingKind::Owner, now).unwrap();
        assert_eq!(code.len(), 8);
        assert!(code.chars().all(|c| c.is_ascii_digit()));

        let auth = redeem_pairing_code(&conn, 111, &code, now).unwrap();
        assert_eq!(auth.role, Role::Owner);
        assert_eq!(auth.display_name, "Wyatt");
        assert!(is_owner(&conn, 111).unwrap());
    }

    #[test]
    fn member_code_grants_member_role_even_on_empty_db() {
        // Audit S-1 ownership hardening: a member-kind code does NOT
        // grant Owner on a fresh install. Before, the first redeemer was
        // promoted automatically; that's how a brute-forced member code
        // could escalate.
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let code = generate_pairing_code(&conn, "Wyatt", PairingKind::Member, now).unwrap();
        let auth = redeem_pairing_code(&conn, 111, &code, now).unwrap();
        assert_eq!(auth.role, Role::Member);
        assert!(!is_owner(&conn, 111).unwrap());
    }

    #[test]
    fn invalid_code_rejected_with_neutral_message() {
        // Audit S-3: don't oracle "expired" vs "no such code".
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let err = redeem_pairing_code(&conn, 111, "99999999", now).unwrap_err();
        assert_eq!(err.to_string(), REJECT_MSG);
    }

    #[test]
    fn expired_code_rejected_with_same_message_as_invalid() {
        // Audit S-3: expired and invalid produce the same user-facing
        // string. Internal distinction stays in tracing::debug.
        let conn = fresh();
        let issue = datetime!(2026-04-28 12:00:00 UTC);
        let code = generate_pairing_code(&conn, "Wyatt", PairingKind::Member, issue).unwrap();
        let now = issue + Duration::minutes(11);
        let err = redeem_pairing_code(&conn, 111, &code, now).unwrap_err();
        assert_eq!(err.to_string(), REJECT_MSG);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM telegram_pending_pairings WHERE pairing_code = ?1",
                params![code],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "expired row should have been cleaned up");
    }

    #[test]
    fn code_consumed_on_redeem() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let code = generate_pairing_code(&conn, "Wyatt", PairingKind::Owner, now).unwrap();
        redeem_pairing_code(&conn, 111, &code, now).unwrap();
        let err = redeem_pairing_code(&conn, 222, &code, now).unwrap_err();
        assert_eq!(err.to_string(), REJECT_MSG);
    }

    #[test]
    fn already_authorized_chat_refused_on_re_redeem() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let c1 = generate_pairing_code(&conn, "Wyatt", PairingKind::Owner, now).unwrap();
        redeem_pairing_code(&conn, 111, &c1, now).unwrap();

        let c2 = generate_pairing_code(&conn, "Wyatt 2", PairingKind::Member, now).unwrap();
        let err = redeem_pairing_code(&conn, 111, &c2, now).unwrap_err();
        assert!(err.to_string().contains("already authorized"));
    }

    #[test]
    fn per_chat_cooldown_kicks_in_after_five_fails() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        // Burn five wrong codes from one chat.
        for _ in 0..5 {
            let _ = redeem_pairing_code(&conn, 111, "00000000", now);
        }
        // Sixth attempt is rate-limited even with a valid code.
        let valid = generate_pairing_code(&conn, "Wyatt", PairingKind::Owner, now).unwrap();
        let err = redeem_pairing_code(&conn, 111, &valid, now).unwrap_err();
        assert_eq!(err.to_string(), REJECT_MSG);

        // After 31 seconds the cooldown lifts (first step is 30s).
        let later = now + Duration::seconds(31);
        let auth = redeem_pairing_code(&conn, 111, &valid, later).unwrap();
        assert_eq!(auth.role, Role::Owner);
    }

    #[test]
    fn per_chat_cooldown_escalates_on_repeated_offenses() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        for _ in 0..5 {
            let _ = redeem_pairing_code(&conn, 111, "00000000", now);
        }
        let blocked_until = chat_blocked_until(&conn, 111).unwrap().unwrap();
        assert_eq!(blocked_until - now, Duration::seconds(30));

        // Burn the cooldown, then trip the threshold again.
        let later = now + Duration::seconds(31);
        for _ in 0..5 {
            let _ = redeem_pairing_code(&conn, 111, "00000000", later);
        }
        let next_block = chat_blocked_until(&conn, 111).unwrap().unwrap();
        assert!(next_block - later >= Duration::minutes(1));
    }

    #[test]
    fn global_pause_after_ten_failures_in_a_minute() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        // Ten unique chats each fail once — per-chat counter never trips
        // but global ceiling should.
        for i in 0..10 {
            let _ = redeem_pairing_code(&conn, 1000 + i, "00000000", now);
        }
        // Even an honest chat with a valid code is rejected during the
        // global pause.
        let valid = generate_pairing_code(&conn, "Wyatt", PairingKind::Owner, now).unwrap();
        let err = redeem_pairing_code(&conn, 9999, &valid, now).unwrap_err();
        assert_eq!(err.to_string(), REJECT_MSG);

        // After 31 seconds the pause lifts.
        let later = now + Duration::seconds(31);
        let auth = redeem_pairing_code(&conn, 9999, &valid, later).unwrap();
        assert_eq!(auth.role, Role::Owner);
    }

    #[test]
    fn successful_redemption_clears_attempts_row() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        // A couple of misses (below threshold).
        for _ in 0..2 {
            let _ = redeem_pairing_code(&conn, 111, "00000000", now);
        }
        let attempts: i64 = conn
            .query_row(
                "SELECT attempts FROM telegram_redemption_attempts WHERE chat_id = 111",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(attempts, 2);

        // Now succeed — row should be deleted.
        let valid = generate_pairing_code(&conn, "Wyatt", PairingKind::Owner, now).unwrap();
        redeem_pairing_code(&conn, 111, &valid, now).unwrap();
        let still: Option<i64> = conn
            .query_row(
                "SELECT attempts FROM telegram_redemption_attempts WHERE chat_id = 111",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert!(still.is_none(), "successful redeem must clear attempts row");
    }

    #[test]
    fn cannot_remove_owner() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let c = generate_pairing_code(&conn, "Wyatt", PairingKind::Owner, now).unwrap();
        redeem_pairing_code(&conn, 111, &c, now).unwrap();
        let err = remove_member(&conn, 111).unwrap_err();
        assert!(err.to_string().contains("cannot remove the owner"));
    }

    #[test]
    fn clear_all_removes_owner_and_members_and_rate_limit_state() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let c1 = generate_pairing_code(&conn, "Wyatt", PairingKind::Owner, now).unwrap();
        redeem_pairing_code(&conn, 111, &c1, now).unwrap();
        let c2 = generate_pairing_code(&conn, "Spouse", PairingKind::Member, now).unwrap();
        redeem_pairing_code(&conn, 222, &c2, now).unwrap();
        let _orphan = generate_pairing_code(&conn, "Orphan", PairingKind::Member, now).unwrap();
        // Trip some rate-limit state.
        for _ in 0..3 {
            let _ = redeem_pairing_code(&conn, 333, "00000000", now);
        }

        let n = clear_all(&conn).unwrap();
        assert_eq!(n, 2);
        assert!(list_members(&conn).unwrap().is_empty());
        let pending: i64 = conn
            .query_row("SELECT COUNT(*) FROM telegram_pending_pairings", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(pending, 0);
        let attempts: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM telegram_redemption_attempts",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(attempts, 0);
        let global_blocked: Option<OffsetDateTime> = conn
            .query_row(
                "SELECT blocked_until FROM telegram_redemption_global WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(global_blocked.is_none());
    }

    #[test]
    fn remove_member_works() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let c1 = generate_pairing_code(&conn, "Wyatt", PairingKind::Owner, now).unwrap();
        redeem_pairing_code(&conn, 111, &c1, now).unwrap();
        let c2 = generate_pairing_code(&conn, "Spouse", PairingKind::Member, now).unwrap();
        redeem_pairing_code(&conn, 222, &c2, now).unwrap();

        assert!(remove_member(&conn, 222).unwrap());
        assert!(is_authorized(&conn, 222).unwrap().is_none());
    }
}
