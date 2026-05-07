use crate::crypto::DeviceIdentity;
use crate::network::behaviour::{build_swarm, MeshBehaviour, MeshBehaviourEvent};
use crate::network::protocol::{FileRequest, FileResponse};
use crate::storage::db::Database;
use crate::storage::index::FileIndex;
use crate::sync::clock::HybridClock;
use crate::sync::crdt::{merge_versions, MergeOutcome};

use anyhow::Result;
use futures::StreamExt;
use libp2p::request_response::{self, ResponseChannel};
use libp2p::swarm::SwarmEvent;
use libp2p::{mdns, Multiaddr, PeerId, Swarm};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};

/// High-level mesh node: runs the swarm event loop, handles sync requests.
pub struct MeshNode {
    identity: DeviceIdentity,
    db: Arc<Mutex<Database>>,
    _file_index: Arc<FileIndex>,
    _clock: Arc<Mutex<HybridClock>>,
    mesh_root: PathBuf,
    listen_port: u16,
}

impl MeshNode {
    pub fn new(
        identity: DeviceIdentity,
        db: Database,
        file_index: FileIndex,
        clock: HybridClock,
        mesh_root: PathBuf,
        listen_port: u16,
    ) -> Self {
        Self {
            identity,
            db: Arc::new(Mutex::new(db)),
            _file_index: Arc::new(file_index),
            _clock: Arc::new(Mutex::new(clock)),
            mesh_root,
            listen_port,
        }
    }

    /// Run the mesh node event loop. This blocks until shutdown.
    pub async fn run(&self) -> Result<()> {
        let mut swarm = build_swarm(self.identity.keypair(), self.listen_port)?;

        let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", self.listen_port).parse()?;
        swarm.listen_on(listen_addr)?;

        info!(peer_id = %self.identity.peer_id(), "mesh node starting");

        let mut discovered_peers: HashSet<PeerId> = HashSet::new();

        loop {
            match swarm.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. } => {
                    info!(%address, "listening");
                }

                SwarmEvent::Behaviour(MeshBehaviourEvent::Mdns(mdns::Event::Discovered(peers))) => {
                    for (peer_id, addr) in peers {
                        if discovered_peers.insert(peer_id) {
                            info!(%peer_id, %addr, "discovered peer");
                            swarm.dial(addr)?;
                        }
                    }
                }

                SwarmEvent::Behaviour(MeshBehaviourEvent::Mdns(mdns::Event::Expired(peers))) => {
                    for (peer_id, _) in peers {
                        discovered_peers.remove(&peer_id);
                        debug!(%peer_id, "peer expired from mDNS");
                    }
                }

                SwarmEvent::Behaviour(MeshBehaviourEvent::RequestResponse(
                    request_response::Event::Message { peer, message },
                )) => match message {
                    request_response::Message::Request {
                        request, channel, ..
                    } => {
                        self.handle_request(&mut swarm, peer, request, channel);
                    }
                    request_response::Message::Response { response, .. } => {
                        self.handle_response(peer, response);
                    }
                },

                SwarmEvent::Behaviour(MeshBehaviourEvent::RequestResponse(
                    request_response::Event::OutboundFailure { peer, error, .. },
                )) => {
                    warn!(%peer, %error, "outbound request failed");
                }

                SwarmEvent::Behaviour(MeshBehaviourEvent::RequestResponse(
                    request_response::Event::InboundFailure { peer, error, .. },
                )) => {
                    warn!(%peer, %error, "inbound request failed");
                }

                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    info!(%peer_id, "connection established");
                    // Request index from new peer.
                    swarm
                        .behaviour_mut()
                        .request_response
                        .send_request(&peer_id, FileRequest::ListIndex);
                }

                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    debug!(%peer_id, "connection closed");
                }

                _ => {}
            }
        }
    }

    fn handle_request(
        &self,
        swarm: &mut Swarm<MeshBehaviour>,
        peer: PeerId,
        request: FileRequest,
        channel: ResponseChannel<FileResponse>,
    ) {
        debug!(%peer, "handling request");
        let response = match request {
            FileRequest::ListIndex => {
                let db = self.db.lock().unwrap();
                match db.list_all_versions() {
                    Ok(versions) => FileResponse::Index { versions },
                    Err(e) => FileResponse::Error {
                        message: e.to_string(),
                    },
                }
            }
            FileRequest::GetFile { path } => {
                let abs_path = self.mesh_root.join(&path);
                match fs::read(&abs_path) {
                    Ok(data) => {
                        let db = self.db.lock().unwrap();
                        match db.get_file(&path) {
                            Ok(Some(version)) => FileResponse::FileContent {
                                path,
                                data,
                                version,
                            },
                            Ok(None) => FileResponse::Error {
                                message: format!("file not in index: {path}"),
                            },
                            Err(e) => FileResponse::Error {
                                message: e.to_string(),
                            },
                        }
                    }
                    Err(e) => FileResponse::Error {
                        message: format!("reading {path}: {e}"),
                    },
                }
            }
            FileRequest::PushIndex { versions } => {
                let (accepted, conflicts) = self.apply_remote_versions(&versions);
                FileResponse::Ack {
                    accepted,
                    conflicts,
                }
            }
        };

        if let Err(resp) = swarm
            .behaviour_mut()
            .request_response
            .send_response(channel, response)
        {
            warn!("failed to send response: {resp:?}");
        }
    }

    fn handle_response(&self, peer: PeerId, response: FileResponse) {
        match response {
            FileResponse::Index { versions } => {
                info!(%peer, count = versions.len(), "received remote index");
                let (accepted, conflicts) = self.apply_remote_versions(&versions);
                info!(accepted, conflicts, "index merge complete");
            }
            FileResponse::FileContent {
                path,
                data,
                version,
                ..
            } => {
                let abs_path = self.mesh_root.join(&path);
                if let Some(parent) = abs_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                match fs::write(&abs_path, &data) {
                    Ok(()) => {
                        let db = self.db.lock().unwrap();
                        let _ = db.upsert_file(&version);
                        info!(path = %path, size = data.len(), "file synced from peer");
                    }
                    Err(e) => error!(path = %path, %e, "failed to write synced file"),
                }
            }
            FileResponse::Ack {
                accepted,
                conflicts,
            } => {
                debug!(accepted, conflicts, "push acknowledged");
            }
            FileResponse::Error { message } => {
                warn!(%peer, message, "peer returned error");
            }
        }
    }

    fn apply_remote_versions(
        &self,
        remote_versions: &[crate::sync::crdt::FileVersion],
    ) -> (usize, usize) {
        let db = self.db.lock().unwrap();
        let mut accepted = 0usize;
        let mut conflicts = 0usize;

        for remote in remote_versions {
            let local = match db.get_file(&remote.path) {
                Ok(Some(l)) => l,
                Ok(None) => {
                    // New file from remote — accept it.
                    if db.upsert_file(remote).is_ok() {
                        accepted += 1;
                    }
                    continue;
                }
                Err(e) => {
                    error!(path = %remote.path, %e, "db error during merge");
                    continue;
                }
            };

            match merge_versions(&local, remote) {
                MergeOutcome::KeepLocal => {}
                MergeOutcome::AcceptRemote(fv) => {
                    if db.upsert_file(&fv).is_ok() {
                        accepted += 1;
                    }
                }
                MergeOutcome::Conflict {
                    local: l,
                    remote: r,
                } => {
                    let _ = db.record_conflict(&remote.path, &l, &r);
                    conflicts += 1;
                }
            }
        }

        (accepted, conflicts)
    }
}
