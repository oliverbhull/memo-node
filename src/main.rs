mod api;
mod audio;
mod config;
mod storage;
mod sync;
mod transcribe;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

use api::WebSocketServer;
use audio::{BleAudioReceiver, OpusDecoder};
use config::Config;
use storage::{Storage, Transcription};
use sync::{Discovery, PeerManager, PeerSyncServer};
use transcribe::WhisperTranscriber;

#[derive(Parser)]
#[command(name = "memo-node")]
#[command(about = "Memo Network Node - Transcription and sync daemon", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the memo-node daemon
    Start,
    /// Show node status
    Status,
    /// Show recent transcription logs
    Logs {
        /// Number of logs to show
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "memo_node=debug,info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Start => start_daemon().await,
        Commands::Status => show_status().await,
        Commands::Logs { limit } => show_logs(limit).await,
    }
}

async fn start_daemon() -> Result<()> {
    info!("Starting memo-node daemon");

    // Load configuration
    let config = Config::load()?;
    info!("Node ID: {}", config.node.id);

    // Initialize storage
    let storage_path = config.storage_path()?;
    let storage = Storage::new(&storage_path)?;
    info!("Storage initialized at {}", storage_path.display());

    // Create channels for new transcriptions
    let (transcription_tx, transcription_rx) = mpsc::unbounded_channel::<Transcription>();
    let (ws_broadcast_tx, _) = broadcast::channel::<Transcription>(100);

    // Initialize WebSocket server for memo-desktop
    let ws_addr = format!("{}:{}", config.api.listen_address, config.api.websocket_port)
        .parse()
        .context("Invalid WebSocket address")?;
    let ws_server = WebSocketServer::new(storage.clone(), ws_broadcast_tx.clone());

    tokio::spawn(async move {
        if let Err(e) = ws_server.serve(ws_addr).await {
            error!("WebSocket server error: {}", e);
        }
    });

    // Initialize gRPC server for peer sync
    let grpc_server = PeerSyncServer::new(
        config.node.id.clone(),
        storage.clone(),
        transcription_tx.clone(),
    );
    let grpc_port = config.sync.grpc_port;

    tokio::spawn(async move {
        if let Err(e) = grpc_server.serve(grpc_port).await {
            error!("gRPC server error: {}", e);
        }
    });

    // Bridge: forward transcriptions from gRPC to WebSocket broadcast
    let ws_broadcast_tx_clone = ws_broadcast_tx.clone();
    tokio::spawn(async move {
        let mut rx = transcription_rx;
        while let Some(transcription) = rx.recv().await {
            let _ = ws_broadcast_tx_clone.send(transcription);
        }
    });

    // Initialize peer manager
    let peer_manager = Arc::new(PeerManager::new(
        config.node.id.clone(),
        storage.clone(),
        config.sync.sync_interval,
    ));

    // Start sync loop
    let peer_manager_clone = peer_manager.clone();
    tokio::spawn(async move {
        peer_manager_clone.start_sync_loop().await;
    });

    // Initialize mDNS discovery
    let (discovery, mut peer_rx) = Discovery::new(config.node.id.clone(), config.sync.grpc_port)?;
    discovery.start()?;

    // Handle discovered peers
    let peer_manager_clone = peer_manager.clone();
    tokio::spawn(async move {
        while let Some(peer) = peer_rx.recv().await {
            info!("Adding peer: {} at {}:{}", peer.node_id, peer.address, peer.grpc_port);
            peer_manager_clone
                .add_peer(peer.node_id, peer.address, peer.grpc_port)
                .await;
        }
    });

    // Initialize audio pipeline
    let service_uuid = config
        .audio
        .memo_service_uuid
        .parse()
        .context("Invalid service UUID")?;
    let char_uuid = config
        .audio
        .memo_characteristic_uuid
        .parse()
        .context("Invalid characteristic UUID")?;

    let (ble_receiver, mut audio_rx) = BleAudioReceiver::new(service_uuid, char_uuid);
    let ble_receiver = Arc::new(ble_receiver);

    tokio::spawn(async move {
        if let Err(e) = ble_receiver.start().await {
            error!("BLE receiver error: {}", e);
        }
    });

    // Initialize audio decoder
    let (decoded_tx, decoded_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut decoder = OpusDecoder::new(16000, opus::Channels::Mono).unwrap();

        while let Some(encoded_audio) = audio_rx.recv().await {
            match decoder.decode(&encoded_audio) {
                Ok(decoded) => {
                    if let Err(e) = decoded_tx.send(decoded) {
                        error!("Failed to send decoded audio: {}", e);
                    }
                }
                Err(e) => {
                    error!("Failed to decode audio: {}", e);
                }
            }
        }
    });

    // Initialize transcriber
    let (transcriber, mut transcription_rx) =
        WhisperTranscriber::new(&config.transcription.model, decoded_rx);

    tokio::spawn(async move {
        if let Err(e) = transcriber.start().await {
            error!("Transcriber error: {}", e);
        }
    });

    // Handle transcriptions
    let node_id = config.node.id.clone();
    let storage_clone = storage.clone();
    let ws_broadcast_tx_clone2 = ws_broadcast_tx.clone();

    tokio::spawn(async move {
        while let Some(text) = transcription_rx.recv().await {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            let transcription = Transcription {
                id: Uuid::new_v4().to_string(),
                timestamp,
                text,
                source_node: node_id.clone(),
                memo_device_id: None,
                synced: false,
            };

            if let Err(e) = storage_clone.insert_transcription(&transcription) {
                error!("Failed to store transcription: {}", e);
            } else {
                info!("Stored transcription: {}", transcription.text);
                let _ = ws_broadcast_tx_clone2.send(transcription);
            }
        }
    });

    info!("memo-node daemon started successfully");
    info!("WebSocket API: {}:{}", config.api.listen_address, config.api.websocket_port);
    info!("gRPC peer sync: 0.0.0.0:{}", config.sync.grpc_port);

    // Keep running
    tokio::signal::ctrl_c().await?;
    info!("Shutting down...");

    Ok(())
}

async fn show_status() -> Result<()> {
    let config = Config::load()?;
    let storage_path = config.storage_path()?;
    let storage = Storage::new(&storage_path)?;

    let (total, synced) = storage.count_transcriptions()?;
    let local = total - synced;
    let peers = storage.get_peers()?;

    println!("Node: {}", config.node.id);
    println!("Transcriptions: {} local, {} synced", local, synced);
    println!("Peers:");

    if peers.is_empty() {
        println!("  (none)");
    } else {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        for peer in peers {
            let seconds_ago = now - peer.last_seen;
            println!("  {} (last seen {}s ago)", peer.node_id, seconds_ago);
        }
    }

    Ok(())
}

async fn show_logs(limit: usize) -> Result<()> {
    let config = Config::load()?;
    let storage_path = config.storage_path()?;
    let storage = Storage::new(&storage_path)?;

    let transcriptions = storage.get_recent_transcriptions(limit)?;

    if transcriptions.is_empty() {
        println!("No transcriptions yet");
        return Ok(());
    }

    println!("Recent transcriptions:");
    for t in transcriptions.iter().rev() {
        let timestamp = chrono::DateTime::from_timestamp(t.timestamp, 0)
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S");
        println!(
            "[{}] [{}] {}",
            timestamp, t.source_node, t.text
        );
    }

    Ok(())
}
