use crate::sync::clock::HybridClock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Version vector: maps `node_id → counter`.
/// Used to detect concurrent modifications across devices.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct VersionVector(pub HashMap<u64, u64>);

impl VersionVector {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn increment(&mut self, node_id: u64) {
        let entry = self.0.entry(node_id).or_insert(0);
        *entry += 1;
    }

    pub fn get(&self, node_id: u64) -> u64 {
        self.0.get(&node_id).copied().unwrap_or(0)
    }

    /// Returns `true` if `self` dominates (is ≥ for all keys and > for at least one).
    pub fn dominates(&self, other: &VersionVector) -> bool {
        let dominated = other.0.iter().all(|(&k, &v)| self.get(k) >= v);
        let strictly_greater = self.0.iter().any(|(&k, &v)| v > other.get(k));
        dominated && strictly_greater
    }

    /// Returns `true` if the two vectors are concurrent (neither dominates).
    pub fn is_concurrent(&self, other: &VersionVector) -> bool {
        !self.dominates(other) && !other.dominates(self) && self != other
    }

    /// Pointwise merge (max of each component).
    pub fn merge(&mut self, other: &VersionVector) {
        for (&k, &v) in &other.0 {
            let entry = self.0.entry(k).or_insert(0);
            *entry = (*entry).max(v);
        }
    }
}

/// Metadata for a single file version in the mesh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileVersion {
    /// Relative path within the shared mesh root.
    pub path: String,
    /// BLAKE3 content hash.
    pub content_hash: String,
    /// File size in bytes.
    pub size: u64,
    /// HLC timestamp of the last modification.
    pub hlc: HybridClock,
    /// Version vector at the time of this write.
    pub version: VersionVector,
    /// `true` if the file was deleted (tombstone).
    pub deleted: bool,
    /// Unix permissions mode (optional).
    pub mode: Option<u32>,
}

/// Result of merging two file versions.
#[derive(Debug)]
pub enum MergeOutcome {
    /// Local version is up-to-date; nothing to do.
    KeepLocal,
    /// Remote version supersedes local; accept it.
    AcceptRemote(FileVersion),
    /// Concurrent edits detected; both versions are retained.
    Conflict {
        local: FileVersion,
        remote: FileVersion,
    },
}

/// Merge two file versions using version vectors, with HLC as tiebreaker.
pub fn merge_versions(local: &FileVersion, remote: &FileVersion) -> MergeOutcome {
    if local.version.dominates(&remote.version) {
        MergeOutcome::KeepLocal
    } else if remote.version.dominates(&local.version) {
        MergeOutcome::AcceptRemote(remote.clone())
    } else if local.version == remote.version {
        // Identical version vectors → same state.
        MergeOutcome::KeepLocal
    } else {
        // Concurrent modifications — use LWW (HLC) as automatic resolution.
        if remote.hlc > local.hlc {
            MergeOutcome::AcceptRemote(remote.clone())
        } else if local.hlc > remote.hlc {
            MergeOutcome::KeepLocal
        } else {
            // Truly simultaneous — flag conflict for user resolution.
            MergeOutcome::Conflict {
                local: local.clone(),
                remote: remote.clone(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_version(node_id: u64, counter: u64, wall_ms: u64) -> FileVersion {
        let mut vv = VersionVector::new();
        vv.0.insert(node_id, counter);
        FileVersion {
            path: "test.txt".into(),
            content_hash: "abc".into(),
            size: 100,
            hlc: HybridClock {
                wall_ms,
                counter: 0,
                node_id,
            },
            version: vv,
            deleted: false,
            mode: None,
        }
    }

    #[test]
    fn remote_dominates() {
        let local = make_version(1, 1, 1000);
        let mut remote = make_version(1, 2, 2000);
        remote.version.0.insert(1, 2);
        assert!(matches!(
            merge_versions(&local, &remote),
            MergeOutcome::AcceptRemote(_)
        ));
    }

    #[test]
    fn concurrent_resolves_lww() {
        let local = make_version(1, 1, 1000);
        let remote = make_version(2, 1, 2000);
        assert!(matches!(
            merge_versions(&local, &remote),
            MergeOutcome::AcceptRemote(_)
        ));
    }

    #[test]
    fn version_vector_dominates() {
        let mut a = VersionVector::new();
        a.0.insert(1, 3);
        a.0.insert(2, 2);
        let mut b = VersionVector::new();
        b.0.insert(1, 2);
        b.0.insert(2, 2);
        assert!(a.dominates(&b));
        assert!(!b.dominates(&a));
    }
}
