use anyhow::{Context, Result};
use audiopus::{coder::Decoder, Channels, SampleRate};

pub struct OpusDecoder {
    decoder: Decoder,
    sample_rate: u32,
    frame_size_samples: usize,
}

impl OpusDecoder {
    pub fn new(sample_rate: u32, channels: Channels) -> Result<Self> {
        if sample_rate != 16000 {
            anyhow::bail!("Opus decoder only supports 16kHz");
        }
        
        let frame_duration_ms = 20; // 20ms frames
        let frame_size_samples = (sample_rate * frame_duration_ms / 1000) as usize;
        
        // Create Opus decoder (mono, 16kHz)
        let decoder = Decoder::new(
            SampleRate::Hz16000,
            channels,
        ).context("Failed to create Opus decoder")?;

        Ok(Self {
            decoder,
            sample_rate,
            frame_size_samples,
        })
    }

    pub fn decode(&mut self, encoded: &[u8]) -> Result<Vec<i16>> {
        if encoded.is_empty() {
            return Ok(Vec::new());
        }

        // Memo device sends bundles: [bundle_index:1][num_frames:1][frame1_size:1][frame1_data:N]...
        // Skip bundle_index (first byte) and parse bundle
        if encoded.len() < 2 {
            tracing::debug!("Packet too short: {} bytes", encoded.len());
            return Ok(Vec::new()); // Not enough data for a bundle
        }

        let bundle_index = encoded[0];
        let bundle_data = &encoded[1..]; // Skip bundle_index
        
        if bundle_data.is_empty() {
            return Ok(Vec::new());
        }
        
        let num_frames = bundle_data[0] as usize;
        
        // Sanity check - reasonable number of frames
        if num_frames == 0 || num_frames > 10 {
            tracing::debug!("Invalid frame count: {} (bundle_index: {}, total_len: {})", 
                num_frames, bundle_index, encoded.len());
            return Ok(Vec::new());
        }
        
        let mut all_samples = Vec::new();
        let mut offset = 1; // Skip frame count byte

        // Decode each frame in the bundle
        for frame_idx in 0..num_frames {
            if offset >= bundle_data.len() {
                tracing::debug!("Bundle truncated at frame {} (offset: {}, len: {})", 
                    frame_idx, offset, bundle_data.len());
                break; // Bundle truncated
            }

            // Read frame size (1 byte)
            let frame_size = bundle_data[offset] as usize;
            offset += 1;

            if frame_size == 0 {
                tracing::debug!("Zero-size frame at index {}", frame_idx);
                continue; // Skip zero-size frames
            }

            if offset + frame_size > bundle_data.len() {
                tracing::debug!("Frame {} size {} exceeds bundle data (offset: {}, len: {})", 
                    frame_idx, frame_size, offset, bundle_data.len());
                break; // Frame size exceeds available data
            }

            // Extract frame data
            let frame_data = &bundle_data[offset..offset + frame_size];
            
            // Decode this frame using audiopus (same as memo-stt)
            let mut pcm = vec![0i16; self.frame_size_samples];
            
            match self.decoder.decode(Some(frame_data), &mut pcm, false) {
                Ok(samples_decoded) => {
                    if samples_decoded > 0 {
                        pcm.truncate(samples_decoded);
                        all_samples.extend_from_slice(&pcm);
                    }
                }
                Err(e) => {
                    // Only log occasionally to avoid spam
                    if frame_idx == 0 && num_frames > 0 {
                        tracing::debug!("Failed to decode Opus frame {} (size: {}): {}", 
                            frame_idx, frame_size, e);
                    }
                }
            }

            offset += frame_size;
        }

        Ok(all_samples)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opus_decoder_creation() {
        let decoder = OpusDecoder::new(16000, Channels::Mono);
        assert!(decoder.is_ok());
    }
    
    #[test]
    fn test_frame_size() {
        let decoder = OpusDecoder::new(16000, Channels::Mono).unwrap();
        // 20ms at 16kHz = 320 samples
        assert_eq!(decoder.frame_size_samples, 320);
    }
}
