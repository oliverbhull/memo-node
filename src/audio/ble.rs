use anyhow::{Context, Result};
use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Manager, Peripheral};
use futures_util::StreamExt;
use std::collections::HashSet;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

// Control characteristic UUIDs (from memo-stt)
const CONTROL_TX_UUID: &str = "1234A003-1234-5678-1234-56789ABCDEF0";
const CONTROL_RX_UUID: &str = "1234A002-1234-5678-1234-56789ABCDEF0";

// Control response values from device
const RESP_SPEECH_START: u8 = 0x01;  // Button pressed - start recording
const RESP_SPEECH_END: u8 = 0x02;    // Button pressed again - stop recording

// Control commands to device
const CMD_START_RECORDING: u8 = 10;
const CMD_END_RECORDING: u8 = 12;

pub struct BleAudioReceiver {
    service_uuid: Uuid,
    characteristic_uuid: Uuid,
    audio_tx: mpsc::UnboundedSender<Vec<u8>>,
    is_recording: Arc<AtomicBool>,
    connected_devices: Arc<Mutex<HashSet<String>>>, // Track connected device names
}

impl BleAudioReceiver {
    pub fn new(
        service_uuid: Uuid,
        characteristic_uuid: Uuid,
    ) -> (Self, mpsc::UnboundedReceiver<Vec<u8>>, Arc<AtomicBool>) {
        let (audio_tx, audio_rx) = mpsc::unbounded_channel();
        let is_recording = Arc::new(AtomicBool::new(true)); // Start recording by default

        (
            Self {
                service_uuid,
                characteristic_uuid,
                audio_tx,
                is_recording: is_recording.clone(),
                connected_devices: Arc::new(Mutex::new(HashSet::new())),
            },
            audio_rx,
            is_recording,
        )
    }

