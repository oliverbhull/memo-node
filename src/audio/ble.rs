use anyhow::{Context, Result};
use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, ScanFilter,
};
use btleplug::platform::{Manager, Peripheral};
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

pub struct BleAudioReceiver {
    service_uuid: Uuid,
    characteristic_uuid: Uuid,
    audio_tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl BleAudioReceiver {
    pub fn new(
        service_uuid: Uuid,
        characteristic_uuid: Uuid,
    ) -> (Self, mpsc::UnboundedReceiver<Vec<u8>>) {
        let (audio_tx, audio_rx) = mpsc::unbounded_channel();

        (
            Self {
                service_uuid,
                characteristic_uuid,
                audio_tx,
            },
            audio_rx,
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

        info!("Found Memo device: {}", local_name);

        // Connect to the device
        if !peripheral.is_connected().await? {
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

        // Find our characteristic
        let characteristics = peripheral.characteristics();
        let audio_char = characteristics
            .iter()
            .find(|c| c.uuid == self.characteristic_uuid)
            .context("Audio characteristic not found")?;

        info!("Found audio characteristic on {}", local_name);

        // Subscribe to notifications
        self.subscribe_to_audio(peripheral, audio_char, &local_name)
            .await?;

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
}
