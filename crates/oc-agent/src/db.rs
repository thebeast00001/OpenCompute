//! The agent's SQLite database.
//!
//! Schema changes are numbered SQL files in `migrations/`, applied in order
//! when the database is opened. A database written by a newer release is
//! refused instead of being guessed at.

use std::net::SocketAddr;
use std::path::Path;

use oc_core::DeviceId;
use rusqlite::{Connection, OptionalExtension, params};

/// Every migration this build knows, oldest first. Never edit one that has
/// shipped; add a new file.
const MIGRATIONS: &[(u32, &str)] = &[(1, include_str!("../migrations/0001_init.sql"))];

/// A database that could not be opened or used.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// SQLite reported an error.
    #[error("database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The file was written by a newer release.
    #[error(
        "the database is at schema version {found}, but this build only knows \
         version {known}. Update OpenCompute, or use a different data directory."
    )]
    TooNew {
        /// Version found in the file.
        found: u32,
        /// Newest version this build can read.
        known: u32,
    },
}

/// One stored roster entry, exactly as it was signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOp {
    /// BLAKE3 hash of `body`.
    pub op_id: [u8; 32],
    /// The encoded operation.
    pub body: Vec<u8>,
    /// Signature over `body` by the cluster root key.
    pub signature: Vec<u8>,
}

/// What the database remembers about another device.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StoredDevice {
    /// Encoded static inventory, if one was ever received.
    pub inventory: Option<Vec<u8>>,
    /// When the inventory was received.
    pub inventory_at_ms: Option<u64>,
    /// When the device was last heard from.
    pub last_seen_ms: Option<u64>,
    /// Addresses the device was last reached at.
    pub addrs: Vec<SocketAddr>,
}

