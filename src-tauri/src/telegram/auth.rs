//! Pairing-code authentication and chat whitelisting.
//!
//! Bots are publicly addressable, so without an explicit allow-list anyone
//! who guesses or learns the bot's username could send messages to it.
//! The pairing-code flow is:
//!
//! 1. Desktop GUI calls `generate_pairing_code(name)` → 6-digit code shown
//!    to the user with a 10-minute TTL.
//! 2. The user opens the bot in Telegram and sends `/start <code>`.
//! 3. Router calls `redeem_pairing_code(chat_id, code, now)`. The first
//!    successful redemption becomes the household OWNER; subsequent
//!    redemptions become MEMBERS.
//! 4. `is_authorized(chat_id)` thereafter gates every incoming message.
//!
//! Sensitive operations (invite, remove member) require `is_owner`.

use anyhow::{anyhow, Result};
use rand::Rng;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

const PAIRING_TTL: Duration = Duration::minutes(10);

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizedChat {
    pub chat_id: i64,
    pub display_name: String,
    pub role: Role,
}

/// Insert a new pairing code for a member-to-be. Returns the 6-digit code.
/// The display name is what the redeeming chat will be recorded as.
pub fn generate_pairing_code(
    conn: &Connection,
    display_name: &str,
    now: OffsetDateTime,
) -> Result<String> {
    let display_name = display_name.trim();
    if display_name.is_empty() {
        return Err(anyhow!("display_name cannot be empty"));
    }
    let mut rng = rand::thread_rng();
    // 6 digits, leading-zero-padded.
    let code = format!("{:06}", rng.gen_range(0..1_000_000));
    let expires_at = now + PAIRING_TTL;

    // First clear any expired rows so the table doesn't grow unbounded.
    conn.execute(
        "DELETE FROM telegram_pending_pairings WHERE expires_at < ?1",
        params![now],
    )?;

    conn.execute(
        "INSERT INTO telegram_pending_pairings (pairing_code, display_name, expires_at)
         VALUES (?1, ?2, ?3)",
        params![code, display_name, expires_at],
    )?;
    Ok(code)
}

/// Consume a pairing code: validate it exists and isn't expired, then
/// insert the chat into the authorized list with the appropriate role.
/// First chat to redeem becomes owner.
pub fn redeem_pairing_code(
    conn: &Connection,
    chat_id: i64,
    code: &str,
    now: OffsetDateTime,
) -> Result<AuthorizedChat> {
    let code = code.trim();
    if code.is_empty() {
        return Err(anyhow!("pairing code is empty"));
    }

    // If chat is already authorized, refuse — preserves their existing role.
    if let Some(existing) = is_authorized(conn, chat_id)? {
        return Err(anyhow!(
            "chat {} is already authorized as {}",
            chat_id,
            existing.role.as_str()
        ));
    }

    let tx = conn.unchecked_transaction()?;

    // Look up + atomically consume the code.
    let row: Option<(String, OffsetDateTime)> = tx
        .query_row(
            "SELECT display_name, expires_at FROM telegram_pending_pairings WHERE pairing_code = ?1",
            params![code],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    let (display_name, expires_at) =
        row.ok_or_else(|| anyhow!("invalid or expired pairing code"))?;
    if expires_at < now {
        // Clean up the expired row and report the error.
        tx.execute(
            "DELETE FROM telegram_pending_pairings WHERE pairing_code = ?1",
            params![code],
        )?;
        tx.commit()?;
        return Err(anyhow!("pairing code expired"));
    }

    // Determine role: first authorized chat = owner.
    let owner_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM telegram_authorized_chats WHERE role = 'owner'",
        [],
        |r| r.get(0),
    )?;
    let role = if owner_count == 0 {
        Role::Owner
    } else {
        Role::Member
    };

    tx.execute(
        "DELETE FROM telegram_pending_pairings WHERE pairing_code = ?1",
        params![code],
    )?;
    tx.execute(
        "INSERT INTO telegram_authorized_chats (chat_id, display_name, role) VALUES (?1, ?2, ?3)",
        params![chat_id, display_name, role.as_str()],
    )?;
    tx.commit()?;

    Ok(AuthorizedChat {
        chat_id,
        display_name,
        role,
    })
}

