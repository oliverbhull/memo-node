use crate::storage::{Peer, Storage, Transcription};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Duration};
use tonic::{transport::Server, Request, Response, Status};
use tracing::{debug, info, warn};

// Generated proto code
pub mod proto {
    tonic::include_proto!("memo");
}

use proto::{
    memo_sync_server::{MemoSync, MemoSyncServer as TonicMemoSyncServer},
    PingRequest, PingResponse, PushResponse, SinceRequest, Transcription as ProtoTranscription,
};

#[derive(Clone)]
pub struct PeerSyncServer {
    node_id: String,
    storage: Storage,
    broadcast_tx: mpsc::UnboundedSender<Transcription>,
}

impl PeerSyncServer {
    pub fn new(
        node_id: String,
        storage: Storage,
        broadcast_tx: mpsc::UnboundedSender<Transcription>,
    ) -> Self {
        Self {
            node_id,
            storage,
            broadcast_tx,
        }
    }

    pub async fn serve(self, port: u16) -> Result<()> {
        let addr = format!("0.0.0.0:{}", port).parse()?;
        info!("Starting gRPC server on {}", addr);

        Server::builder()
            .add_service(TonicMemoSyncServer::new(self))
            .serve(addr)
            .await
            .context("gRPC server failed")?;

        Ok(())
    }
}

#[tonic::async_trait]
impl MemoSync for PeerSyncServer {
    async fn ping(&self, request: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
        let req = request.into_inner();
        debug!("Received ping from {}", req.node_id);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        Ok(Response::new(PingResponse {
            node_id: self.node_id.clone(),
            timestamp,
        }))
    }

    type GetTranscriptionsSinceStream =
        tokio_stream::wrappers::ReceiverStream<Result<ProtoTranscription, Status>>;

    async fn get_transcriptions_since(
        &self,
        request: Request<SinceRequest>,
    ) -> Result<Response<Self::GetTranscriptionsSinceStream>, Status> {
        let req = request.into_inner();
        debug!("Getting transcriptions since {}", req.since_timestamp);

        let transcriptions = self
            .storage
            .get_transcriptions_since(req.since_timestamp)
            .map_err(|e| Status::internal(format!("Storage error: {}", e)))?;

        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            for t in transcriptions {
                let proto_t = ProtoTranscription {
                    id: t.id,
                    timestamp: t.timestamp,
                    text: t.text,
                    source_node: t.source_node,
                    memo_device_id: t.memo_device_id.unwrap_or_default(),
                };

                if tx.send(Ok(proto_t)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    async fn push_transcriptions(
        &self,
        request: Request<tonic::Streaming<ProtoTranscription>>,
    ) -> Result<Response<PushResponse>, Status> {
        let mut stream = request.into_inner();
        let mut received = 0;

        while let Some(proto_t) = stream
            .message()
            .await
            .map_err(|e| Status::internal(format!("Stream error: {}", e)))?
        {
            let transcription = Transcription {
                id: proto_t.id,
                timestamp: proto_t.timestamp,
                text: proto_t.text,
                source_node: proto_t.source_node,
                memo_device_id: if proto_t.memo_device_id.is_empty() {
                    None
                } else {
                    Some(proto_t.memo_device_id)
                },
                synced: true, // Mark as synced since it came from a peer
            };

            self.storage
                .insert_transcription(&transcription)
                .map_err(|e| Status::internal(format!("Storage error: {}", e)))?;

            // Broadcast to connected clients (memo-desktop)
            let _ = self.broadcast_tx.send(transcription);

            received += 1;
        }

        debug!("Received {} transcriptions", received);

        Ok(Response::new(PushResponse { received }))
    }
}

pub struct PeerManager {
    node_id: String,
    storage: Storage,
    peers: Arc<RwLock<HashMap<String, PeerConnection>>>,
    sync_interval: Duration,
}

struct PeerConnection {
    node_id: String,
    address: IpAddr,
    grpc_port: u16,
}

impl PeerManager {
    pub fn new(node_id: String, storage: Storage, sync_interval_secs: u64) -> Self {
        Self {
            node_id,
            storage,
            peers: Arc::new(RwLock::new(HashMap::new())),
            sync_interval: Duration::from_secs(sync_interval_secs),
        }
    }

    pub async fn add_peer(&self, node_id: String, address: IpAddr, grpc_port: u16) {
        let mut peers = self.peers.write().await;
        peers.insert(
            node_id.clone(),
            PeerConnection {
                node_id,
                address,
                grpc_port,
            },
        );
    }

    pub async fn start_sync_loop(self: Arc<Self>) {
        let mut ticker = interval(self.sync_interval);

        loop {
            ticker.tick().await;
            self.sync_with_peers().await;
        }
    }

    async fn sync_with_peers(&self) {
        let peers = self.peers.read().await;

        for peer_conn in peers.values() {
            if let Err(e) = self.sync_with_peer(peer_conn).await {
                warn!(
                    "Failed to sync with peer {}: {}",
                    peer_conn.node_id, e
                );
            }
        }
    }

    async fn sync_with_peer(&self, peer_conn: &PeerConnection) -> Result<()> {
        let addr = format!("http://{}:{}", peer_conn.address, peer_conn.grpc_port);

        let mut client = proto::memo_sync_client::MemoSyncClient::connect(addr)
            .await
            .context("Failed to connect to peer")?;

        // Get the last sync timestamp for this peer
        let last_sync = self
            .storage
            .get_peer(&peer_conn.node_id)?
            .map(|p| p.last_sync_timestamp)
            .unwrap_or(0);

        // Fetch transcriptions since last sync
        let request = tonic::Request::new(SinceRequest {
            since_timestamp: last_sync,
        });

        let mut stream = client
            .get_transcriptions_since(request)
            .await
            .context("Failed to get transcriptions")?
            .into_inner();

        let mut count = 0;
        let mut latest_timestamp = last_sync;

        while let Some(proto_t) = stream.message().await? {
            let transcription = Transcription {
                id: proto_t.id,
                timestamp: proto_t.timestamp,
                text: proto_t.text.clone(),
                source_node: proto_t.source_node,
                memo_device_id: if proto_t.memo_device_id.is_empty() {
                    None
                } else {
                    Some(proto_t.memo_device_id)
                },
                synced: true,
            };

            self.storage.insert_transcription(&transcription)?;

            if proto_t.timestamp > latest_timestamp {
                latest_timestamp = proto_t.timestamp;
            }

            count += 1;
            debug!("Synced transcription: {}", proto_t.text);
        }

        // Update peer sync timestamp
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.storage.upsert_peer(&Peer {
            node_id: peer_conn.node_id.clone(),
            last_seen: now,
            last_sync_timestamp: latest_timestamp,
        })?;

        if count > 0 {
            info!(
                "Synced {} transcriptions from {}",
                count, peer_conn.node_id
            );
        }

        Ok(())
    }
}
