use crate::sync::clock::HybridClock;
use crate::sync::crdt::{FileVersion, VersionVector};
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use tracing::info;

/// SQLite-backed metadata database for local file index.
pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).context("opening database")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("opening in-memory database")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS files (
                path            TEXT PRIMARY KEY,
                content_hash    TEXT NOT NULL,
                size            INTEGER NOT NULL,
                hlc_wall_ms     INTEGER NOT NULL,
                hlc_counter     INTEGER NOT NULL,
                hlc_node_id     INTEGER NOT NULL,
                version_vector  TEXT NOT NULL,
                deleted         INTEGER NOT NULL DEFAULT 0,
                mode            INTEGER,
                updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS sync_state (
                peer_id         TEXT PRIMARY KEY,
                last_sync_hlc   TEXT NOT NULL,
                updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS conflicts (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                path            TEXT NOT NULL,
                local_hash      TEXT NOT NULL,
                remote_hash     TEXT NOT NULL,
                local_version   TEXT NOT NULL,
                remote_version  TEXT NOT NULL,
                resolved        INTEGER NOT NULL DEFAULT 0,
                created_at      TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_files_hash ON files(content_hash);
            CREATE INDEX IF NOT EXISTS idx_conflicts_path ON conflicts(path);
            ",
        )?;
        info!("database migrated");
        Ok(())
    }

    /// Insert or update a file version in the index.
    pub fn upsert_file(&self, fv: &FileVersion) -> Result<()> {
        let vv_json = serde_json::to_string(&fv.version)?;
        self.conn.execute(
            "INSERT INTO files (path, content_hash, size, hlc_wall_ms, hlc_counter, hlc_node_id, version_vector, deleted, mode)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(path) DO UPDATE SET
                content_hash = excluded.content_hash,
                size = excluded.size,
                hlc_wall_ms = excluded.hlc_wall_ms,
                hlc_counter = excluded.hlc_counter,
                hlc_node_id = excluded.hlc_node_id,
                version_vector = excluded.version_vector,
                deleted = excluded.deleted,
                mode = excluded.mode,
                updated_at = datetime('now')",
            params![
                fv.path,
                fv.content_hash,
                fv.size as i64,
                fv.hlc.wall_ms as i64,
                fv.hlc.counter as i64,
                fv.hlc.node_id as i64,
                vv_json,
                fv.deleted as i32,
                fv.mode.map(|m| m as i64),
            ],
        )?;
        Ok(())
    }

    /// Retrieve file metadata by path.
    pub fn get_file(&self, path: &str) -> Result<Option<FileVersion>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, content_hash, size, hlc_wall_ms, hlc_counter, hlc_node_id,
                    version_vector, deleted, mode
             FROM files WHERE path = ?1",
        )?;
        let mut rows = stmt.query_map(params![path], |row| {
            Ok(RawRow {
                path: row.get(0)?,
                content_hash: row.get(1)?,
                size: row.get::<_, i64>(2)?,
                hlc_wall_ms: row.get::<_, i64>(3)?,
                hlc_counter: row.get::<_, i64>(4)?,
                hlc_node_id: row.get::<_, i64>(5)?,
                version_vector: row.get::<_, String>(6)?,
                deleted: row.get::<_, i32>(7)?,
                mode: row.get::<_, Option<i64>>(8)?,
            })
        })?;
        match rows.next() {
            Some(Ok(raw)) => Ok(Some(raw.into_file_version()?)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// List all (non-deleted) files.
    pub fn list_files(&self) -> Result<Vec<FileVersion>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, content_hash, size, hlc_wall_ms, hlc_counter, hlc_node_id,
                    version_vector, deleted, mode
             FROM files WHERE deleted = 0
             ORDER BY path",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RawRow {
                path: row.get(0)?,
                content_hash: row.get(1)?,
                size: row.get::<_, i64>(2)?,
                hlc_wall_ms: row.get::<_, i64>(3)?,
                hlc_counter: row.get::<_, i64>(4)?,
                hlc_node_id: row.get::<_, i64>(5)?,
                version_vector: row.get::<_, String>(6)?,
                deleted: row.get::<_, i32>(7)?,
                mode: row.get::<_, Option<i64>>(8)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?.into_file_version()?);
        }
        Ok(out)
    }

    /// List all files (including deleted tombstones) for sync diffing.
    pub fn list_all_versions(&self) -> Result<Vec<FileVersion>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, content_hash, size, hlc_wall_ms, hlc_counter, hlc_node_id,
                    version_vector, deleted, mode
             FROM files ORDER BY path",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RawRow {
                path: row.get(0)?,
                content_hash: row.get(1)?,
                size: row.get::<_, i64>(2)?,
                hlc_wall_ms: row.get::<_, i64>(3)?,
                hlc_counter: row.get::<_, i64>(4)?,
                hlc_node_id: row.get::<_, i64>(5)?,
                version_vector: row.get::<_, String>(6)?,
                deleted: row.get::<_, i32>(7)?,
                mode: row.get::<_, Option<i64>>(8)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?.into_file_version()?);
        }
        Ok(out)
    }

    /// Record a conflict for later resolution.
    pub fn record_conflict(
        &self,
        path: &str,
        local: &FileVersion,
        remote: &FileVersion,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO conflicts (path, local_hash, remote_hash, local_version, remote_version)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                path,
                local.content_hash,
                remote.content_hash,
                serde_json::to_string(&local.version)?,
                serde_json::to_string(&remote.version)?,
            ],
        )?;
        Ok(())
    }

    /// List unresolved conflicts.
    pub fn list_conflicts(&self) -> Result<Vec<ConflictRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, local_hash, remote_hash, created_at
             FROM conflicts WHERE resolved = 0 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ConflictRecord {
                id: row.get(0)?,
                path: row.get(1)?,
                local_hash: row.get(2)?,
                remote_hash: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Mark a conflict as resolved.
    pub fn resolve_conflict(&self, conflict_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE conflicts SET resolved = 1 WHERE id = ?1",
            params![conflict_id],
        )?;
        Ok(())
    }

    /// Delete a file (mark as tombstone).
    pub fn mark_deleted(&self, path: &str, hlc: HybridClock, node_id: u64) -> Result<()> {
        if let Some(mut fv) = self.get_file(path)? {
            fv.deleted = true;
            fv.hlc = hlc;
            fv.version.increment(node_id);
            self.upsert_file(&fv)?;
        }
        Ok(())
    }
}

