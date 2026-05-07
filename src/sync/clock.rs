use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Hybrid Logical Clock (HLC) for ordering events across devices.
///
/// Combines a physical wall-clock component with a logical counter
/// so that causally related events are always ordered correctly,
/// even when device clocks drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridClock {
    /// Wall-clock time in milliseconds since UNIX epoch.
    pub wall_ms: u64,
    /// Logical counter — breaks ties when wall clocks are equal.
    pub counter: u32,
    /// Originating node id (compact u64 hash of PeerId).
    pub node_id: u64,
}

impl HybridClock {
    pub fn now(node_id: u64) -> Self {
        Self {
            wall_ms: Self::physical_ms(),
            counter: 0,
            node_id,
        }
    }

    /// Advance the clock on a local event, returning the new timestamp.
    pub fn tick(&mut self) -> Self {
        let phys = Self::physical_ms();
        if phys > self.wall_ms {
            self.wall_ms = phys;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
        *self
    }

    /// Merge with a remote timestamp (on receive), returning the new local time.
    pub fn merge(&mut self, remote: &HybridClock) -> Self {
        let phys = Self::physical_ms();
        let max_wall = phys.max(self.wall_ms).max(remote.wall_ms);

        if max_wall == self.wall_ms && max_wall == remote.wall_ms {
            self.counter = self.counter.max(remote.counter) + 1;
        } else if max_wall == self.wall_ms {
            self.counter += 1;
        } else if max_wall == remote.wall_ms {
            self.counter = remote.counter + 1;
        } else {
            // physical clock is ahead of both
            self.counter = 0;
        }
        self.wall_ms = max_wall;
        *self
    }

    fn physical_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

impl PartialOrd for HybridClock {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HybridClock {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.wall_ms
            .cmp(&other.wall_ms)
            .then(self.counter.cmp(&other.counter))
            .then(self.node_id.cmp(&other.node_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_increments() {
        let mut c = HybridClock::now(1);
        let t1 = c.tick();
        let t2 = c.tick();
        assert!(t2 > t1);
    }

    #[test]
    fn merge_picks_max() {
        let mut local = HybridClock::now(1);
        let remote = HybridClock {
            wall_ms: local.wall_ms + 1000,
            counter: 5,
            node_id: 2,
        };
        let merged = local.merge(&remote);
        assert!(merged.wall_ms >= remote.wall_ms);
    }
}
