use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "datamesh",
    version,
    about = "Decentralized P2P file mesh for personal devices"
)]
pub struct Cli {
    /// Path to the mesh root directory (files to sync).
    #[arg(short, long, env = "DATAMESH_ROOT")]
    pub root: Option<PathBuf>,

    /// Path to the config/data directory.
    #[arg(short, long, env = "DATAMESH_CONFIG")]
    pub config: Option<PathBuf>,

    /// Port for the P2P listener.
    #[arg(short, long, default_value = "4001", env = "DATAMESH_PORT")]
    pub port: u16,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Initialise a new mesh root directory.
    Init,

    /// Start the mesh daemon (P2P sync + file watcher).
    Daemon,

    /// Show this device's peer ID and status.
    Status,

    /// List all indexed files.
    Files,

    /// Trust a remote peer by their peer ID.
    Trust {
        /// The peer ID to trust.
        peer_id: String,
        /// Friendly alias for the peer.
        #[arg(short, long, default_value = "peer")]
        alias: String,
    },

    /// List trusted peers.
    Peers,

    /// Show unresolved file conflicts.
    Conflicts,

    /// Resolve a conflict by picking local or remote version.
    Resolve {
        /// Conflict ID.
        conflict_id: i64,
        /// Which version to keep: "local" or "remote".
        #[arg(value_parser = ["local", "remote"])]
        keep: String,
    },

    /// Run a one-time full scan of the mesh root.
    Scan,
}

impl Cli {
    pub fn mesh_root(&self) -> PathBuf {
        self.root
            .clone()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join("datamesh"))
    }

    pub fn config_dir(&self) -> PathBuf {
        self.config
            .clone()
            .unwrap_or_else(|| self.mesh_root().join(".datamesh"))
    }
}
