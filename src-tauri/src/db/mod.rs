//! Database connection and forward-only migrations.
//!
//! Migrations are SQL files embedded at compile time. Each file's last
//! statement bumps `PRAGMA user_version`, and the runner applies any
//! files whose version exceeds the current `user_version`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

/// One forward-only migration. `recreate = true` marks migrations whose
/// SQL performs a table-recreate dance and therefore needs SQLite's
/// foreign-key enforcement disabled around the swap. The runner manages
/// the surrounding `PRAGMA foreign_keys` statements so that the schema
/// changes themselves can be wrapped in a transaction (SQLite forbids
/// `PRAGMA foreign_keys` inside a transaction).
struct Migration {
    version: u32,
    name: &'static str,
    sql: &'static str,
    recreate: bool,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_init",
        sql: include_str!("migrations/0001_init.sql"),
        recreate: false,
    },
    Migration {
        version: 2,
        name: "0002_seed_categories",
        sql: include_str!("migrations/0002_seed_categories.sql"),
        recreate: false,
    },
    Migration {
        version: 3,
        name: "0003_curate_seed_actives",
        sql: include_str!("migrations/0003_curate_seed_actives.sql"),
        recreate: false,
    },
    Migration {
        version: 4,
        name: "0004_investing_kind",
        sql: include_str!("migrations/0004_investing_kind.sql"),
        recreate: true,
    },
    Migration {
        version: 5,
        name: "0005_seed_electric_water",
        sql: include_str!("migrations/0005_seed_electric_water.sql"),
        recreate: false,
    },
    Migration {
        version: 6,
        name: "0006_refunds",
        sql: include_str!("migrations/0006_refunds.sql"),
        recreate: true,
    },
    Migration {
        version: 7,
        name: "0007_scheduled_jobs",
        sql: include_str!("migrations/0007_scheduled_jobs.sql"),
        recreate: false,
    },
    Migration {
        version: 8,
        name: "0008_recurring_rules",
        sql: include_str!("migrations/0008_recurring_rules.sql"),
        recreate: false,
    },
    Migration {
        version: 9,
        name: "0009_budget_alert_state",
        sql: include_str!("migrations/0009_budget_alert_state.sql"),
        recreate: false,
    },
    Migration {
        version: 10,
        name: "0010_llm_usage",
        sql: include_str!("migrations/0010_llm_usage.sql"),
        recreate: false,
    },
    Migration {
        version: 11,
        name: "0011_investment_balances",
        sql: include_str!("migrations/0011_investment_balances.sql"),
        recreate: false,
    },
    Migration {
        version: 12,
        name: "0012_csv_import_profiles",
        sql: include_str!("migrations/0012_csv_import_profiles.sql"),
        recreate: false,
    },
    Migration {
        version: 13,
        name: "0013_merchant_rules",
        sql: include_str!("migrations/0013_merchant_rules.sql"),
        recreate: false,
    },
    Migration {
        version: 14,
        name: "0014_csv_expense_source",
        sql: include_str!("migrations/0014_csv_expense_source.sql"),
        recreate: true,
    },
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
///
/// Each migration is wrapped in its own transaction. The migration's
/// final statement bumps `PRAGMA user_version`, so on partial failure
/// (disk full, OOM, panic) the rollback is atomic with the schema
/// rollback — no half-applied schema with unbumped `user_version`,
/// which would brick subsequent launches.
///
/// `recreate: true` migrations need foreign-key enforcement disabled
/// around the table-recreate swap; SQLite forbids `PRAGMA foreign_keys`
/// inside a transaction, so the runner toggles the pragma *outside* the
/// wrapping tx. Any embedded `PRAGMA foreign_keys` statements inside the
/// SQL files become inert no-ops within the tx.
pub fn migrate(conn: &Connection) -> Result<()> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for m in MIGRATIONS {
        if m.version > current {
            apply_migration(conn, m)?;
        }
    }
    Ok(())
}

fn apply_migration(conn: &Connection, m: &Migration) -> Result<()> {
    tracing::info!(target: "db", "applying migration {} ({})", m.name, m.version);
    if m.recreate {
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .with_context(|| format!("disabling FKs for {}", m.name))?;
        let outcome = run_migration_in_tx(conn, m);
        // Always restore FK enforcement, even on failure. db::open also
        // re-asserts foreign_keys=ON on every fresh connection, so a
        // missed restore here self-heals on the next launch.
        let _ = conn.execute_batch("PRAGMA foreign_keys = ON;");
        outcome
    } else {
        run_migration_in_tx(conn, m)
    }
}

