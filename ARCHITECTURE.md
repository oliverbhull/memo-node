# memo-node Architecture

This document describes the internal architecture of memo-node.

## System Components

### 1. Audio Pipeline

```
Memo Device (BLE)
      │
      ▼
BleAudioReceiver (src/audio/ble.rs)
      │ Opus-encoded packets
      ▼
OpusDecoder (src/audio/decoder.rs)
      │ PCM i16 samples
      ▼
WhisperTranscriber (src/transcribe.rs)
      │ Transcription text
      ▼
Storage + Broadcast
```

**BleAudioReceiver** (`src/audio/ble.rs`)
- Scans for BLE devices with the configured service UUID
- Connects to Memo devices automatically
- Subscribes to audio characteristic notifications
- Forwards raw Opus packets to the decoder

**OpusDecoder** (`src/audio/decoder.rs`)
- Decodes Opus audio frames to PCM samples
- Configured for 16kHz mono audio
- Produces i16 samples for Whisper

**WhisperTranscriber** (`src/transcribe.rs`)
- Accumulates audio samples into buffers
- Transcribes using Whisper (placeholder for memo-stt integration)
- Emits transcription text

### 2. Storage Layer

**Storage** (`src/storage.rs`)
- SQLite database with two tables:
  - `transcriptions`: All transcribed text with metadata
  - `peers`: Known peer nodes and sync state
- Thread-safe via Arc<Mutex<Connection>>
- Handles queries for recent history, sync status, etc.

Schema:
```sql
CREATE TABLE transcriptions (
    id TEXT PRIMARY KEY,              -- UUID
    timestamp INTEGER NOT NULL,        -- Unix timestamp
    text TEXT NOT NULL,               -- Transcription
    source_node TEXT NOT NULL,        -- Which node created it
    memo_device_id TEXT,              -- Optional device ID
    synced INTEGER DEFAULT 0          -- Whether it came from peer
);

CREATE TABLE peers (
    node_id TEXT PRIMARY KEY,
    last_seen INTEGER,
    last_sync_timestamp INTEGER       -- Last timestamp synced from this peer
);
```

### 3. Peer Discovery & Sync

**Discovery** (`src/sync/discovery.rs`)
- mDNS service announcement and browsing
- Advertises `_memo-node._tcp.local.` with:
  - `node_id`: Unique node identifier
  - `grpc_port`: Port for peer sync
- Discovers other nodes on the local network
- Sends discovered peers to PeerManager

**PeerManager** (`src/sync/peer.rs`)
- Maintains a registry of known peers
- Runs periodic sync loop (default: every 30 seconds)
- For each peer:
  1. Connects via gRPC
  2. Requests transcriptions since last sync
  3. Stores new transcriptions
  4. Updates peer sync timestamp

**PeerSyncServer** (`src/sync/peer.rs`)
- gRPC server implementing the MemoSync service
- Handles incoming sync requests from peers
- Streams transcriptions to requesting nodes
- Receives pushed transcriptions from peers

### 4. WebSocket API

**WebSocketServer** (`src/api/websocket.rs`)
- Local-only WebSocket server for memo-desktop
- Broadcasts new transcriptions to all connected clients
- Handles client commands:
  - `get_history`: Fetch recent transcriptions
- Sends events:
  - `transcription`: New transcription available
  - `peer_connected`: New peer discovered
  - `peer_disconnected`: Peer went offline

### 5. Configuration

**Config** (`src/config.rs`)
- Layered configuration system:
  1. Default config (embedded `config/default.toml`)
  2. User config (`~/.config/memo-node/config.toml`)
  3. Environment variables (`MEMO_NODE_*`)
- Handles path expansion (e.g., `~/.memo`)
- Creates necessary directories

## Data Flow

### Scenario 1: Local Transcription

```
1. Memo device connects via BLE
2. Audio packets → BleAudioReceiver
3. Opus decode → PCM samples
4. Whisper transcription → text
5. Store in SQLite with synced=false
6. Broadcast to WebSocket clients (memo-desktop)
7. Background sync picks up and pushes to peers
```

### Scenario 2: Peer Sync

```
1. PeerManager sync loop ticks
2. For each peer:
   a. Query last_sync_timestamp from DB
   b. gRPC GetTranscriptionsSince(last_sync_timestamp)
   c. Store received transcriptions with synced=true
   d. Broadcast to WebSocket clients
   e. Update peer last_sync_timestamp
```

### Scenario 3: memo-desktop Connection

```
1. memo-desktop connects to ws://127.0.0.1:9877
2. Sends { type: "get_history", data: { limit: 100 } }
3. Receives history response with all transcriptions
4. Subscribes to live transcription events
5. Updates UI in real-time
```

## Thread Model

The daemon uses Tokio for async execution:

