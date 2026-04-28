//! Database connection and forward-only migrations.
//!
//! Migrations are SQL files embedded at compile time. Each file's last
//! statement bumps `PRAGMA user_version`, and the runner applies any
//! files whose version exceeds the current `user_version`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

const MIGRATIONS: &[(u32, &str, &str)] = &[
    (1, "0001_init", include_str!("migrations/0001_init.sql")),
    (
        2,
        "0002_seed_categories",
        include_str!("migrations/0002_seed_categories.sql"),
    ),
];

/// Open a SQLite connection at the given path, creating the file if
/// necessary, and apply runtime PRAGMAs (foreign keys, WAL).
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("creating database parent directory {}", parent.display())
        })?;
    }
    let conn = Connection::open(path)
        .with_context(|| format!("opening database at {}", path.display()))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(conn)
}

/// Open an in-memory connection (for tests).
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

/// Apply any migrations whose version is greater than the connection's
/// current `user_version`. Idempotent.
pub fn migrate(conn: &Connection) -> Result<()> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (version, name, sql) in MIGRATIONS {
        if *version > current {
            tracing::info!(target: "db", "applying migration {} ({})", name, version);
            conn.execute_batch(sql)
                .with_context(|| format!("applying migration {name}"))?;
        }
    }
    Ok(())
}

/// Default on-disk database path for this user.
///
/// - Linux:   `~/.local/share/moneypenny/db.sqlite`
/// - macOS:   `~/Library/Application Support/moneypenny/db.sqlite`
/// - Windows: `%APPDATA%\moneypenny\db.sqlite`
pub fn default_db_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "moneypenny", "moneypenny")
        .context("could not resolve platform data directory")?;
    Ok(dirs.data_dir().join("db.sqlite"))
}
