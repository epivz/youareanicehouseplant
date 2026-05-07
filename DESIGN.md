# DataMesh — System Design Document

## 1. Overview

DataMesh is a decentralized peer-to-peer file synchronization system that treats all personal devices as a single unified "mesh." Files saved on any device are instantly searchable and streamable from any other device in the mesh — without relying on corporate cloud servers.

## 2. Design Goals

| Goal | Approach |
|------|----------|
| **No central server** | Pure P2P via libp2p; mDNS for LAN discovery |
| **Distributed consistency** | Version Vectors + Hybrid Logical Clocks |
| **Zero trust** | Ed25519 identities, Noise-encrypted transport, explicit peer trust |
| **Performance** | Rust for memory safety/speed, BLAKE3 for hashing, SQLite for indexing |
| **Observability** | OpenTelemetry from day one |

## 3. Architecture

### 3.1 Layers

```
┌──────────────────────────────────────────┐
│                   CLI                     │
├──────────────────────────────────────────┤
│              Sync Engine                  │
│   (watch → index → detect → propagate)   │
├────────────────┬─────────────────────────┤
│    Network     │       Storage           │
│  (libp2p P2P)  │  (SQLite + FS watcher)  │
├────────────────┼─────────────────────────┤
│    Crypto      │       CRDT              │
│  (identities)  │  (version vectors, HLC) │
└────────────────┴─────────────────────────┘
```

### 3.2 Network Layer

**Transport:** TCP with Noise encryption and Yamux multiplexing (via libp2p).

**Discovery:** mDNS for automatic LAN peer discovery. Peers on the same network segment find each other within seconds.

**Protocol:** Custom request-response protocol (`/datamesh/sync/1.0.0`) with CBOR serialization:

| Request | Response | Description |
|---------|----------|-------------|
| `ListIndex` | `Index { versions }` | Exchange full file metadata index |
| `GetFile { path }` | `FileContent { path, data, version }` | Transfer a single file |
| `PushIndex { versions }` | `Ack { accepted, conflicts }` | Push metadata updates |

### 3.3 Conflict Resolution

**Problem:** Two devices modify the same file while offline, then reconnect.

**Solution:** Two-tier approach:

1. **Version Vectors** detect concurrency:
   - Device A modifies file → VV becomes `{A: 2, B: 1}`
   - Device B modifies same file → VV becomes `{A: 1, B: 2}`
   - Neither dominates → **concurrent** edit detected

2. **HLC provides automatic resolution:**
   - Compare Hybrid Logical Clock timestamps
   - Last-Writer-Wins (LWW) for automatic conflict resolution
   - Truly simultaneous edits (identical HLC) → manual conflict, persisted in DB

This gives the best of both worlds: strong consistency detection with pragmatic automatic resolution.

### 3.4 Storage

**Local Index:** SQLite database (`index.db`) stores file metadata:
- Path, content hash (BLAKE3), size, permissions
- HLC timestamp, version vector
- Deleted flag (tombstone)

**Content Hashing:** BLAKE3 for fast cryptographic file hashing. Enables content-addressable deduplication.

**Filesystem Watcher:** `notify` crate monitors the mesh root for real-time change detection.

### 3.5 Security Model

```
Device A                        Device B
┌─────────┐                    ┌─────────┐
│ Ed25519  │  Noise handshake  │ Ed25519  │
│ Keypair  │◄────────────────►│ Keypair  │
└─────────┘                    └─────────┘
     │                              │
     ▼                              ▼
 Trust Store                   Trust Store
 (allowlist)                   (allowlist)
```

- Each device generates a persistent Ed25519 keypair
- Peers must be explicitly trusted via `datamesh trust <peer_id>`
- libp2p Noise protocol provides authenticated encryption for all traffic
- No PKI or CA required — trust is managed locally

## 4. Data Model

### Files Table
```sql
CREATE TABLE files (
    path            TEXT PRIMARY KEY,
    content_hash    TEXT NOT NULL,        -- BLAKE3 hex
    size            INTEGER NOT NULL,
    hlc_wall_ms     INTEGER NOT NULL,     -- HLC wall clock
    hlc_counter     INTEGER NOT NULL,     -- HLC logical counter
    hlc_node_id     INTEGER NOT NULL,     -- originating node
    version_vector  TEXT NOT NULL,         -- JSON {node_id: counter}
    deleted         INTEGER DEFAULT 0,    -- tombstone flag
    mode            INTEGER,              -- Unix permissions
    updated_at      TEXT DEFAULT NOW
);
```

### Conflicts Table
```sql
CREATE TABLE conflicts (
    id              INTEGER PRIMARY KEY,
    path            TEXT NOT NULL,
    local_hash      TEXT NOT NULL,
    remote_hash     TEXT NOT NULL,
    local_version   TEXT NOT NULL,         -- JSON version vector
    remote_version  TEXT NOT NULL,
    resolved        INTEGER DEFAULT 0,
    created_at      TEXT DEFAULT NOW
);
```

## 5. Sync Protocol Flow

```
Device A                              Device B
   │                                      │
   │──── mDNS Discovery ─────────────────►│
   │                                      │
   │◄─── Connection Established ──────────│
   │                                      │
   │──── ListIndex ──────────────────────►│
   │◄─── Index { versions } ─────────────│
   │                                      │
   │  [compare version vectors locally]   │
   │                                      │
   │──── GetFile { path: "doc.md" } ────►│
   │◄─── FileContent { data, version } ──│
   │                                      │
   │  [write file, update local index]    │
   │                                      │
   │──── PushIndex { updated versions } ─►│
   │◄─── Ack { accepted: 3, conflicts: 0}│
   │                                      │
```

## 6. Future Work

- **DHT-based discovery** for cross-network sync (Kademlia)
- **Chunked file transfer** for large files (streaming)
- **Selective sync** — choose which folders to sync per device
- **Mobile support** — iOS/Android companion apps
- **WebRTC transport** for NAT traversal
- **Compression** (zstd) for transfer efficiency
- **Benchmarking suite** with latency/throughput metrics
