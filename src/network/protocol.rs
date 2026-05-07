use crate::sync::crdt::FileVersion;
use serde::{Deserialize, Serialize};

/// Request types for the mesh file-transfer protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileRequest {
    /// Request the full file index from a peer.
    ListIndex,
    /// Request the content of a specific file by its relative path.
    GetFile { path: String },
    /// Push a batch of updated file versions (metadata only).
    PushIndex { versions: Vec<FileVersion> },
}

/// Response types for the mesh file-transfer protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileResponse {
    /// The peer's complete file index.
    Index { versions: Vec<FileVersion> },
    /// File content (raw bytes, base64-encoded for serialization).
    FileContent {
        path: String,
        data: Vec<u8>,
        version: FileVersion,
    },
    /// Acknowledgement for PushIndex.
    Ack { accepted: usize, conflicts: usize },
    /// Error response.
    Error { message: String },
}

/// Protocol name used for libp2p request-response.
pub const PROTOCOL_NAME: &str = "/datamesh/sync/1.0.0";

/// Codec marker type for the mesh protocol.
#[derive(Debug, Clone)]
pub struct MeshProtocol;
