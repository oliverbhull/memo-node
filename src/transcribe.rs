use anyhow::{Context, Result};
use memo_stt::SttEngine;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Whisper transcription using memo-stt
pub struct WhisperTranscriber {
    engine: Arc<tokio::sync::Mutex<SttEngine>>,
    audio_rx: mpsc::UnboundedReceiver<Vec<i16>>,
    transcription_tx: mpsc::UnboundedSender<String>,
}

impl WhisperTranscriber {
    pub fn new(
        model_name: &str,
        threads: u32,
        audio_rx: mpsc::UnboundedReceiver<Vec<i16>>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<String>)> {
        let (transcription_tx, transcription_rx) = mpsc::unbounded_channel();

        // Validate model name for Raspberry Pi (optimized for base.en and small.en)
        validate_model_for_pi(model_name)?;

        // Map config model names to memo-stt model paths
        let model_path = map_model_name_to_path(model_name)?;

        info!("Initializing Whisper engine with model: {} (configured for {} threads)", model_name, threads);
        info!("Model path: {:?}", model_path);
        // Note: Thread count configuration is available but memo-stt currently uses
        // a fixed thread count internally. This setting is reserved for future use
        // or if memo-stt adds thread configuration support.

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
            },
            transcription_rx,
        ))
    }

    pub async fn start(mut self) -> Result<()> {
        info!("Starting Whisper transcriber");

        // Buffer to accumulate audio samples
        let mut audio_buffer: Vec<i16> = Vec::new();
        let min_duration_samples = 16000 * 2; // 2 seconds at 16kHz

        while let Some(audio_chunk) = self.audio_rx.recv().await {
            debug!("Received audio chunk: {} samples", audio_chunk.len());

            audio_buffer.extend_from_slice(&audio_chunk);

            // Once we have enough audio, transcribe
            if audio_buffer.len() >= min_duration_samples {
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
        }

        Ok(())
    }

    async fn transcribe_audio(&self, audio: &[i16]) -> Result<String> {
        debug!("Transcribing {} samples", audio.len());

        // memo-stt expects i16 samples directly, no conversion needed
        // It handles normalization internally
        let engine = self.engine.lock().await;
        
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
