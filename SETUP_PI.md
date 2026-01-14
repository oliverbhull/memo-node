# Raspberry Pi Setup Guide for memo-node

This guide walks you through setting up and testing memo-node on Raspberry Pi 5 with AI Hat.

## Prerequisites

### 1. Install System Dependencies

```bash
# Update package list
sudo apt-get update

# Install required system packages
sudo apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    libasound2-dev \
    bluez \
    libbluetooth-dev \
    libdbus-1-dev \
    libclang-dev \
    clang \
    libx11-dev \
    libxi-dev \
    libxext-dev \
    libxtst-dev

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### 2. Verify BLE Hardware

```bash
# Check if Bluetooth is available
hciconfig

# If Bluetooth is not running, start it
sudo systemctl start bluetooth
sudo systemctl enable bluetooth

# Check Bluetooth status
sudo systemctl status bluetooth
```

### 3. Set Up BLE Permissions

```bash
# Add your user to the bluetooth group (if not already)
sudo usermod -aG bluetooth $USER

# You may need to log out and back in for group changes to take effect
# Or use newgrp to apply immediately:
newgrp bluetooth

# Verify you're in the bluetooth group
groups
```

## Building the Projects

### 1. Build memo-stt First

Since memo-node depends on memo-stt, build it first:

```bash
cd ~/memo/memo-stt

# Build ONLY the library (not the binary) - memo-node only needs the library
# This avoids X11 dependencies which aren't needed for headless operation
cargo build --release --lib

# Verify the build succeeded (library only, no binary)
ls -lh target/release/libmemo_stt*.rlib target/release/libmemo_stt*.so 2>/dev/null || echo "Library built successfully"
```

**Note**: We use `--lib` to build only the library, not the binary. The binary requires X11 libraries for keyboard/mouse control, but memo-node only needs the STT engine library. This makes the build faster and avoids unnecessary dependencies.

**Note**: The first build will download and compile many dependencies. This can take 30-60 minutes on a Raspberry Pi. Subsequent builds will be much faster.

### 2. Build memo-node

```bash
cd ~/memo/memo-node

# Build in release mode
cargo build --release

# Verify the build succeeded
ls -lh target/release/memo-node
```

## Configuration

### 1. Create User Configuration

Create the configuration directory and file:

```bash
mkdir -p ~/.config/memo-node

# Create config file
cat > ~/.config/memo-node/config.toml << 'EOF'
[node]
id = "pi-workshop"  # Change this to your preferred node name

[audio]
# BLE UUIDs are already set in default.toml, but you can override here if needed
# memo_service_uuid = "1234A000-1234-5678-1234-56789ABCDEF0"
# memo_characteristic_uuid = "1234A001-1234-5678-1234-56789ABCDEF0"

[transcription]
# Use base.en for good balance, or small.en for higher accuracy
model = "base.en"
# Number of threads (4-6 recommended for Pi 5)
threads = 4

[api]
# Optional: Set your HTTPS endpoint URL here
# Leave empty to disable HTTPS posting
https_endpoint = "http://localhost:6969"
# Example: https_endpoint = "https://api.example.com/transcriptions"

[storage]
# Database will be stored at ~/.memo/transcriptions.db by default
# path = "~/.memo/transcriptions.db"
EOF
```

### 2. Verify Configuration

```bash
# Check that config file exists
cat ~/.config/memo-node/config.toml

# Test configuration loading (should not error)
~/memo/memo-node/target/release/memo-node status
```

## Testing the Setup

### 1. Test BLE Scanning (Without Device)

First, verify BLE scanning works:

```bash
# Run with debug logging to see BLE activity
RUST_LOG=debug ~/memo/memo-node/target/release/memo-node start
```

You should see:
- "Starting BLE audio receiver"
- "Using BLE adapter: ..."
- "Scanning for Memo devices with service UUID ..."

Press `Ctrl+C` to stop.

### 2. Test with Memo Device

1. **Power on your memo device** (XIAO nRF52840 BLE Sense)
2. **Ensure device is in advertising mode** (should show red LED)

3. **Start memo-node**:
```bash
RUST_LOG=info ~/memo/memo-node/target/release/memo-node start
```

4. **Expected behavior**:
   - memo-node should detect the device: "Found Memo device: memo_Gen0v2"
   - Connection: "Connected to memo_Gen0v2"
   - Audio subscription: "Subscribed to audio from memo_Gen0v2"
   - When you speak into the device, you should see:
     - "Received audio chunk: X samples"
     - "Transcribing X samples"
     - "Transcribed: [your speech]"
     - "Stored transcription: [your speech]"

5. **Check status** (in another terminal):
```bash
~/memo/memo-node/target/release/memo-node status
```

6. **View recent transcriptions**:
```bash
~/memo/memo-node/target/release/memo-node logs --limit 10
```

### 3. Test HTTPS Endpoint (Optional)

If you have an HTTPS endpoint configured:

1. **Set up a test endpoint** (you can use a service like webhook.site for testing):
```bash
# Get a test webhook URL from https://webhook.site
# Then update your config:
nano ~/.config/memo-node/config.toml
# Set: https_endpoint = "https://webhook.site/your-unique-id"
```

2. **Restart memo-node** and speak into the device

3. **Check the webhook site** to see if transcriptions are being posted

## Hailo AI Hat Acceleration (Experimental)

The Raspberry Pi 5 with Hailo AI Hat (26 TOPS) can potentially accelerate Whisper transcription, but requires additional setup:

### Current Status

- **Whisper.cpp** (used by memo-stt) supports ACCEL backends that can auto-detect accelerators
- **Hailo AI Hat** integration requires:
  1. Hailo drivers and SDK installed on the Pi
  2. whisper.cpp compiled with ACCEL backend support
  3. Custom integration work to expose Hailo as an ACCEL device

### Enabling GPU/ACCEL Auto-Detection

The current implementation enables `use_gpu = true` in WhisperContextParameters, which allows whisper.cpp to:
- Auto-detect GPU backends (Metal, CUDA, Vulkan, OpenCL)
- Auto-detect ACCEL backends (like Hailo if properly configured)

### To Enable Hailo Acceleration (Future Work)

1. **Install Hailo SDK**:
   ```bash
   # Follow Hailo's official documentation for Raspberry Pi AI Kit setup
   # This typically involves installing drivers and runtime libraries
   ```

2. **Verify Hailo Detection**:
   ```bash
   # Check if Hailo device is detected
   ls /dev/hailo*
   ```

3. **Rebuild whisper-rs with ACCEL support**:
   - This may require modifying whisper-rs build configuration
   - Or using a fork that supports Hailo

### Current Optimization

For now, the best performance improvements come from:
- Using `base.en` model (smaller, faster)
- Optimized thread count (auto-detected from CPU cores)
- CPU optimizations (enabled by default in release builds)

**Note**: Hailo acceleration is experimental and requires significant integration work. The current CPU-based implementation should work well for real-time transcription with the `base.en` model.

## Troubleshooting

### BLE Connection Issues

```bash
# Check if Bluetooth is running
sudo systemctl status bluetooth

