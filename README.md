# DataMesh

A decentralized peer-to-peer file mesh for personal devices. Sync files across your laptop, phone, and desktop without ever hitting a corporate server.

## Architecture

```
┌─────────────┐     libp2p (Noise-encrypted)     ┌─────────────┐
│   Device A  │◄──────────────────────────────────►│   Device B  │
│             │         mDNS / TCP                │             │
│  ┌────────┐ │                                   │ ┌────────┐  │
│  │ SQLite │ │     request-response protocol     │ │ SQLite │  │
│  │ Index  │ │◄──────────────────────────────────►│ │ Index  │  │
│  └────────┘ │                                   │ └────────┘  │
│  ┌────────┐ │                                   │ ┌────────┐  │
│  │  File  │ │                                   │ │  File  │  │
│  │ Watcher│ │                                   │ │ Watcher│  │
│  └────────┘ │                                   │ └────────┘  │
└─────────────┘                                   └─────────────┘
```

### Key Components

| Module | Purpose |
|--------|---------|
| `crypto` | Device identity (Ed25519 keypairs), trusted peer management |
| `network` | libp2p swarm, mDNS discovery, Noise-encrypted transport, request-response file protocol |
| `storage` | SQLite file index, BLAKE3 content hashing, filesystem watcher |
| `sync` | Hybrid Logical Clocks, Version Vectors, CRDT-based conflict resolution |
| `cli` | Full CLI for mesh operations |
| `telemetry` | OpenTelemetry + tracing instrumentation |

## How It Works

### Distributed Consistency

DataMesh uses **Version Vectors** combined with **Hybrid Logical Clocks (HLC)** to solve the distributed consistency problem:

1. **Version Vectors** — Each file carries a version vector `{device_id → counter}`. When a file is modified locally, the device increments its own counter. During sync, version vectors are compared:
   - If one dominates → accept the newer version
   - If concurrent → conflict detected

2. **HLC Timestamps** — When concurrent edits are detected, Last-Writer-Wins (LWW) semantics using HLC timestamps provide automatic resolution. Truly simultaneous edits (identical HLC) are flagged as conflicts for manual resolution.

3. **Tombstones** — Deleted files are tracked as tombstones so deletions propagate correctly across the mesh.

### Zero Trust Architecture

- Every device generates a unique **Ed25519 keypair** on first initialization
- All P2P communication uses **libp2p Noise protocol** — every packet is encrypted and mutually authenticated
- Devices must be explicitly **trusted** before they can sync (allowlist model)
- No central server or certificate authority required

### Sync Protocol

1. **Discovery** — Devices find each other via mDNS on the local network
2. **Index Exchange** — On connection, peers exchange their full file index (metadata + version vectors)
3. **Diff & Merge** — Indices are compared using CRDT merge rules
4. **File Transfer** — Changed files are requested and transferred peer-to-peer
5. **Continuous Watch** — A filesystem watcher detects local changes in real-time

## Tech Stack

- **Rust** — Memory safety, zero-cost abstractions, fearless concurrency
- **libp2p** — Battle-tested P2P networking (Noise encryption, Yamux multiplexing, mDNS discovery)
- **SQLite** — Embedded local file index via `rusqlite`
- **BLAKE3** — Fast cryptographic content hashing
- **tokio** — Async runtime
- **tracing + OpenTelemetry** — Structured logging and distributed tracing

## Quick Start

```bash
# Build
cargo build --release

# Initialize a mesh directory
datamesh init --root ~/my-mesh

# Start the daemon (P2P sync + file watcher)
datamesh daemon --root ~/my-mesh

# On another device, init and start with the same mesh root
datamesh init --root ~/my-mesh
datamesh daemon --root ~/my-mesh

# Trust a peer (exchange peer IDs out-of-band)
datamesh trust <PEER_ID> --alias "laptop"

# Check status
datamesh status

# List indexed files
datamesh files

# Run a manual scan
datamesh scan

# View unresolved conflicts
datamesh conflicts

# Resolve a conflict
datamesh resolve <CONFLICT_ID> local
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DATAMESH_ROOT` | `~/datamesh` | Path to the mesh root directory |
| `DATAMESH_CONFIG` | `<root>/.datamesh` | Path to config/data directory |
| `DATAMESH_PORT` | `4001` | P2P listener port |
| `RUST_LOG` | `datamesh=info,warn` | Tracing verbosity |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | — | OTLP collector endpoint for OpenTelemetry |

## Project Structure

```
src/
├── main.rs              # CLI entry point, command dispatch
├── lib.rs               # Library root
├── cli/
│   ├── mod.rs
│   └── commands.rs      # Clap-based CLI definitions
├── crypto/
│   ├── mod.rs
│   └── identity.rs      # Ed25519 device identity + trust store
├── network/
│   ├── mod.rs
│   ├── behaviour.rs     # libp2p NetworkBehaviour (mDNS + request-response)
│   ├── node.rs          # Mesh node event loop, sync handling
│   └── protocol.rs      # File transfer protocol messages
├── storage/
│   ├── mod.rs
│   ├── db.rs            # SQLite schema, queries (files, conflicts, sync state)
│   ├── index.rs         # File indexing, BLAKE3 hashing, full scan
│   └── watcher.rs       # Filesystem change notifications
├── sync/
│   ├── mod.rs
│   ├── clock.rs         # Hybrid Logical Clock (HLC)
│   ├── crdt.rs          # Version Vectors, merge logic, conflict detection
│   └── engine.rs        # Watch→index pipeline orchestration
└── telemetry/
    └── mod.rs           # tracing + OpenTelemetry init
```

## Development

```bash
# Run tests
cargo test

# Lint
cargo clippy

# Format
cargo fmt

# Build release binary
cargo build --release
```

## License

MIT
