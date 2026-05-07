use anyhow::{Context, Result};
use libp2p::identity::{self, Keypair};
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// Persistent device identity backed by an Ed25519 keypair.
/// Each device in the mesh has a unique identity used for
/// mutual authentication and message signing.
#[derive(Clone)]
pub struct DeviceIdentity {
    keypair: Keypair,
    peer_id: PeerId,
    config_dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct TrustedPeer {
    peer_id: String,
    alias: String,
}

#[derive(Serialize, Deserialize, Default)]
struct TrustStore {
    trusted_peers: Vec<TrustedPeer>,
}

impl DeviceIdentity {
    /// Load an existing identity from disk, or generate a new one.
    pub fn load_or_create(config_dir: &Path) -> Result<Self> {
        fs::create_dir_all(config_dir)
            .with_context(|| format!("creating config dir {}", config_dir.display()))?;

        let key_path = config_dir.join("device_key");

        let keypair = if key_path.exists() {
            let bytes = fs::read(&key_path).context("reading device key")?;
            Keypair::ed25519_from_bytes(bytes).context("parsing device key")?
        } else {
            let kp = identity::Keypair::generate_ed25519();
            // Store raw Ed25519 secret key bytes (first 32 bytes of the keypair encoding)
            if let Ok(ed_kp) = kp.clone().try_into_ed25519() {
                fs::write(&key_path, ed_kp.to_bytes()).context("writing device key")?;
            }
            info!("generated new device identity");
            kp
        };

        let peer_id = PeerId::from(keypair.public());
        info!(%peer_id, "device identity loaded");

        Ok(Self {
            keypair,
            peer_id,
            config_dir: config_dir.to_path_buf(),
        })
    }

    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Add a peer to the trusted device list.
    pub fn trust_peer(&self, peer_id: PeerId, alias: &str) -> Result<()> {
        let mut store = self.load_trust_store()?;
        let pid = peer_id.to_string();
        if store.trusted_peers.iter().any(|p| p.peer_id == pid) {
            info!(%peer_id, "peer already trusted");
            return Ok(());
        }
        store.trusted_peers.push(TrustedPeer {
            peer_id: pid,
            alias: alias.to_string(),
        });
        self.save_trust_store(&store)?;
        info!(%peer_id, alias, "peer trusted");
        Ok(())
    }

    /// Check whether a peer is in the trusted device list.
    pub fn is_trusted(&self, peer_id: &PeerId) -> Result<bool> {
        let store = self.load_trust_store()?;
        Ok(store
            .trusted_peers
            .iter()
            .any(|p| p.peer_id == peer_id.to_string()))
    }

    /// Return all trusted peers.
    pub fn trusted_peers(&self) -> Result<Vec<(PeerId, String)>> {
        let store = self.load_trust_store()?;
        let mut out = Vec::new();
        for tp in store.trusted_peers {
            if let Ok(pid) = tp.peer_id.parse() {
                out.push((pid, tp.alias));
            }
        }
        Ok(out)
    }

    fn trust_store_path(&self) -> PathBuf {
        self.config_dir.join("trusted_peers.json")
    }

    fn load_trust_store(&self) -> Result<TrustStore> {
        let path = self.trust_store_path();
        if path.exists() {
            let data = fs::read_to_string(&path).context("reading trust store")?;
            Ok(serde_json::from_str(&data).context("parsing trust store")?)
        } else {
            Ok(TrustStore::default())
        }
    }

    fn save_trust_store(&self, store: &TrustStore) -> Result<()> {
        let data = serde_json::to_string_pretty(store)?;
        fs::write(self.trust_store_path(), data).context("writing trust store")?;
        Ok(())
    }
}