/// The agent's database.
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Opens (creating if needed) the database file and brings its schema up
    /// to date.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
        // WAL keeps readers from blocking the writer and survives a crash
        // mid-write.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::prepare(conn)
    }

    /// Opens a private in-memory database. For tests.
    pub fn open_in_memory() -> Result<Self, DbError> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(conn: Connection) -> Result<Self, DbError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut db = Self { conn };
        db.migrate(MIGRATIONS)?;
        Ok(db)
    }

    fn migrate(&mut self, migrations: &[(u32, &str)]) -> Result<(), DbError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 version       INTEGER PRIMARY KEY,
                 applied_at_ms INTEGER NOT NULL
             )",
        )?;
        let found = self.schema_version()?;
        let known = migrations.last().map_or(0, |(v, _)| *v);
        if found > known {
            return Err(DbError::TooNew { found, known });
        }
        for (version, sql) in migrations.iter().filter(|(v, _)| *v > found) {
            // Each migration and its bookkeeping row commit together, so a
            // crash leaves the schema at a whole version.
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
                params![version, now_ms()],
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    /// The newest migration applied to this database.
    pub fn schema_version(&self) -> Result<u32, DbError> {
        let version: Option<u32> =
            self.conn
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                    row.get(0)
                })?;
        Ok(version.unwrap_or(0))
    }

    // ---------------------------------------------------------------- meta

    /// Reads a single stored value.
    pub fn meta(&self, key: &str) -> Result<Option<Vec<u8>>, DbError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Stores a single value, replacing any earlier one.
    pub fn set_meta(&self, key: &str, value: &[u8]) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------- roster

    /// Adds roster entries. Entries already present are left as they are.
    pub fn insert_roster_ops(&mut self, ops: &[StoredOp]) -> Result<(), DbError> {
        let tx = self.conn.transaction()?;
        {
            let mut insert = tx.prepare(
                "INSERT OR IGNORE INTO roster_ops (op_id, body, signature, received_at_ms)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            let now = now_ms();
            for op in ops {
                insert.execute(params![&op.op_id[..], op.body, op.signature, now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Every stored roster entry.
    pub fn roster_ops(&self) -> Result<Vec<StoredOp>, DbError> {
        let mut select = self
            .conn
            .prepare("SELECT op_id, body, signature FROM roster_ops ORDER BY op_id")?;
        let rows = select.query_map([], |row| {
            let op_id: Vec<u8> = row.get(0)?;
            Ok((op_id, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?))
        })?;
        let mut ops = Vec::new();
        for row in rows {
            let (op_id, body, signature) = row?;
            // A row with a malformed id cannot have been written by this
            // code; skip it and let signature checks judge the rest.
            if let Ok(op_id) = <[u8; 32]>::try_from(op_id.as_slice()) {
                ops.push(StoredOp {
                    op_id,
                    body,
                    signature,
                });
            }
        }
        Ok(ops)
    }

    // ------------------------------------------------------------- devices

    /// Stores a device's static inventory.
    pub fn set_device_inventory(&self, device: DeviceId, inventory: &[u8]) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO devices (device_id, inventory, inventory_at_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT (device_id) DO UPDATE
             SET inventory = excluded.inventory, inventory_at_ms = excluded.inventory_at_ms",
            params![&device.as_bytes()[..], inventory, now_ms()],
        )?;
        Ok(())
    }

    /// Records that a device was heard from, and where.
    pub fn set_device_seen(
        &self,
        device: DeviceId,
        seen_ms: u64,
        addrs: &[SocketAddr],
    ) -> Result<(), DbError> {
        let addrs = addrs
            .iter()
            .map(SocketAddr::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        // Keep the old addresses when none are known this time.
        self.conn.execute(
            "INSERT INTO devices (device_id, last_seen_ms, addrs) VALUES (?1, ?2, ?3)
             ON CONFLICT (device_id) DO UPDATE
             SET last_seen_ms = excluded.last_seen_ms,
                 addrs = CASE WHEN excluded.addrs = '' THEN addrs ELSE excluded.addrs END",
            params![&device.as_bytes()[..], to_sql_ms(seen_ms), addrs],
        )?;
        Ok(())
    }

    /// What is remembered about one device.
    pub fn device(&self, device: DeviceId) -> Result<Option<StoredDevice>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT inventory, inventory_at_ms, last_seen_ms, addrs
                 FROM devices WHERE device_id = ?1",
                [&device.as_bytes()[..]],
                |row| {
                    let addrs: String = row.get(3)?;
                    Ok(StoredDevice {
                        inventory: row.get(0)?,
                        inventory_at_ms: from_sql_ms(row.get(1)?),
                        last_seen_ms: from_sql_ms(row.get(2)?),
                        addrs: addrs
                            .split_whitespace()
                            .filter_map(|a| a.parse().ok())
                            .collect(),
                    })
                },
            )
            .optional()?)
    }

    /// Forgets everything stored about a device.
    pub fn delete_device(&self, device: DeviceId) -> Result<(), DbError> {
        self.conn.execute(
            "DELETE FROM devices WHERE device_id = ?1",
            [&device.as_bytes()[..]],
        )?;
        Ok(())
    }
}

/// SQLite integers are signed 64-bit, so timestamps cross the boundary as
/// `i64`. Milliseconds since 1970 fit for the next 292 million years.
fn to_sql_ms(ms: u64) -> i64 {
    i64::try_from(ms).unwrap_or(i64::MAX)
}

fn from_sql_ms(ms: Option<i64>) -> Option<u64> {
    ms.and_then(|ms| u64::try_from(ms).ok())
}

fn now_ms() -> i64 {
    use oc_core::time::Clock;
    to_sql_ms(oc_core::time::SystemClock.now_unix_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(n: u8) -> DeviceId {
        DeviceId::from_bytes([n; 32])
    }

    #[test]
    fn a_new_database_is_at_the_latest_version() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(
            db.schema_version().unwrap(),
            MIGRATIONS.last().unwrap().0,
            "every migration should have been applied"
        );
    }

    #[test]
    fn migration_numbers_increase_by_one() {
        for (index, (version, sql)) in MIGRATIONS.iter().enumerate() {
            assert_eq!(*version as usize, index + 1);
            assert!(!sql.trim().is_empty());
        }
    }

    #[test]
    fn reopening_keeps_data_and_applies_nothing_twice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        {
            let db = Db::open(&path).unwrap();
            db.set_meta("cluster_id", b"abc").unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.meta("cluster_id").unwrap().as_deref(), Some(&b"abc"[..]));
        let applied: u32 = db
            .conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(applied as usize, MIGRATIONS.len());
    }

    #[test]
    fn later_migrations_apply_on_top_of_earlier_ones() {
        let mut db = Db::open_in_memory().unwrap();
        db.set_meta("kept", b"yes").unwrap();
        let mut extended = MIGRATIONS.to_vec();
        let next = extended.last().unwrap().0 + 1;
        extended.push((next, "CREATE TABLE added_later (id INTEGER PRIMARY KEY);"));
        db.migrate(&extended).unwrap();
        assert_eq!(db.schema_version().unwrap(), next);
        assert_eq!(db.meta("kept").unwrap().as_deref(), Some(&b"yes"[..]));
        db.conn
            .execute("INSERT INTO added_later (id) VALUES (1)", [])
            .unwrap();
    }

    #[test]
    fn a_failed_migration_changes_nothing() {
        let mut db = Db::open_in_memory().unwrap();
        let mut broken = MIGRATIONS.to_vec();
        let next = broken.last().unwrap().0 + 1;
        broken.push((next, "CREATE TABLE half_done (id INTEGER); NOT VALID SQL;"));
        assert!(db.migrate(&broken).is_err());
        assert_eq!(db.schema_version().unwrap(), next - 1);
        let exists: u32 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'half_done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0, "the half-applied table must be rolled back");
    }

    #[test]
    fn a_database_from_a_newer_release_is_refused() {
        let mut db = Db::open_in_memory().unwrap();
        db.conn
            .execute(
                "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (999, 0)",
                [],
            )
            .unwrap();
        let err = db.migrate(MIGRATIONS).unwrap_err();
        assert!(matches!(err, DbError::TooNew { found: 999, .. }), "{err}");
    }

    #[test]
    fn meta_values_are_replaced() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.meta("k").unwrap().is_none());
        db.set_meta("k", b"1").unwrap();
        db.set_meta("k", b"2").unwrap();
        assert_eq!(db.meta("k").unwrap().as_deref(), Some(&b"2"[..]));
    }

    #[test]
    fn roster_ops_are_stored_once() {
        let mut db = Db::open_in_memory().unwrap();
        let op = |n: u8| StoredOp {
            op_id: [n; 32],
            body: vec![n, n],
            signature: vec![n; 64],
        };
        db.insert_roster_ops(&[op(2), op(1)]).unwrap();
        db.insert_roster_ops(&[op(1), op(3)]).unwrap();
        assert_eq!(db.roster_ops().unwrap(), vec![op(1), op(2), op(3)]);
    }

    #[test]
    fn device_records_merge_instead_of_overwriting() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.device(device(1)).unwrap().is_none());

        let addr: SocketAddr = "192.0.2.7:7422".parse().unwrap();
        db.set_device_seen(device(1), 1_000, &[addr]).unwrap();
        db.set_device_inventory(device(1), b"inventory").unwrap();
        // A later sighting with no known address keeps the old one.
        db.set_device_seen(device(1), 2_000, &[]).unwrap();

        let stored = db.device(device(1)).unwrap().unwrap();
        assert_eq!(stored.inventory.as_deref(), Some(&b"inventory"[..]));
        assert!(stored.inventory_at_ms.is_some());
        assert_eq!(stored.last_seen_ms, Some(2_000));
        assert_eq!(stored.addrs, vec![addr]);

        db.delete_device(device(1)).unwrap();
        assert!(db.device(device(1)).unwrap().is_none());
    }
}
