use crate::storage::{Storage, Transcription};
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, RwLock};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ServerMessage {
    #[serde(rename = "transcription")]
    Transcription {
        id: String,
        timestamp: i64,
        text: String,
        source_node: String,
        memo_device_id: Option<String>,
    },
    #[serde(rename = "peer_connected")]
    PeerConnected { node_id: String },
    #[serde(rename = "peer_disconnected")]
    PeerDisconnected { node_id: String },
    #[serde(rename = "history")]
    History { transcriptions: Vec<TranscriptionData> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionData {
    pub id: String,
    pub timestamp: i64,
    pub text: String,
    pub source_node: String,
    pub memo_device_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ClientMessage {
    #[serde(rename = "get_history")]
    GetHistory { limit: Option<usize> },
}

pub struct WebSocketServer {
    storage: Storage,
    broadcast_tx: broadcast::Sender<Transcription>,
    clients: Arc<RwLock<Vec<broadcast::Sender<ServerMessage>>>>,
}

impl WebSocketServer {
    pub fn new(
        storage: Storage,
        broadcast_tx: broadcast::Sender<Transcription>,
    ) -> Self {
        Self {
            storage,
            broadcast_tx,
            clients: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn serve(self, addr: SocketAddr) -> Result<()> {
        let listener = TcpListener::bind(addr)
            .await
            .context("Failed to bind WebSocket server")?;

        info!("WebSocket server listening on {}", addr);

        let server = Arc::new(self);

        // Spawn task to broadcast transcriptions to all clients
        let server_clone = server.clone();
        tokio::spawn(async move {
            server_clone.broadcast_loop().await;
        });

        while let Ok((stream, peer_addr)) = listener.accept().await {
            let server = server.clone();
            tokio::spawn(async move {
                if let Err(e) = server.handle_connection(stream, peer_addr).await {
                    error!("WebSocket error for {}: {}", peer_addr, e);
                }
            });
        }

        Ok(())
    }

    async fn broadcast_loop(&self) {
        let mut rx = self.broadcast_tx.subscribe();

        while let Ok(transcription) = rx.recv().await {
            let msg = ServerMessage::Transcription {
                id: transcription.id,
                timestamp: transcription.timestamp,
                text: transcription.text,
                source_node: transcription.source_node,
                memo_device_id: transcription.memo_device_id,
            };

            self.broadcast_to_clients(msg).await;
        }
    }

    async fn broadcast_to_clients(&self, msg: ServerMessage) {
        let clients = self.clients.read().await;

        for client_tx in clients.iter() {
            if let Err(e) = client_tx.send(msg.clone()) {
                warn!("Failed to broadcast to client: {}", e);
            }
        }
    }

    async fn handle_connection(&self, stream: TcpStream, addr: SocketAddr) -> Result<()> {
        info!("New WebSocket connection from {}", addr);

        let ws_stream = tokio_tungstenite::accept_async(stream)
            .await
            .context("Failed to accept WebSocket connection")?;

        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        // Create a channel for this client
        let (client_tx, mut client_rx) = broadcast::channel::<ServerMessage>(100);
        let (response_tx, mut response_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

        // Add client to the list
        {
            let mut clients = self.clients.write().await;
            clients.push(client_tx);
        }

        // Spawn task to send messages to this client
        let send_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = client_rx.recv() => {
                        match result {
                            Ok(msg) => {
                                if let Ok(json) = serde_json::to_string(&msg) {
                                    if ws_sender.send(Message::Text(json)).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    result = response_rx.recv() => {
                        match result {
                            Some(msg) => {
                                if ws_sender.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        });

        // Handle incoming messages from client
        while let Some(msg_result) = ws_receiver.next().await {
            match msg_result {
                Ok(Message::Text(text)) => {
                    debug!("Received message from {}: {}", addr, text);

                    if let Err(e) = self.handle_client_message(&text, &response_tx).await {
                        error!("Error handling client message: {}", e);
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("Client {} closed connection", addr);
                    break;
                }
                Ok(Message::Ping(data)) => {
                    let _ = response_tx.send(Message::Pong(data));
                }
                Err(e) => {
                    error!("WebSocket error for {}: {}", addr, e);
                    break;
                }
                _ => {}
            }
        }

        send_task.abort();
        info!("Connection closed for {}", addr);

        Ok(())
    }

    async fn handle_client_message(
        &self,
        text: &str,
        response_tx: &tokio::sync::mpsc::UnboundedSender<Message>,
    ) -> Result<()> {
        let client_msg: ClientMessage = serde_json::from_str(text)
            .context("Failed to parse client message")?;

        match client_msg {
            ClientMessage::GetHistory { limit } => {
                let transcriptions = self
                    .storage
                    .get_recent_transcriptions(limit.unwrap_or(100))?;

                let data: Vec<TranscriptionData> = transcriptions
                    .into_iter()
                    .map(|t| TranscriptionData {
                        id: t.id,
                        timestamp: t.timestamp,
                        text: t.text,
                        source_node: t.source_node,
                        memo_device_id: t.memo_device_id,
                    })
                    .collect();

                let response = ServerMessage::History {
                    transcriptions: data,
                };

                let json = serde_json::to_string(&response)?;
                response_tx.send(Message::Text(json))?;
            }
        }

        Ok(())
    }

    pub async fn notify_peer_connected(&self, node_id: String) {
        let msg = ServerMessage::PeerConnected { node_id };
        self.broadcast_to_clients(msg).await;
    }

    pub async fn notify_peer_disconnected(&self, node_id: String) {
        let msg = ServerMessage::PeerDisconnected { node_id };
        self.broadcast_to_clients(msg).await;
    }
}
