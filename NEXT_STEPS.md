# Next Steps

The memo-node project is now scaffolded and ready for integration work.

## What's Complete

✅ **Project Structure**
- Cargo project with all dependencies
- Protocol buffer definitions for gRPC
- Directory structure for audio, sync, and API modules

✅ **Core Components**
- Configuration system with layered config support
- SQLite storage with migrations
- mDNS peer discovery
- gRPC peer sync (server + client)
- WebSocket API for memo-desktop
- BLE audio receiver
- Opus decoder
- Whisper transcription pipeline (placeholder)
- CLI with start/status/logs commands

✅ **Documentation**
- README with usage instructions
- ARCHITECTURE.md with detailed system design
- Inline code documentation

## Before First Run

### 1. Install Protocol Buffers Compiler

```bash
# macOS
brew install protobuf

# Ubuntu/Debian
sudo apt-get install protobuf-compiler
```

### 2. Update BLE UUIDs

Edit your user config at `~/.config/memo-node/config.toml`:

```toml
[audio]
memo_service_uuid = "your-actual-memo-service-uuid"
memo_characteristic_uuid = "your-actual-memo-characteristic-uuid"
```

Replace with the actual UUIDs from your Memo device firmware.

### 3. Build the Project

```bash
cd memo-node
cargo build --release
```

## Integration Tasks

### Task 1: Integrate memo-stt

**Location**: `src/transcribe.rs:95-105`

Currently there's a placeholder implementation. Once memo-stt is ready:

1. Add memo-stt as a dependency in `Cargo.toml`
2. Replace the placeholder with actual Whisper bindings:

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

3. Test with actual audio from Memo device

### Task 2: Update memo-desktop

memo-desktop should connect to memo-node instead of doing its own audio processing.

**Changes needed**:

1. **Remove** from memo-desktop:
   - Direct BLE connection code
   - Whisper transcription code
   - Audio processing pipeline

2. **Add** to memo-desktop:
   - WebSocket client connecting to `ws://127.0.0.1:9877`
   - Message handlers for transcription events
   - History fetch on startup

Example integration:

```typescript
// In memo-desktop main process or renderer
import WebSocket from 'ws';

const ws = new WebSocket('ws://127.0.0.1:9877');

ws.on('open', () => {
  console.log('Connected to memo-node');

  // Request history on startup
  ws.send(JSON.stringify({
    type: 'get_history',
    data: { limit: 100 }
  }));
});

ws.on('message', (data) => {
  const msg = JSON.parse(data.toString());

  switch (msg.type) {
    case 'transcription':
      // New transcription from any node
      displayTranscription(msg.data);
      break;

    case 'history':
      // Initial history response
      displayHistory(msg.data.transcriptions);
      break;

    case 'peer_connected':
      console.log(`Peer connected: ${msg.data.node_id}`);
      break;

    case 'peer_disconnected':
      console.log(`Peer disconnected: ${msg.data.node_id}`);
      break;
  }
});

ws.on('error', (error) => {
  console.error('WebSocket error:', error);
});
```

3. **Test** the integration:
   - Start memo-node: `memo-node start`
   - Start memo-desktop
   - Verify transcriptions appear in memo-desktop
   - Test with Memo device on both MacBook and Pi

### Task 3: Test Peer Sync

**On MacBook**:
```bash
memo-node start
```

**On Raspberry Pi**:
```bash
# Create config at ~/.config/memo-node/config.toml
[node]
id = "pi-workshop"

[transcription]
model = "tiny.en"  # Lighter model for Pi

# Then start
memo-node start
```

**Verify**:
1. Check logs to see peer discovery
2. Create transcription on Pi
3. Verify it appears on MacBook via `memo-node logs`
4. Verify it appears in memo-desktop

### Task 4: Raspberry Pi Deployment

1. Cross-compile for ARM or build on Pi:
   ```bash
   cargo build --release --target aarch64-unknown-linux-gnu
   ```

2. Copy binary to Pi:
   ```bash
   scp target/release/memo-node pi@raspberrypi:~/
   ```