```
Main Thread
  ├─ WebSocket Server Task
  │    └─ Per-client connection tasks
  ├─ gRPC Server Task
  │    └─ Per-request handler tasks
  ├─ Peer Manager Sync Loop Task
  ├─ mDNS Discovery Event Loop Task
  ├─ BLE Scanner Task
  │    └─ Per-device notification tasks
  ├─ Opus Decoder Task
  ├─ Whisper Transcriber Task
  └─ Transcription Storage Task
```

All tasks communicate via:
- `mpsc::unbounded_channel` for audio/transcription pipeline
- `broadcast::channel` for transcription events
- `Arc<RwLock<HashMap>>` for peer registry
- `Arc<Mutex<Connection>>` for SQLite

## Protocol Details

### gRPC (proto/memo.proto)

**Ping**: Health check between peers
```protobuf
rpc Ping(PingRequest) returns (PingResponse);
```

**GetTranscriptionsSince**: Pull transcriptions from peer
```protobuf
rpc GetTranscriptionsSince(SinceRequest) returns (stream Transcription);
```
- Client provides timestamp
- Server streams all transcriptions newer than that timestamp
- Client stores them and updates sync timestamp

**PushTranscriptions**: Push transcriptions to peer
```protobuf
rpc PushTranscriptions(stream Transcription) returns (PushResponse);
```
- Currently unused (pull-based sync only)
- Reserved for future push-based updates

### WebSocket (JSON)

**Server → Client Messages**:

```json
{
  "type": "transcription",
  "data": {
    "id": "uuid",
    "timestamp": 1234567890,
    "text": "transcription text",
    "source_node": "node-id",
    "memo_device_id": "device-id"
  }
}
```

```json
{ "type": "peer_connected", "data": { "node_id": "pi-workshop" } }
{ "type": "peer_disconnected", "data": { "node_id": "pi-workshop" } }
```

```json
{
  "type": "history",
  "data": {
    "transcriptions": [
      { "id": "...", "timestamp": ..., "text": "...", ... }
    ]
  }
}
```

**Client → Server Messages**:

```json
{ "type": "get_history", "data": { "limit": 100 } }
```

## Integration Points

### memo-stt Integration

Location: `src/transcribe.rs:95-105`

Replace the placeholder with actual Whisper bindings:

```rust
async fn transcribe_audio(&self, audio: &[i16]) -> Result<String> {
    // Convert i16 to f32
    let samples_f32: Vec<f32> = audio
        .iter()
        .map(|&s| s as f32 / 32768.0)
        .collect();

    // Call memo-stt
    let text = memo_stt::transcribe(&samples_f32, &self.model_path)?;
    Ok(text)
}
```

### memo-desktop Integration

memo-desktop should:
1. Remove its own BLE and Whisper code
2. Connect to `ws://127.0.0.1:9877`
3. Request history on startup
4. Listen for real-time transcription events
5. Display all transcriptions (local + synced)

Example:
```typescript
const ws = new WebSocket('ws://127.0.0.1:9877');

ws.onopen = () => {
  ws.send(JSON.stringify({
    type: 'get_history',
    data: { limit: 100 }
  }));
};

ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);

  if (msg.type === 'transcription') {
    displayNewTranscription(msg.data);
  } else if (msg.type === 'history') {
    displayHistory(msg.data.transcriptions);
  }
};
```

## Error Handling

- BLE connection failures: Log and retry on next scan
- Opus decode errors: Skip frame, continue
- Whisper failures: Log error, don't crash
- Storage errors: Propagate to caller, log
- gRPC sync failures: Log warning, retry on next sync loop
- WebSocket disconnect: Clean up client, continue serving others

## Security Considerations

Current implementation assumes:
- Trusted local network (no authentication between nodes)
- Local-only WebSocket (127.0.0.1)
- No encryption on peer sync (plain gRPC)

For production deployment:
- Add mutual TLS for gRPC
- Add authentication tokens for WebSocket
- Add node allowlist/denylist
- Encrypt database at rest

## Performance

### Memory
- SQLite holds full transcription history on disk
- In-memory: Only active connections and buffers
- Audio buffers: ~2 seconds × 16kHz × 2 bytes = 64KB
- Transcription broadcast: One copy per WebSocket client

### CPU
- Opus decode: Minimal overhead
- Whisper transcription: Most expensive operation
  - `tiny.en`: ~50ms on Pi, ~20ms on MacBook
  - `base.en`: ~200ms on Pi, ~50ms on MacBook
- mDNS: Minimal background overhead
- gRPC sync: Negligible (infrequent, small payloads)

### Network
- BLE: ~20 kbps (Opus compressed audio)
- gRPC sync: ~1KB per transcription, sync every 30s
- mDNS: ~100 bytes every 30s

## Future Extensions

This architecture supports future capabilities:

1. **Intent Classification**: Parse transcriptions in storage task, route to handlers
2. **Command Execution**: Add execution engine, route commands to appropriate nodes
3. **Multi-device Support**: Already tracked via `memo_device_id`
4. **Cloud Sync**: Add cloud peer with authentication
5. **Search**: Add full-text search on transcriptions table
6. **Voice Match**: Add speaker identification to transcriptions