    pub async fn start(self: Arc<Self>) -> Result<()> {
        info!("Starting BLE audio receiver");

        let manager = Manager::new()
            .await
            .context("Failed to create BLE manager")?;

        let adapters = manager.adapters().await.context("Failed to get BLE adapters")?;
        let adapter = adapters
            .into_iter()
            .next()
            .context("No BLE adapters found")?;

        info!("Using BLE adapter: {}", adapter.adapter_info().await?);

        // Start scanning
        adapter
            .start_scan(ScanFilter::default())
            .await
            .context("Failed to start BLE scan")?;

        info!(
            "Scanning for Memo devices with service UUID {}",
            self.service_uuid
        );

        // Keep scanning and connecting to devices
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            let peripherals = adapter
                .peripherals()
                .await
                .context("Failed to get peripherals")?;

            for peripheral in peripherals {
                if let Err(e) = self.try_connect_device(&peripheral).await {
                    debug!("Failed to connect to device: {}", e);
                }
            }
        }
    }

    async fn try_connect_device(&self, peripheral: &Peripheral) -> Result<()> {
        let properties = peripheral.properties().await?.context("No properties")?;

        let local_name = properties.local_name.unwrap_or_default();

        // Check if this device has our service
        if !properties.services.contains(&self.service_uuid) {
            return Ok(());
        }

        // Check if we're already connected and set up for this device
        {
            let mut connected = self.connected_devices.lock().unwrap();
            if connected.contains(&local_name) {
                // Already connected and set up, skip
                return Ok(());
            }
        }

        info!("Found Memo device: {}", local_name);

        // Connect to the device
        let was_connected = peripheral.is_connected().await?;
        if !was_connected {
            peripheral
                .connect()
                .await
                .context("Failed to connect to device")?;
            info!("Connected to {}", local_name);
        }

        // Discover services
        peripheral
            .discover_services()
            .await
            .context("Failed to discover services")?;

        // Find characteristics
        let characteristics = peripheral.characteristics();
        let audio_char = characteristics
            .iter()
            .find(|c| c.uuid == self.characteristic_uuid)
            .context("Audio characteristic not found")?;

        let control_tx_uuid = Uuid::parse_str(CONTROL_TX_UUID)
            .context("Failed to parse control TX UUID")?;
        let control_rx_uuid = Uuid::parse_str(CONTROL_RX_UUID)
            .context("Failed to parse control RX UUID")?;

        let control_tx_char = characteristics
            .iter()
            .find(|c| c.uuid == control_tx_uuid);
        let control_rx_char = characteristics
            .iter()
            .find(|c| c.uuid == control_rx_uuid);

        info!("Found audio characteristic on {}", local_name);
        if control_tx_char.is_some() {
            info!("Found control TX characteristic on {}", local_name);
        }
        if control_rx_char.is_some() {
            info!("Found control RX characteristic on {}", local_name);
        }

        // Subscribe to audio notifications
        self.subscribe_to_audio(&peripheral, audio_char, &local_name)
            .await?;

        // Subscribe to control TX notifications (button events)
        if let Some(control_tx) = control_tx_char {
            self.subscribe_to_control(peripheral.clone(), control_tx, &local_name)
                .await?;
        }

        // Send START command to begin recording (if control RX is available)
        if let Some(control_rx) = control_rx_char {
            info!("Sending START_RECORDING command to {}", local_name);
            let start_cmd = vec![CMD_START_RECORDING];
            if let Err(e) = peripheral.write(control_rx, &start_cmd, WriteType::WithoutResponse).await {
                warn!("Failed to send START command: {}", e);
            } else {
                info!("START_RECORDING command sent to {}", local_name);
                self.is_recording.store(true, Ordering::Release);
            }
        }

        // Mark this device as connected and set up
        {
            let mut connected = self.connected_devices.lock().unwrap();
            connected.insert(local_name.clone());
        }

        Ok(())
    }

    async fn subscribe_to_audio(
        &self,
        peripheral: &Peripheral,
        characteristic: &Characteristic,
        device_name: &str,
    ) -> Result<()> {
        peripheral
            .subscribe(characteristic)
            .await
            .context("Failed to subscribe to characteristic")?;

        info!("Subscribed to audio from {}", device_name);

        let audio_tx = self.audio_tx.clone();
        let peripheral = peripheral.clone();
        let characteristic = characteristic.clone();
        let device_name = device_name.to_string();

        tokio::spawn(async move {
            let mut notification_stream = peripheral.notifications().await.unwrap();

            while let Some(data) = notification_stream.next().await {
                if data.uuid == characteristic.uuid {
                    debug!("Received {} bytes of audio data", data.value.len());

                    if let Err(e) = audio_tx.send(data.value) {
                        error!("Failed to send audio data: {}", e);
                        break;
                    }
                }
            }

            warn!("Audio notification stream ended for {}", device_name);
        });

        Ok(())
    }

    async fn subscribe_to_control(
        &self,
        peripheral: Peripheral,
        characteristic: &Characteristic,
        device_name: &str,
    ) -> Result<()> {
        peripheral
            .subscribe(characteristic)
            .await
            .context("Failed to subscribe to control characteristic")?;

        info!("Subscribed to control events from {}", device_name);

        let is_recording = self.is_recording.clone();
        let peripheral_clone = peripheral.clone();
        let characteristic_uuid = characteristic.uuid;
        let device_name = device_name.to_string();

        tokio::spawn(async move {
            let mut notification_stream = match peripheral_clone.notifications().await {
                Ok(stream) => stream,
                Err(e) => {
                    error!("Failed to get notification stream for control: {}", e);
                    return;
                }
            };

            // Track last control value to avoid duplicate processing
            let mut last_control_value: Option<u8> = None;
            
            while let Some(data) = notification_stream.next().await {
                if data.uuid == characteristic_uuid && !data.value.is_empty() {
                    let control_value = data.value[0];
                    
                    // Skip if we just processed this value (debounce duplicates)
                    if last_control_value == Some(control_value) {
                        continue;
                    }
                    last_control_value = Some(control_value);
                    
                    match control_value {
                        RESP_SPEECH_START => {
                            let current = is_recording.load(Ordering::Acquire);
                            if !current {
                                info!("Button pressed - starting recording on {}", device_name);
                                is_recording.store(true, Ordering::Release);
                            }
                        }
                        RESP_SPEECH_END => {
                            let current = is_recording.load(Ordering::Acquire);
                            if current {
                                info!("Button pressed again - stopping recording on {}", device_name);
                                is_recording.store(false, Ordering::Release);
                            }
                        }
                        _ => {
                            debug!("Received control event: 0x{:02X} from {}", control_value, device_name);
                        }
                    }
                }
            }

            warn!("Control notification stream ended for {}", device_name);
        });

        Ok(())
    }
}