/// Look up an authorized chat by id. Returns `None` if not authorized.
pub fn is_authorized(conn: &Connection, chat_id: i64) -> Result<Option<AuthorizedChat>> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT display_name, role FROM telegram_authorized_chats WHERE chat_id = ?1",
            params![chat_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
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
        .ok();
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
/// will become the new owner.
pub fn clear_all(conn: &Connection) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let n = tx.execute("DELETE FROM telegram_authorized_chats", [])?;
    tx.execute("DELETE FROM telegram_pending_pairings", [])?;
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
    fn first_redeemer_becomes_owner() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let code = generate_pairing_code(&conn, "Wyatt", now).unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));

        let auth = redeem_pairing_code(&conn, 111, &code, now).unwrap();
        assert_eq!(auth.role, Role::Owner);
        assert_eq!(auth.display_name, "Wyatt");
        assert!(is_owner(&conn, 111).unwrap());
    }

    #[test]
    fn second_redeemer_becomes_member() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let c1 = generate_pairing_code(&conn, "Wyatt", now).unwrap();
        redeem_pairing_code(&conn, 111, &c1, now).unwrap();

        let c2 = generate_pairing_code(&conn, "Spouse", now).unwrap();
        let auth = redeem_pairing_code(&conn, 222, &c2, now).unwrap();
        assert_eq!(auth.role, Role::Member);
        assert!(!is_owner(&conn, 222).unwrap());
    }

    #[test]
    fn invalid_code_rejected() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let err = redeem_pairing_code(&conn, 111, "999999", now).unwrap_err();
        assert!(err.to_string().contains("invalid or expired"));
    }

    #[test]
    fn expired_code_rejected_and_cleaned_up() {
        let conn = fresh();
        let issue = datetime!(2026-04-28 12:00:00 UTC);
        let code = generate_pairing_code(&conn, "Wyatt", issue).unwrap();
        // Try to redeem 11 minutes later — past the 10-minute TTL.
        let now = issue + Duration::minutes(11);
        let err = redeem_pairing_code(&conn, 111, &code, now).unwrap_err();
        assert!(err.to_string().contains("expired"));
        // Row was cleaned up
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM telegram_pending_pairings WHERE pairing_code = ?1",
                params![code],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn code_consumed_on_redeem() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let code = generate_pairing_code(&conn, "Wyatt", now).unwrap();
        redeem_pairing_code(&conn, 111, &code, now).unwrap();
        // Try to redeem again with a different chat — should fail.
        let err = redeem_pairing_code(&conn, 222, &code, now).unwrap_err();
        assert!(err.to_string().contains("invalid or expired"));
    }

    #[test]
    fn already_authorized_chat_refused_on_re_redeem() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let c1 = generate_pairing_code(&conn, "Wyatt", now).unwrap();
        redeem_pairing_code(&conn, 111, &c1, now).unwrap();

        let c2 = generate_pairing_code(&conn, "Wyatt 2", now).unwrap();
        let err = redeem_pairing_code(&conn, 111, &c2, now).unwrap_err();
        assert!(err.to_string().contains("already authorized"));
    }

    #[test]
    fn cannot_remove_owner() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let c = generate_pairing_code(&conn, "Wyatt", now).unwrap();
        redeem_pairing_code(&conn, 111, &c, now).unwrap();
        let err = remove_member(&conn, 111).unwrap_err();
        assert!(err.to_string().contains("cannot remove the owner"));
    }

    #[test]
    fn clear_all_removes_owner_and_members_and_pending() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let c1 = generate_pairing_code(&conn, "Wyatt", now).unwrap();
        redeem_pairing_code(&conn, 111, &c1, now).unwrap();
        let c2 = generate_pairing_code(&conn, "Spouse", now).unwrap();
        redeem_pairing_code(&conn, 222, &c2, now).unwrap();
        // A pairing that hasn't been redeemed yet — should also be cleared.
        let _orphan = generate_pairing_code(&conn, "Orphan", now).unwrap();

        let n = clear_all(&conn).unwrap();
        assert_eq!(n, 2, "should report two authorized chats removed");
        assert!(list_members(&conn).unwrap().is_empty());
        let pending: i64 = conn
            .query_row("SELECT COUNT(*) FROM telegram_pending_pairings", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(pending, 0);
    }

    #[test]
    fn clear_all_idempotent_on_empty_db() {
        let conn = fresh();
        let n = clear_all(&conn).unwrap();
        assert_eq!(n, 0);
        // Second call: still no error, still 0.
        let n = clear_all(&conn).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn clear_all_then_first_redeem_becomes_owner_again() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let c1 = generate_pairing_code(&conn, "Wyatt", now).unwrap();
        redeem_pairing_code(&conn, 111, &c1, now).unwrap();
        clear_all(&conn).unwrap();

        let c2 = generate_pairing_code(&conn, "Wyatt2", now).unwrap();
        let auth = redeem_pairing_code(&conn, 999, &c2, now).unwrap();
        assert_eq!(auth.role, Role::Owner);
    }

    #[test]
    fn remove_member_works() {
        let conn = fresh();
        let now = datetime!(2026-04-28 12:00:00 UTC);
        let c1 = generate_pairing_code(&conn, "Wyatt", now).unwrap();
        redeem_pairing_code(&conn, 111, &c1, now).unwrap();
        let c2 = generate_pairing_code(&conn, "Spouse", now).unwrap();
        redeem_pairing_code(&conn, 222, &c2, now).unwrap();

        assert!(remove_member(&conn, 222).unwrap());
        assert!(is_authorized(&conn, 222).unwrap().is_none());
    }
}
