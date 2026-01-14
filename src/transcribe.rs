use anyhow::{Context, Result};
use memo_stt::SttEngine;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Whisper transcription using memo-stt
pub struct WhisperTranscriber {
    engine: Arc<tokio::sync::Mutex<SttEngine>>,
    audio_rx: mpsc::UnboundedReceiver<Vec<i16>>,
    transcription_tx: mpsc::UnboundedSender<String>,
    is_recording: Arc<AtomicBool>,
}

impl WhisperTranscriber {
    pub fn new(
        model_name: &str,
        threads: u8,
        audio_rx: mpsc::UnboundedReceiver<Vec<i16>>,
        is_recording: Arc<AtomicBool>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<String>)> {
        let (transcription_tx, transcription_rx) = mpsc::unbounded_channel();

        // Validate model name for Raspberry Pi (optimized for base.en and small.en)
        validate_model_for_pi(model_name)?;

        // Map config model names to memo-stt model paths
        let model_path = map_model_name_to_path(model_name)?;

        info!("Initializing Whisper engine with model: {} (configured for {} threads)", model_name, threads);
        info!("Model path: {:?}", model_path);
        // Note: Thread count is optimized automatically by memo-stt based on CPU cores
        // The configured thread count is logged for reference but memo-stt will use
        // optimal thread count (min of CPU cores or 8) for best performance

        // Create memo-stt engine
        // memo-stt handles model downloading automatically
        let engine = SttEngine::new(&model_path, 16000)
            .context("Failed to create Whisper engine")?;

        // Warm up the engine to reduce first-transcription latency
        engine.warmup()
            .context("Failed to warm up Whisper engine")?;

        info!("Whisper engine initialized and warmed up");

        Ok((
            Self {
                engine: Arc::new(tokio::sync::Mutex::new(engine)),
                audio_rx,
                transcription_tx,
                is_recording,
            },
            transcription_rx,
        ))
    }

