use libp2p::identity::Keypair;
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::swarm::NetworkBehaviour;
use libp2p::{mdns, noise, tcp, yamux, StreamProtocol, Swarm, SwarmBuilder};
use std::time::Duration;
use tracing::info;

use super::protocol::{FileRequest, FileResponse, PROTOCOL_NAME};

/// Combined network behaviour for the mesh node.
#[derive(NetworkBehaviour)]
pub struct MeshBehaviour {
    pub mdns: mdns::tokio::Behaviour,
    pub request_response: request_response::cbor::Behaviour<FileRequest, FileResponse>,
}

/// Build a libp2p Swarm with our mesh behaviour.
pub fn build_swarm(keypair: &Keypair, listen_port: u16) -> anyhow::Result<Swarm<MeshBehaviour>> {
    let swarm = SwarmBuilder::with_existing_identity(keypair.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|key| {
            let mdns =
                mdns::tokio::Behaviour::new(mdns::Config::default(), key.public().to_peer_id())?;

            let rr = request_response::cbor::Behaviour::new(
                [(StreamProtocol::new(PROTOCOL_NAME), ProtocolSupport::Full)],
                request_response::Config::default().with_request_timeout(Duration::from_secs(60)),
            );

            Ok(MeshBehaviour {
                mdns,
                request_response: rr,
            })
        })?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(300)))
        .build();

    info!(
        peer_id = %swarm.local_peer_id(),
        port = listen_port,
        "swarm built"
    );

    Ok(swarm)
}
