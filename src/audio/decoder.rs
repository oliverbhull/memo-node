use anyhow::{Context, Result};
use opus::{Channels, Decoder};

pub struct OpusDecoder {
    decoder: Decoder,
    sample_rate: u32,
}

impl OpusDecoder {
    pub fn new(sample_rate: u32, channels: Channels) -> Result<Self> {
        let decoder =
            Decoder::new(sample_rate, channels).context("Failed to create Opus decoder")?;

        Ok(Self {
            decoder,
            sample_rate,
        })
    }

    pub fn decode(&mut self, encoded: &[u8]) -> Result<Vec<i16>> {
        if encoded.is_empty() {
            return Ok(Vec::new());
        }

        // Memo device sends bundles: [bundle_index:1][num_frames:1][frame1_size:1][frame1_data:N]...
        // Skip bundle_index (first byte) and parse bundle
        if encoded.len() < 2 {
            return Ok(Vec::new()); // Not enough data for a bundle
        }

        let bundle_data = &encoded[1..]; // Skip bundle_index
        let num_frames = bundle_data[0] as usize;
        
        let mut all_samples = Vec::new();
        let mut offset = 1; // Skip frame count byte

        // Decode each frame in the bundle
        for _ in 0..num_frames {
            if offset >= bundle_data.len() {
                break; // Bundle truncated
            }

            // Read frame size (1 byte)
            let frame_size = bundle_data[offset] as usize;
            offset += 1;

            if offset + frame_size > bundle_data.len() {
                break; // Frame size exceeds available data
            }

            // Extract frame data
            let frame_data = &bundle_data[offset..offset + frame_size];
            
            // Decode this frame
            // Opus frames are typically 2.5, 5, 10, 20, 40 or 60 ms
            // For 16kHz, 20ms = 320 samples
            let frame_size_samples = (self.sample_rate / 50) as usize; // 20ms frame
            let mut output = vec![0i16; frame_size_samples];

            match self.decoder.decode(frame_data, &mut output, false) {
                Ok(len) => {
                    output.truncate(len);
                    all_samples.extend_from_slice(&output);
                }
                Err(e) => {
                    // Log but continue - some frames may fail
                    tracing::debug!("Failed to decode Opus frame: {}", e);
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
}