    pub async fn start(mut self) -> Result<()> {
        info!("Starting Whisper transcriber");

        // Buffer to accumulate audio samples for the full recording
        let mut audio_buffer: Vec<i16> = Vec::new();
        let mut was_recording = self.is_recording.load(Ordering::Acquire);

        loop {
            // Receive audio chunks (with timeout to allow periodic recording state checks)
            tokio::select! {
                audio_chunk = self.audio_rx.recv() => {
                    match audio_chunk {
                        Some(chunk) => {
                            let is_recording_now = self.is_recording.load(Ordering::Acquire);
                            
                            // If recording just stopped, transcribe the accumulated audio
                            if was_recording && !is_recording_now && !audio_buffer.is_empty() {
                                info!("Recording stopped, transcribing {} samples", audio_buffer.len());
                                
                                match self.transcribe_audio(&audio_buffer).await {
                                    Ok(text) => {
                                        if !text.trim().is_empty() {
                                            info!("Transcribed: {}", text);
                                            if let Err(e) = self.transcription_tx.send(text) {
                                                error!("Failed to send transcription: {}", e);
                                            }
                                        } else {
                                            debug!("Transcription returned empty text");
                                        }
                                    }
                                    Err(e) => {
                                        error!("Transcription failed: {}", e);
                                    }
                                }

                                // Clear buffer after transcription
                                audio_buffer.clear();
                            }

                            // Only accumulate audio while recording
                            if is_recording_now {
                                debug!("Received audio chunk: {} samples", chunk.len());
                                audio_buffer.extend_from_slice(&chunk);
                            }
                            
                            was_recording = is_recording_now;
                        }
                        None => {
                            // Channel closed, check if we need to transcribe final buffer
                            let is_recording_now = self.is_recording.load(Ordering::Acquire);
                            if was_recording && !is_recording_now && !audio_buffer.is_empty() {
                                info!("Channel closed, transcribing final {} samples", audio_buffer.len());
                                
                                match self.transcribe_audio(&audio_buffer).await {
                                    Ok(text) => {
                                        if !text.trim().is_empty() {
                                            info!("Transcribed: {}", text);
                                            if let Err(e) = self.transcription_tx.send(text) {
                                                error!("Failed to send transcription: {}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("Transcription failed: {}", e);
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                    // Periodic check for recording state changes
                    let is_recording_now = self.is_recording.load(Ordering::Acquire);
                    
                    // If recording just stopped, transcribe the accumulated audio
                    if was_recording && !is_recording_now && !audio_buffer.is_empty() {
                        info!("Recording stopped (periodic check), transcribing {} samples", audio_buffer.len());
                        
                        match self.transcribe_audio(&audio_buffer).await {
                            Ok(text) => {
                                if !text.trim().is_empty() {
                                    info!("Transcribed: {}", text);
                                    if let Err(e) = self.transcription_tx.send(text) {
                                        error!("Failed to send transcription: {}", e);
                                    }
                                } else {
                                    debug!("Transcription returned empty text");
                                }
                            }
                            Err(e) => {
                                error!("Transcription failed: {}", e);
                            }
                        }

                        // Clear buffer after transcription
                        audio_buffer.clear();
                    }
                    
                    was_recording = is_recording_now;
                }
            }
        }

        Ok(())
    }

    async fn transcribe_audio(&self, audio: &[i16]) -> Result<String> {
        debug!("Transcribing {} samples", audio.len());

        // memo-stt expects i16 samples directly, no conversion needed
        // It handles normalization internally
        let mut engine = self.engine.lock().await;
        
        engine.transcribe(audio)
            .map_err(|e| anyhow::anyhow!("Transcription error: {}", e))
    }
}

/// Validate model name for Raspberry Pi optimization
/// 
/// Recommends base.en or small.en for Pi hardware, but allows other models
/// with a warning. Full model filenames (containing .bin) are always allowed.
fn validate_model_for_pi(model_name: &str) -> Result<()> {
    // Allow full model filenames
    if model_name.contains(".bin") {
        // Warn if not a recommended model for Pi
        if !model_name.contains("base") && !model_name.contains("small") && !model_name.contains("tiny") {
            warn!(
                "Model '{}' may be too large/slow for Raspberry Pi. Recommended: base.en or small.en",
                model_name
            );
        }
        return Ok(());
    }

    // For simple model names, validate
    match model_name {
        "base.en" | "small.en" | "tiny.en" => Ok(()),
        _ => {
            warn!(
                "Model '{}' not optimized for Raspberry Pi. Recommended: base.en or small.en",
                model_name
            );
            Ok(()) // Allow but warn
        }
    }
}

/// Map config model names to actual model file paths
/// 
/// Converts simple names like "base.en" to full model file paths
/// that memo-stt can use. Models will be auto-downloaded if needed.
fn map_model_name_to_path(model_name: &str) -> Result<String> {
    // Map config model names to actual Whisper model file names
    let model_file = match model_name {
        "base.en" => "ggml-base.en.bin",
        "small.en" => "ggml-small.en-q5_1.bin", // Default model
        "tiny.en" => "ggml-tiny.en.bin",
        // If it's already a full model name, use it as-is
        name if name.contains(".bin") => name,
        // Otherwise, assume it's a model name and add prefix
        name => {
            warn!("Unknown model name '{}', using as-is. Expected: base.en, small.en, or full model filename", name);
            if name.ends_with(".bin") {
                name
            } else {
                return Err(anyhow::anyhow!(
                    "Invalid model name: {}. Use 'base.en', 'small.en', or a full model filename",
                    name
                ));
            }
        }
    };

    Ok(model_file.to_string())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_name_mapping() {
        assert_eq!(map_model_name_to_path("base.en").unwrap(), "ggml-base.en.bin");
        assert_eq!(map_model_name_to_path("small.en").unwrap(), "ggml-small.en-q5_1.bin");
    }
}
