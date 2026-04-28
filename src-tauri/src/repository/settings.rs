//! Key-value settings backed by the `settings` table.
//!
//! Used for non-secret app config (currency, locale, LLM provider
//! choice, Ollama endpoint/model, setup-complete flag, etc.). Secrets
//! (bot token, API key) live in the OS keychain via the `secrets`
//! module, NOT here.

use anyhow::Result;
use rusqlite::{params, Connection};

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    let row: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .ok();
    Ok(row)
}

pub fn set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, key: &str) -> Result<bool> {
    let n = conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
    Ok(n > 0)
}

pub fn get_or_default(conn: &Connection, key: &str, default_value: &str) -> Result<String> {
    Ok(get(conn, key)?.unwrap_or_else(|| default_value.to_string()))
}

// ---------------------------------------------------------------------
// Well-known keys (kept here so typos don't spread).
// ---------------------------------------------------------------------

pub mod keys {
    pub const DEFAULT_CURRENCY: &str = "default_currency";
    pub const LOCALE: &str = "locale";
    pub const LLM_PROVIDER: &str = "llm_provider"; // "anthropic" | "ollama"
    pub const OLLAMA_ENDPOINT: &str = "ollama_endpoint";
    pub const OLLAMA_MODEL: &str = "ollama_model";
    pub const ANTHROPIC_MODEL: &str = "anthropic_model";
    pub const SETUP_COMPLETE: &str = "setup_complete"; // "1" | "0"
    pub const SETUP_STEP: &str = "setup_step"; // last completed step number
    pub const PRIVACY_MODE: &str = "privacy_mode"; // "1" | "0" (off by default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn fresh() -> Connection {
        let c = db::open_in_memory().unwrap();
        db::migrate(&c).unwrap();
        c
    }

    #[test]
    fn round_trip() {
        let conn = fresh();
        assert_eq!(get(&conn, "x").unwrap(), None);
        set(&conn, "x", "hello").unwrap();
        assert_eq!(get(&conn, "x").unwrap().as_deref(), Some("hello"));
    }

    #[test]
    fn upsert() {
        let conn = fresh();
        set(&conn, "x", "a").unwrap();
        set(&conn, "x", "b").unwrap();
        assert_eq!(get(&conn, "x").unwrap().as_deref(), Some("b"));
    }

    #[test]
    fn delete_removes() {
        let conn = fresh();
        set(&conn, "x", "a").unwrap();
        assert!(delete(&conn, "x").unwrap());
        assert_eq!(get(&conn, "x").unwrap(), None);
    }

    #[test]
    fn or_default() {
        let conn = fresh();
        assert_eq!(get_or_default(&conn, "x", "def").unwrap(), "def");
        set(&conn, "x", "actual").unwrap();
        assert_eq!(get_or_default(&conn, "x", "def").unwrap(), "actual");
    }
}
