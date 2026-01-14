# memo-node

Headless daemon for the Memo Network. Handles BLE audio from Memo devices, transcribes via Whisper, and syncs transcriptions across nodes.

## Architecture

```
┌──────────┐         ┌──────────────────┐
│   Memo   │──BLE───▶│   Raspberry Pi   │
│  Device  │         │   (memo-node)    │
└──────────┘         └────────┬─────────┘
      │                       │
      │                       │ gRPC sync
      │                       │
      │              ┌────────▼─────────┐
      └─────BLE─────▶│     MacBook      │
                     │   (memo-node +   │
                     │   memo-desktop)  │
                     └──────────────────┘
```

## Features

- **BLE Audio Ingress**: Connects to Memo devices via Bluetooth LE
- **Whisper Transcription**: Local speech-to-text using Whisper models
- **SQLite Storage**: Persistent local storage of transcriptions
- **Peer Sync**: Automatic discovery and sync via mDNS + gRPC
- **WebSocket API**: Local API for memo-desktop integration

## Prerequisites

Install Protocol Buffers compiler:

```bash
# macOS
brew install protobuf

# Ubuntu/Debian
sudo apt-get install protobuf-compiler

# Or download from https://github.com/protocolbuffers/protobuf/releases
```

## Installation

```bash
cargo build --release
```

## Configuration

Configuration is loaded from:
1. `config/default.toml` (embedded defaults)
2. `~/.config/memo-node/config.toml` (user overrides)
3. Environment variables (`MEMO_NODE_*`)

### Example User Config

Create `~/.config/memo-node/config.toml`:

```toml
[node]
id = "macbook-oliver"  # or "pi-workshop"

[audio]
memo_service_uuid = "your-memo-service-uuid"
memo_characteristic_uuid = "your-memo-characteristic-uuid"

[transcription]
model = "base.en"  # or "tiny.en" for Raspberry Pi
```

## Usage

### Start the daemon

```bash
memo-node start
```

The daemon will:
- Listen for BLE connections from Memo devices
- Transcribe audio locally via Whisper
- Discover and sync with peer nodes on the network
- Expose WebSocket API on `127.0.0.1:9877` for memo-desktop

### Check status

```bash
memo-node status
```

Output:
```
Node: macbook-oliver
Transcriptions: 47 local, 23 synced
Peers:
  pi-workshop (last seen 5s ago)
```

### View logs

```bash
memo-node logs --limit 10
```

## API

### WebSocket (memo-desktop)

Connect to `ws://127.0.0.1:9877`

#### Server → Client

```json
{
  "type": "transcription",
  "data": {
    "id": "abc123",
    "timestamp": 1234567890,
    "text": "Remember to call Kevin tomorrow",
    "source_node": "pi-workshop",
    "memo_device_id": null
  }
}
```

```json
{
  "type": "peer_connected",
  "data": { "node_id": "pi-workshop" }
}
```

#### Client → Server

```json
{
  "type": "get_history",
  "data": { "limit": 100 }
}
```

### gRPC (peer sync)

Nodes sync via gRPC on port `9876`. See `proto/memo.proto` for the full protocol.

## Directory Structure

```
memo-node/
├── Cargo.toml
├── build.rs
├── config/
│   └── default.toml
├── proto/
│   └── memo.proto
└── src/
    ├── main.rs           # CLI entry point
    ├── config.rs         # Configuration loading
    ├── storage.rs        # SQLite storage
    ├── transcribe.rs     # Whisper integration (placeholder)
    ├── audio/
    │   ├── mod.rs
    │   ├── ble.rs        # BLE audio receiver
    │   └── decoder.rs    # Opus decoder
    ├── sync/
    │   ├── mod.rs
    │   ├── discovery.rs  # mDNS discovery
    │   └── peer.rs       # gRPC peer sync
    └── api/
        └── websocket.rs  # WebSocket server for memo-desktop
```

## Integration Points

### memo-stt

The transcription pipeline in `src/transcribe.rs` currently has a placeholder implementation. Once `memo-stt` is ready with Whisper bindings, integrate it here:

```rust
// In src/transcribe.rs
async fn transcribe_audio(&self, audio: &[i16]) -> Result<String> {
    let samples_f32: Vec<f32> = audio
        .iter()
        .map(|&s| s as f32 / 32768.0)
        .collect();

    let text = memo_stt::transcribe(&samples_f32, &self.model_path)?;
    Ok(text)
}
```

### memo-desktop

memo-desktop should connect to the local WebSocket API instead of doing its own audio processing:

```typescript
const ws = new WebSocket('ws://127.0.0.1:9877');

ws.on('message', (data) => {
  const msg = JSON.parse(data);

  if (msg.type === 'transcription') {
    displayTranscription(msg.data);
  }
});

// Request history on startup
ws.send(JSON.stringify({
  type: 'get_history',
  data: { limit: 100 }
}));
```

## Development

### Run with debug logging

```bash
RUST_LOG=debug cargo run -- start
```

### Testing peer sync

Run two instances with different configs:

```bash
# Terminal 1 (MacBook)
MEMO_NODE_NODE_ID=macbook-oliver \
MEMO_NODE_SYNC_GRPC_PORT=9876 \
MEMO_NODE_API_WEBSOCKET_PORT=9877 \
cargo run -- start

# Terminal 2 (simulated Pi)
MEMO_NODE_NODE_ID=pi-workshop \
MEMO_NODE_SYNC_GRPC_PORT=9976 \
MEMO_NODE_API_WEBSOCKET_PORT=9977 \
cargo run -- start
```

## License

MIT