# Restart Bluetooth service
sudo systemctl restart bluetooth

# Check Bluetooth adapter
hciconfig
sudo hciconfig hci0 up  # If adapter is down

# Scan for BLE devices manually
sudo hcitool lescan
```

### Permission Issues

```bash
# If you get permission errors, ensure you're in the bluetooth group
groups | grep bluetooth

# If not, add yourself and restart
sudo usermod -aG bluetooth $USER
newgrp bluetooth

# Or run with sudo (not recommended for production)
sudo ~/memo/memo-node/target/release/memo-node start
```

### Model Download Issues

The first time you run memo-node, it will download the Whisper model (~500MB for base.en). This requires internet connectivity:

```bash
# Check internet connection
ping -c 3 8.8.8.8

# If download fails, you can manually download models to:
# ~/.cache/memo-stt/models/
```

### Build Issues

If you encounter build errors:

```bash
# Update Rust toolchain
rustup update

# Clean and rebuild
cd ~/memo/memo-node
cargo clean
cargo build --release

# Check for specific error messages
RUST_LOG=debug cargo build --release 2>&1 | tee build.log
```

### High CPU Usage

If transcription is too slow:

1. **Use a smaller model** (edit `~/.config/memo-node/config.toml`):
```toml
[transcription]
model = "tiny.en"  # Faster but less accurate
```

2. **Reduce thread count**:
```toml
[transcription]
threads = 2  # Reduce if system is overloaded
```

## Running as a Service (Optional)

To run memo-node automatically on boot:

### 1. Create Systemd Service

```bash
sudo nano /etc/systemd/system/memo-node.service
```

Add the following content:

```ini
[Unit]
Description=Memo Network Node
After=network.target bluetooth.target
Wants=bluetooth.target

[Service]
Type=simple
User=oliverhull
Group=bluetooth
WorkingDirectory=/home/oliverhull/memo/memo-node
ExecStart=/home/oliverhull/memo/memo-node/target/release/memo-node start
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal

# Environment variables
Environment="RUST_LOG=info"

[Install]
WantedBy=multi-user.target
```

**Important**: Replace `oliverhull` with your actual username!

### 2. Enable and Start Service

```bash
# Reload systemd
sudo systemctl daemon-reload

# Enable service (start on boot)
sudo systemctl enable memo-node

# Start service
sudo systemctl start memo-node

# Check status
sudo systemctl status memo-node

# View logs
sudo journalctl -u memo-node -f
```

### 3. Service Management

```bash
# Stop service
sudo systemctl stop memo-node

# Restart service
sudo systemctl restart memo-node

# Disable auto-start
sudo systemctl disable memo-node
```

## Performance Tips for Raspberry Pi 5

1. **Use base.en model** for best balance of speed and accuracy
2. **Set threads to 4-6** (Pi 5 has 4 performance cores)
3. **Ensure adequate cooling** - transcription is CPU-intensive
4. **Monitor system resources**:
```bash
# Watch CPU usage
htop

# Watch memory
free -h

# Watch temperature
vcgencmd measure_temp
```

## Next Steps

Once everything is working:

1. **Configure HTTPS endpoint** if you want to post transcriptions to a server
2. **Set up peer sync** with other memo-node instances (MacBook, etc.)
3. **Monitor transcriptions** using the `status` and `logs` commands
4. **Set up as a service** for automatic startup

## Quick Reference

```bash
# Start daemon
~/memo/memo-node/target/release/memo-node start

# Check status
~/memo/memo-node/target/release/memo-node status

# View logs
~/memo/memo-node/target/release/memo-node logs --limit 20

# With debug logging
RUST_LOG=debug ~/memo/memo-node/target/release/memo-node start

# Check configuration
cat ~/.config/memo-node/config.toml

# View database
sqlite3 ~/.memo/transcriptions.db "SELECT * FROM transcriptions ORDER BY timestamp DESC LIMIT 10;"
```

## Support

If you encounter issues:

1. Check logs: `RUST_LOG=debug memo-node start`
2. Verify config: `cat ~/.config/memo-node/config.toml`
3. Check status: `memo-node status`
4. Verify BLE: `hciconfig` and `sudo hcitool lescan`
5. Check system resources: `htop`, `free -h`