fn run_migration_in_tx(conn: &Connection, m: &Migration) -> Result<()> {
    let tx = conn
        .unchecked_transaction()
        .with_context(|| format!("starting tx for {}", m.name))?;
    tx.execute_batch(m.sql)
        .with_context(|| format!("applying migration {}", m.name))?;
    tx.commit()
        .with_context(|| format!("committing migration {}", m.name))?;
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
        for m in MIGRATIONS {
            if m.version <= 2 {
                conn.execute_batch(m.sql)?;
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

        // Now apply migrations 0003 through 0014 (latest).
        migrate(&conn).unwrap();
        let v: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 14);

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
        for m in MIGRATIONS {
            if m.version <= 3 {
                conn.execute_batch(m.sql).unwrap();
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

    /// A failure inside a wrapped migration must roll back cleanly: the
    /// schema is unchanged and `user_version` is not advanced, so the
    /// next migration attempt can re-run from a clean state. This guards
    /// against the v0.3.7-and-earlier behavior where a partial failure
    /// in 0004/0006/0011/0014 would orphan a `*_new` table and brick
    /// every subsequent launch.
    #[test]
    fn failed_migration_rolls_back_atomically() {
        let conn = open_in_memory().unwrap();
        // Take the DB to a known-good v3 state.
        for m in MIGRATIONS {
            if m.version <= 3 {
                conn.execute_batch(m.sql).unwrap();
            }
        }
        let v3: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v3, 3);
        let categories_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM categories", [], |r| r.get(0))
            .unwrap();

        // Construct a synthetic broken migration that does meaningful
        // work, then deliberately errors (insert into a non-existent
        // table). With the wrapping tx, the meaningful work is rolled
        // back. Without the wrapping tx, the orphan row would persist.
        let broken = Migration {
            version: 99,
            name: "9999_broken_test_migration",
            sql: "INSERT INTO categories (name, kind, is_recurring, is_active, is_seed) \
                  VALUES ('OrphanShouldRollback', 'variable', 0, 1, 0); \
                  INSERT INTO does_not_exist (id) VALUES (1); \
                  PRAGMA user_version = 99;",
            recreate: false,
        };
        let result = apply_migration(&conn, &broken);
        assert!(result.is_err(), "broken migration must fail");

        // Schema is unchanged.
        let v_after: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v_after, 3, "user_version must not advance on failure");
        let categories_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM categories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            categories_after, categories_before,
            "rolled-back insert must leave row count unchanged"
        );
        let orphan_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM categories WHERE name = 'OrphanShouldRollback'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            orphan_count, 0,
            "first INSERT must roll back with the failure"
        );
    }

    /// Same guarantee for `recreate: true` migrations: the FK pragma is
    /// toggled outside the tx, the schema rebuild is inside, and a
    /// failure rolls back the rebuild cleanly without leaving an orphan
    /// `*_new` table behind.
    #[test]
    fn failed_recreate_migration_rolls_back_without_orphan_table() {
        let conn = open_in_memory().unwrap();
        for m in MIGRATIONS {
            if m.version <= 3 {
                conn.execute_batch(m.sql).unwrap();
            }
        }
        // Recreate-style broken migration: build a `*_new` table, then
        // error before the rename. The wrapping tx must drop the new
        // table on rollback, so subsequent migrations don't hit
        // "table already exists".
        let broken = Migration {
            version: 99,
            name: "9999_broken_recreate_test",
            sql: "CREATE TABLE categories_new (id INTEGER PRIMARY KEY, name TEXT); \
                  INSERT INTO categories_new (id, name) SELECT id, name FROM categories; \
                  INSERT INTO does_not_exist (id) VALUES (1); \
                  PRAGMA user_version = 99;",
            recreate: true,
        };
        let result = apply_migration(&conn, &broken);
        assert!(result.is_err(), "broken recreate migration must fail");

        let new_table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'table' AND name = 'categories_new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            new_table_exists, 0,
            "categories_new must not survive the failed recreate"
        );
        // FK enforcement is restored even on failure.
        let fk_on: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk_on, 1, "FK enforcement must be re-enabled after failure");
    }
}
