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
        // Opus frames are typically 2.5, 5, 10, 20, 40 or 60 ms
        // For 16kHz, 20ms = 320 samples
        let frame_size = (self.sample_rate / 50) as usize; // 20ms frame
        let mut output = vec![0i16; frame_size * 2]; // Stereo

        let len = self
            .decoder
            .decode(encoded, &mut output, false)
            .context("Failed to decode Opus frame")?;

        output.truncate(len);
        Ok(output)
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