struct RawRow {
    path: String,
    content_hash: String,
    size: i64,
    hlc_wall_ms: i64,
    hlc_counter: i64,
    hlc_node_id: i64,
    version_vector: String,
    deleted: i32,
    mode: Option<i64>,
}

impl RawRow {
    fn into_file_version(self) -> Result<FileVersion, anyhow::Error> {
        let vv: VersionVector = serde_json::from_str(&self.version_vector)?;
        Ok(FileVersion {
            path: self.path,
            content_hash: self.content_hash,
            size: self.size as u64,
            hlc: HybridClock {
                wall_ms: self.hlc_wall_ms as u64,
                counter: self.hlc_counter as u32,
                node_id: self.hlc_node_id as u64,
            },
            version: vv,
            deleted: self.deleted != 0,
            mode: self.mode.map(|m| m as u32),
        })
    }
}

#[derive(Debug)]
pub struct ConflictRecord {
    pub id: i64,
    pub path: String,
    pub local_hash: String,
    pub remote_hash: String,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_fv(path: &str) -> FileVersion {
        FileVersion {
            path: path.to_string(),
            content_hash: "abc123".into(),
            size: 42,
            hlc: HybridClock {
                wall_ms: 1000,
                counter: 0,
                node_id: 1,
            },
            version: VersionVector::new(),
            deleted: false,
            mode: Some(0o644),
        }
    }

    #[test]
    fn roundtrip() -> Result<()> {
        let db = Database::open_in_memory()?;
        let fv = test_fv("hello.txt");
        db.upsert_file(&fv)?;
        let got = db.get_file("hello.txt")?.unwrap();
        assert_eq!(got.path, "hello.txt");
        assert_eq!(got.content_hash, "abc123");
        assert_eq!(got.size, 42);
        Ok(())
    }

    #[test]
    fn list_excludes_deleted() -> Result<()> {
        let db = Database::open_in_memory()?;
        db.upsert_file(&test_fv("a.txt"))?;
        let mut deleted = test_fv("b.txt");
        deleted.deleted = true;
        db.upsert_file(&deleted)?;
        let files = db.list_files()?;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "a.txt");
        Ok(())
    }
}
