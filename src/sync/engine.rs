use crate::storage::db::Database;
use crate::storage::index::FileIndex;
use crate::storage::watcher::{FsEvent, FsWatcher};
use crate::sync::clock::HybridClock;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{debug, error, info};

/// Orchestrates local file watching → indexing pipeline.
/// The network layer handles the actual P2P sync separately.
pub struct SyncEngine {
    db: Arc<Mutex<Database>>,
    file_index: Arc<FileIndex>,
    clock: Arc<Mutex<HybridClock>>,
}

impl SyncEngine {
    pub fn new(
        db: Arc<Mutex<Database>>,
        file_index: Arc<FileIndex>,
        clock: Arc<Mutex<HybridClock>>,
    ) -> Self {
        Self {
            db,
            file_index,
            clock,
        }
    }

    /// Run the initial full scan to bring the index up to date.
    pub fn initial_scan(&self) -> Result<u64> {
        let db = self.db.lock().unwrap();
        let mut clock = self.clock.lock().unwrap();
        self.file_index.full_scan(&db, &mut clock)
    }

    /// Watch for local file changes and update the index.
    /// This runs in a blocking loop; call from a dedicated thread.
    pub fn watch_loop(&self) -> Result<()> {
        let watcher = FsWatcher::new(self.file_index.root().to_path_buf())?;
        info!(root = %self.file_index.root().display(), "sync engine watching");

        loop {
            match watcher.recv_timeout(Duration::from_secs(1)) {
                Some(event) => self.process_event(&event),
                None => {
                    // Drain any batched events.
                    for event in watcher.drain() {
                        self.process_event(&event);
                    }
                }
            }
        }
    }

    fn process_event(&self, event: &FsEvent) {
        let db = self.db.lock().unwrap();
        let mut clock = self.clock.lock().unwrap();

        match event {
            FsEvent::Created(path) | FsEvent::Modified(path) => {
                if path.is_file() {
                    match self.file_index.index_file(path, &db, &mut clock) {
                        Ok(Some(fv)) => debug!(path = %fv.path, "indexed change"),
                        Ok(None) => {}
                        Err(e) => error!(path = %path.display(), %e, "index error"),
                    }
                }
            }
            FsEvent::Removed(path) => match self.file_index.mark_deleted(path, &db, &mut clock) {
                Ok(()) => debug!(path = %path.display(), "marked deleted"),
                Err(e) => error!(path = %path.display(), %e, "delete error"),
            },
        }
    }
}
