use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;
use tracing::{debug, error};

/// Watches the mesh root for file system changes and emits events.
pub struct FsWatcher {
    _watcher: RecommendedWatcher,
    rx: mpsc::Receiver<FsEvent>,
}

#[derive(Debug, Clone)]
pub enum FsEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
}

impl FsWatcher {
    pub fn new(root: PathBuf) -> Result<Self> {
        let (tx, rx) = mpsc::channel();

        let sender = tx.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| match res {
                Ok(event) => {
                    let events = classify_event(&event);
                    for ev in events {
                        if sender.send(ev).is_err() {
                            break;
                        }
                    }
                }
                Err(e) => error!("watch error: {e}"),
            })?;

        watcher.watch(&root, RecursiveMode::Recursive)?;
        debug!(path = %root.display(), "watching for changes");

        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }

    /// Try to receive a pending event (non-blocking).
    pub fn try_recv(&self) -> Option<FsEvent> {
        self.rx.try_recv().ok()
    }

    /// Blocking receive with timeout.
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<FsEvent> {
        self.rx.recv_timeout(timeout).ok()
    }

    /// Drain all pending events.
    pub fn drain(&self) -> Vec<FsEvent> {
        let mut events = Vec::new();
        while let Some(ev) = self.try_recv() {
            events.push(ev);
        }
        events
    }
}

fn classify_event(event: &Event) -> Vec<FsEvent> {
    let mut out = Vec::new();
    for path in &event.paths {
        // Skip hidden files and directories.
        let dominated_by_hidden = path
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
        if dominated_by_hidden {
            continue;
        }

        let ev = match event.kind {
            EventKind::Create(_) => FsEvent::Created(path.clone()),
            EventKind::Modify(_) => FsEvent::Modified(path.clone()),
            EventKind::Remove(_) => FsEvent::Removed(path.clone()),
            _ => continue,
        };
        out.push(ev);
    }
    out
}
