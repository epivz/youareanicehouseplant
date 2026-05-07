use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::sync::{Arc, Mutex};

use datamesh::cli::commands::{Cli, Command};
use datamesh::crypto::DeviceIdentity;
use datamesh::network::MeshNode;
use datamesh::storage::db::Database;
use datamesh::storage::index::FileIndex;
use datamesh::sync::clock::HybridClock;
use datamesh::sync::engine::SyncEngine;

#[tokio::main]
async fn main() -> Result<()> {
    datamesh::telemetry::init()?;

    let cli = Cli::parse();
    let mesh_root = cli.mesh_root();
    let config_dir = cli.config_dir();

    match cli.command {
        Command::Init => cmd_init(&mesh_root, &config_dir),
        Command::Daemon => cmd_daemon(&mesh_root, &config_dir, cli.port).await,
        Command::Status => cmd_status(&config_dir),
        Command::Files => cmd_files(&mesh_root, &config_dir),
        Command::Trust { peer_id, alias } => cmd_trust(&config_dir, &peer_id, &alias),
        Command::Peers => cmd_peers(&config_dir),
        Command::Conflicts => cmd_conflicts(&config_dir),
        Command::Resolve { conflict_id, keep } => cmd_resolve(&config_dir, conflict_id, &keep),
        Command::Scan => cmd_scan(&mesh_root, &config_dir),
    }
}

fn cmd_init(mesh_root: &std::path::Path, config_dir: &std::path::Path) -> Result<()> {
    fs::create_dir_all(mesh_root).context("creating mesh root")?;
    let identity = DeviceIdentity::load_or_create(config_dir)?;
    let db_path = config_dir.join("index.db");
    let _db = Database::open(&db_path)?;
    println!("Mesh initialised!");
    println!("  Root:    {}", mesh_root.display());
    println!("  Config:  {}", config_dir.display());
    println!("  Peer ID: {}", identity.peer_id());
    Ok(())
}

async fn cmd_daemon(
    mesh_root: &std::path::Path,
    config_dir: &std::path::Path,
    port: u16,
) -> Result<()> {
    let identity = DeviceIdentity::load_or_create(config_dir)?;
    let db_path = config_dir.join("index.db");
    let db = Database::open(&db_path)?;
    let node_id = peer_id_to_u64(&identity.peer_id());
    let clock = HybridClock::now(node_id);
    let file_index = FileIndex::new(mesh_root.to_path_buf(), node_id);

    // Initial scan.
    let mut scan_clock = clock;
    let count = file_index.full_scan(&db, &mut scan_clock)?;
    println!("Initial scan: {count} files indexed");

    let db = Arc::new(Mutex::new(db));
    let file_index = Arc::new(file_index);
    let clock = Arc::new(Mutex::new(scan_clock));

    // Start the file watcher in a background thread.
    let sync_engine = SyncEngine::new(db.clone(), file_index.clone(), clock.clone());
    std::thread::spawn(move || {
        if let Err(e) = sync_engine.watch_loop() {
            eprintln!("sync engine error: {e}");
        }
    });

    // Run the P2P node (blocks).
    let db_for_node = {
        let _guard = db.lock().unwrap();
        Database::open(&config_dir.join("index.db"))?
    };
    let fi_for_node = FileIndex::new(mesh_root.to_path_buf(), node_id);
    let node = MeshNode::new(
        identity,
        db_for_node,
        fi_for_node,
        *clock.lock().unwrap(),
        mesh_root.to_path_buf(),
        port,
    );
    node.run().await
}

fn cmd_status(config_dir: &std::path::Path) -> Result<()> {
    let identity = DeviceIdentity::load_or_create(config_dir)?;
    let db_path = config_dir.join("index.db");
    let db = Database::open(&db_path)?;
    let files = db.list_files()?;
    let conflicts = db.list_conflicts()?;

    println!("DataMesh Status");
    println!("  Peer ID:     {}", identity.peer_id());
    println!("  Config:      {}", config_dir.display());
    println!("  Files:       {}", files.len());
    println!("  Conflicts:   {}", conflicts.len());

    let trusted = identity.trusted_peers()?;
    println!("  Trusted:     {} peers", trusted.len());
    Ok(())
}

