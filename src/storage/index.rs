use crate::storage::db::Database;
use crate::sync::clock::HybridClock;
use crate::sync::crdt::FileVersion;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Walks the mesh root directory and indexes files into the database.
pub struct FileIndex {
    root: PathBuf,
    node_id: u64,
}

impl FileIndex {
    pub fn new(root: PathBuf, node_id: u64) -> Self {
        Self { root, node_id }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Full scan of the mesh root, updating the DB for any changes.
    pub fn full_scan(&self, db: &Database, clock: &mut HybridClock) -> Result<u64> {
        let mut count = 0u64;
        self.walk_dir(&self.root, db, clock, &mut count)?;
        info!(count, "full scan complete");
        Ok(count)
    }

    fn walk_dir(
        &self,
        dir: &Path,
        db: &Database,
        clock: &mut HybridClock,
        count: &mut u64,
    ) -> Result<()> {
        let entries =
            fs::read_dir(dir).with_context(|| format!("reading directory {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Skip hidden files/dirs and the .datamesh config dir.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') {
                    continue;
                }
            }

            if path.is_dir() {
                self.walk_dir(&path, db, clock, count)?;
            } else if path.is_file() {
                self.index_file(&path, db, clock)?;
                *count += 1;
            }
        }
        Ok(())
    }

    /// Index a single file: hash it, compare with DB, update if changed.
    pub fn index_file(
        &self,
        abs_path: &Path,
        db: &Database,
        clock: &mut HybridClock,
    ) -> Result<Option<FileVersion>> {
        let rel_path = abs_path
            .strip_prefix(&self.root)
            .with_context(|| {
                format!(
                    "{} not under root {}",
                    abs_path.display(),
                    self.root.display()
                )
            })?
            .to_string_lossy()
            .to_string();

        let metadata =
            fs::metadata(abs_path).with_context(|| format!("stat {}", abs_path.display()))?;
        let size = metadata.len();
        let content_hash = hash_file(abs_path)?;

        // Check existing entry.
        if let Some(existing) = db.get_file(&rel_path)? {
            if existing.content_hash == content_hash && !existing.deleted {
                debug!(path = %rel_path, "unchanged");
                return Ok(None);
            }
        }

        let hlc = clock.tick();
        let mut version = db
            .get_file(&rel_path)?
            .map(|f| f.version)
            .unwrap_or_default();
        version.increment(self.node_id);

        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            Some(metadata.permissions().mode())
        };
        #[cfg(not(unix))]
        let mode = None;

        let fv = FileVersion {
            path: rel_path,
            content_hash,
            size,
            hlc,
            version,
            deleted: false,
            mode,
        };
        db.upsert_file(&fv)?;
        debug!(path = %fv.path, hash = %fv.content_hash, "indexed");
        Ok(Some(fv))
    }

    /// Mark a file as deleted in the index.
    pub fn mark_deleted(
        &self,
        abs_path: &Path,
        db: &Database,
        clock: &mut HybridClock,
    ) -> Result<()> {
        let rel_path = abs_path
            .strip_prefix(&self.root)
            .with_context(|| {
                format!(
                    "{} not under root {}",
                    abs_path.display(),
                    self.root.display()
                )
            })?
            .to_string_lossy()
            .to_string();

        let hlc = clock.tick();
        db.mark_deleted(&rel_path, hlc, self.node_id)?;
        warn!(path = %rel_path, "marked deleted");
        Ok(())
    }
}

/// BLAKE3 hash of file contents, returned as hex string.
pub fn hash_file(path: &Path) -> Result<String> {
    let data = fs::read(path).with_context(|| format!("reading file {}", path.display()))?;
    Ok(blake3::hash(&data).to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn hash_file_works() -> Result<()> {
        let dir = TempDir::new()?;
        let file = dir.path().join("test.txt");
        fs::write(&file, "hello world")?;
        let h1 = hash_file(&file)?;
        let h2 = hash_file(&file)?;
        assert_eq!(h1, h2);
        assert!(!h1.is_empty());
        Ok(())
    }

    #[test]
    fn full_scan_indexes_files() -> Result<()> {
        let dir = TempDir::new()?;
        fs::write(dir.path().join("a.txt"), "aaa")?;
        fs::write(dir.path().join("b.txt"), "bbb")?;
        fs::create_dir(dir.path().join("sub"))?;
        fs::write(dir.path().join("sub/c.txt"), "ccc")?;

        let db = Database::open_in_memory()?;
        let idx = FileIndex::new(dir.path().to_path_buf(), 1);
        let mut clock = HybridClock::now(1);
        let count = idx.full_scan(&db, &mut clock)?;
        assert_eq!(count, 3);

        let files = db.list_files()?;
        assert_eq!(files.len(), 3);
        Ok(())
    }
}