3. Create systemd service:
   ```ini
   [Unit]
   Description=Memo Network Node
   After=network.target bluetooth.target

   [Service]
   Type=simple
   User=pi
   ExecStart=/home/pi/memo-node start
   Restart=always
   RestartSec=10

   [Install]
   WantedBy=multi-user.target
   ```

4. Enable and start:
   ```bash
   sudo systemctl enable memo-node
   sudo systemctl start memo-node
   ```

## Testing Strategy

### Unit Tests

Add tests for core components:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_roundtrip() {
        // Test storing and retrieving transcriptions
    }

    #[test]
    fn test_opus_decode() {
        // Test Opus decoder with sample data
    }

    #[tokio::test]
    async fn test_peer_sync() {
        // Test gRPC sync between two instances
    }
}
```

### Integration Tests

1. **Local two-node setup**:
   - Run two instances on same machine with different ports
   - Verify peer discovery
   - Verify transcription sync

2. **BLE connection**:
   - Connect Memo device
   - Verify audio packets received
   - Verify transcription produced

3. **WebSocket API**:
   - Connect test client
   - Request history
   - Verify real-time events

### End-to-End Tests

1. MacBook + Pi + Memo device + memo-desktop all running
2. Record audio on Memo
3. Verify transcription appears everywhere
4. Disconnect Pi, verify MacBook continues
5. Reconnect Pi, verify sync catches up

## Known Limitations

1. **No Whisper implementation yet**: Waiting for memo-stt integration
2. **BLE UUIDs are placeholders**: Need actual Memo device UUIDs
3. **No authentication**: Assumes trusted local network
4. **Pull-based sync only**: Push sync is defined but not implemented
5. **No error recovery for BLE**: If device disconnects, needs manual reconnect

## Optional Enhancements

### Nice-to-Have Features

1. **Push-based sync**: Implement `PushTranscriptions` RPC for instant sync
2. **Conflict resolution**: Handle same transcription from multiple sources
3. **Transcription editing**: API to edit/delete transcriptions
4. **Search**: Full-text search across all transcriptions
5. **Export**: Export transcriptions to JSON/CSV
6. **Web UI**: Simple web interface as alternative to memo-desktop
7. **Cloud sync**: Optional cloud peer for backup/access from anywhere

### Performance Optimizations

1. **Audio buffering**: Implement voice activity detection to avoid transcribing silence
2. **Batch sync**: Sync multiple transcriptions in one request
3. **Incremental sync**: Only sync new transcriptions, not re-query all
4. **Connection pooling**: Reuse gRPC connections to peers

### Deployment Improvements

1. **Docker support**: Dockerfile for easy deployment
2. **Auto-updates**: Self-update mechanism
3. **Health checks**: Expose health check endpoint
4. **Metrics**: Prometheus metrics for monitoring

## Questions to Resolve

1. **Audio format**: Confirm Memo device sends Opus at 16kHz mono
2. **BLE packet size**: What's the typical packet size? Affects buffering
3. **Whisper model**: Which model should be default? base.en vs small.en?
4. **Sync frequency**: Is 30 seconds the right interval?
5. **History limit**: Should there be a max transcription count? Auto-prune old ones?

## Support

If you encounter issues:

1. Check logs: `RUST_LOG=debug memo-node start`
2. Verify config: `cat ~/.config/memo-node/config.toml`
3. Check status: `memo-node status`
4. Verify database: `sqlite3 ~/.memo/transcriptions.db "SELECT * FROM transcriptions LIMIT 10;"`

## Summary

The foundation is complete. The main integration points are:

1. **memo-stt**: Replace placeholder in `src/transcribe.rs`
2. **memo-desktop**: Connect to WebSocket API instead of direct BLE
3. **Memo device**: Provide actual BLE UUIDs

Once these are integrated, you'll have a complete system where:
- Memo device → either MacBook or Pi
- Transcription happens on whichever node received the audio
- All transcriptions sync between nodes
- memo-desktop displays everything from all sources

The architecture is ready to support future features like intent classification and command routing without major changes.