fn cmd_files(_mesh_root: &std::path::Path, config_dir: &std::path::Path) -> Result<()> {
    let db_path = config_dir.join("index.db");
    let db = Database::open(&db_path)?;
    let files = db.list_files()?;
    if files.is_empty() {
        println!("No files indexed. Run `datamesh scan` first.");
        return Ok(());
    }
    println!("{:<50} {:>10} HASH", "PATH", "SIZE");
    println!("{}", "-".repeat(80));
    for f in &files {
        println!(
            "{:<50} {:>10} {}",
            f.path,
            human_size(f.size),
            &f.content_hash[..12]
        );
    }
    println!("\n{} files total", files.len());
    Ok(())
}

fn cmd_trust(config_dir: &std::path::Path, peer_id_str: &str, alias: &str) -> Result<()> {
    let identity = DeviceIdentity::load_or_create(config_dir)?;
    let peer_id: libp2p::PeerId = peer_id_str.parse().context("invalid peer ID")?;
    identity.trust_peer(peer_id, alias)?;
    println!("Trusted peer {peer_id} as \"{alias}\"");
    Ok(())
}

fn cmd_peers(config_dir: &std::path::Path) -> Result<()> {
    let identity = DeviceIdentity::load_or_create(config_dir)?;
    let peers = identity.trusted_peers()?;
    if peers.is_empty() {
        println!("No trusted peers. Use `datamesh trust <peer_id>` to add one.");
        return Ok(());
    }
    println!("{:<50} ALIAS", "PEER ID");
    println!("{}", "-".repeat(60));
    for (pid, alias) in &peers {
        println!("{:<50} {}", pid, alias);
    }
    Ok(())
}

fn cmd_conflicts(config_dir: &std::path::Path) -> Result<()> {
    let db_path = config_dir.join("index.db");
    let db = Database::open(&db_path)?;
    let conflicts = db.list_conflicts()?;
    if conflicts.is_empty() {
        println!("No unresolved conflicts.");
        return Ok(());
    }
    println!(
        "{:<6} {:<40} {:<14} {:<14} DATE",
        "ID", "PATH", "LOCAL HASH", "REMOTE HASH"
    );
    println!("{}", "-".repeat(90));
    for c in &conflicts {
        println!(
            "{:<6} {:<40} {:<14} {:<14} {}",
            c.id,
            c.path,
            &c.local_hash[..12.min(c.local_hash.len())],
            &c.remote_hash[..12.min(c.remote_hash.len())],
            c.created_at,
        );
    }
    Ok(())
}

fn cmd_resolve(config_dir: &std::path::Path, conflict_id: i64, _keep: &str) -> Result<()> {
    let db_path = config_dir.join("index.db");
    let db = Database::open(&db_path)?;
    db.resolve_conflict(conflict_id)?;
    println!("Conflict {conflict_id} resolved.");
    Ok(())
}

fn cmd_scan(mesh_root: &std::path::Path, config_dir: &std::path::Path) -> Result<()> {
    let identity = DeviceIdentity::load_or_create(config_dir)?;
    let db_path = config_dir.join("index.db");
    let db = Database::open(&db_path)?;
    let node_id = peer_id_to_u64(&identity.peer_id());
    let file_index = FileIndex::new(mesh_root.to_path_buf(), node_id);
    let mut clock = HybridClock::now(node_id);
    let count = file_index.full_scan(&db, &mut clock)?;
    println!("Scanned {count} files.");
    Ok(())
}

fn peer_id_to_u64(peer_id: &libp2p::PeerId) -> u64 {
    let bytes = peer_id.to_bytes();
    let mut buf = [0u8; 8];
    for (i, b) in bytes.iter().take(8).enumerate() {
        buf[i] = *b;
    }
    u64::from_le_bytes(buf)
}

fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    for unit in UNITS {
        if size < 1024.0 {
            return format!("{size:.1} {unit}");
        }
        size /= 1024.0;
    }
    format!("{size:.1} PB")
}
