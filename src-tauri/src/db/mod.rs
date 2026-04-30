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
    (
        3,
        "0003_curate_seed_actives",
        include_str!("migrations/0003_curate_seed_actives.sql"),
    ),
    (
        4,
        "0004_investing_kind",
        include_str!("migrations/0004_investing_kind.sql"),
    ),
    (
        5,
        "0005_seed_electric_water",
        include_str!("migrations/0005_seed_electric_water.sql"),
    ),
];

/// Open a SQLite connection at the given path, creating the file if
/// necessary, and apply runtime PRAGMAs (foreign keys, WAL).
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating database parent directory {}", parent.display()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply migrations 0001 and 0002 only — simulates a database left
    /// behind by v0.1.1, before migration 0003 was introduced.
    fn apply_through_v2(conn: &Connection) -> Result<()> {
        for (version, _name, sql) in MIGRATIONS {
            if *version <= 2 {
                conn.execute_batch(sql)?;
            }
        }
        Ok(())
    }

    fn collect_active_seed_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM categories WHERE is_seed = 1 AND is_active = 1 ORDER BY name",
            )
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    const EXPECTED_DEFAULT_ACTIVE: &[&str] = &[
        "Auto Insurance",
        "Clothing",
        "Dining Out",
        "Entertainment",
        "Groceries",
        "Health Insurance",
        "Household",
        "Internet",
        "Misc",
        "Personal Care",
        "Phone",
        "Rent / Mortgage",
        "Renters / Home Insurance",
        "Transportation / Gas",
    ];

    #[test]
    fn fresh_install_has_curated_default_actives() {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let active = collect_active_seed_names(&conn);
        assert_eq!(active.len(), EXPECTED_DEFAULT_ACTIVE.len());
        for (i, name) in EXPECTED_DEFAULT_ACTIVE.iter().enumerate() {
            assert_eq!(&active[i], name, "mismatch at idx {i}");
        }
    }

    #[test]
    fn upgrade_migration_preserves_engaged_categories() {
        let conn = open_in_memory().unwrap();
        // Simulate a v0.1.1 install: everything seeded as active.
        apply_through_v2(&conn).unwrap();
        let updated = conn
            .execute("UPDATE categories SET is_active = 1 WHERE is_seed = 1", [])
            .unwrap();
        assert_eq!(updated, 29);

        // User logged a Coffee expense.
        let coffee_id: i64 = conn
            .query_row("SELECT id FROM categories WHERE name = 'Coffee'", [], |r| {
                r.get(0)
            })
            .unwrap();
        conn.execute(
            "INSERT INTO expenses (amount_cents, category_id, occurred_at, source) \
             VALUES (500, ?1, '2026-04-01T12:00:00Z', 'manual')",
            [coffee_id],
        )
        .unwrap();

        // User set a target on Pets.
        conn.execute(
            "UPDATE categories SET monthly_target_cents = 5000 WHERE name = 'Pets'",
            [],
        )
        .unwrap();

        // Now apply migrations 0003+0004+0005.
        migrate(&conn).unwrap();
        let v: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 5);

        let active = collect_active_seed_names(&conn);
        let mut expected: Vec<String> = EXPECTED_DEFAULT_ACTIVE
            .iter()
            .map(|s| s.to_string())
            .collect();
        expected.push("Coffee".into());
        expected.push("Pets".into());
        expected.sort();
        assert_eq!(active, expected, "engaged categories must be preserved");
    }

    #[test]
    fn upgrade_migration_does_not_touch_user_categories() {
        let conn = open_in_memory().unwrap();
        apply_through_v2(&conn).unwrap();
        // Force everything seed-active to mimic v0.1.1 baseline.
        conn.execute("UPDATE categories SET is_active = 1 WHERE is_seed = 1", [])
            .unwrap();
        // User created a custom category and left it inactive.
        conn.execute(
            "INSERT INTO categories (name, kind, is_recurring, is_active, is_seed) \
             VALUES ('Boats', 'variable', 0, 0, 0)",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let row: (i64, i64) = conn
            .query_row(
                "SELECT is_active, is_seed FROM categories WHERE name = 'Boats'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, (0, 0), "user category must be untouched");
    }

    #[test]
    fn investing_seed_categories_present_and_inactive_by_default() {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT name, is_active FROM categories \
                 WHERE kind = 'investing' AND is_seed = 1 ORDER BY name",
            )
            .unwrap();
        let rows: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let names: Vec<_> = rows.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["401k", "Investing", "Roth IRA", "Savings"]);
        for (name, active) in &rows {
            assert_eq!(*active, 0, "{name} should be inactive by default");
        }
    }

    #[test]
    fn migration_0004_preserves_existing_user_categories() {
        // Simulate a v0.1.3 database: apply 0001+0002+0003, add a user
        // category and a couple of expenses, then apply 0004.
        let conn = open_in_memory().unwrap();
        for (version, _name, sql) in MIGRATIONS {
            if *version <= 3 {
                conn.execute_batch(sql).unwrap();
            }
        }
        conn.execute(
            "INSERT INTO categories (name, kind, is_recurring, is_active, is_seed) \
             VALUES ('Boats', 'variable', 0, 1, 0)",
            [],
        )
        .unwrap();
        let boat_id: i64 = conn
            .query_row("SELECT id FROM categories WHERE name = 'Boats'", [], |r| {
                r.get(0)
            })
            .unwrap();
        conn.execute(
            "INSERT INTO expenses (amount_cents, category_id, occurred_at, source) \
             VALUES (1000, ?1, '2026-04-01T12:00:00Z', 'manual')",
            [boat_id],
        )
        .unwrap();

        // Apply 0004 (and any later migrations).
        migrate(&conn).unwrap();

        // Boats survives with its expense intact.
        let still_there: i64 = conn
            .query_row(
                "SELECT id FROM categories WHERE name = 'Boats' AND is_seed = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still_there, boat_id);
        let expense_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM expenses WHERE category_id = ?1",
                [boat_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(expense_count, 1);

        // Investing kind now accepted.
        conn.execute(
            "INSERT INTO categories (name, kind, is_recurring, is_active, is_seed) \
             VALUES ('Brokerage', 'investing', 0, 1, 0)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let active_first = collect_active_seed_names(&conn);
        migrate(&conn).unwrap();
        let active_second = collect_active_seed_names(&conn);
        assert_eq!(active_first, active_second);
    }
}
