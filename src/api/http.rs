use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// HTTP client for posting transcriptions to HTTPS endpoint
pub struct HttpClient {
    client: Client,
    endpoint: String,
}

impl HttpClient {
    /// Create a new HTTP client with the specified endpoint
    pub fn new(endpoint: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { client, endpoint })
    }

    /// Post a transcription to the configured HTTPS endpoint
    /// 
    /// Uses exponential backoff retry logic:
    /// - First retry: 1 second
    /// - Second retry: 2 seconds
    /// - Third retry: 4 seconds
    /// - Max 3 retries
    pub async fn post_transcription(
        &self,
        id: &str,
        timestamp: i64,
        text: &str,
        source_node: &str,
        memo_device_id: Option<&str>,
    ) -> Result<()> {
        let payload = json!({
            "id": id,
            "timestamp": timestamp,
            "text": text,
            "source_node": source_node,
            "memo_device_id": memo_device_id,
        });

        let mut retry_count = 0;
        const MAX_RETRIES: u32 = 3;

        loop {
            match self
                .client
                .post(&self.endpoint)
                .json(&payload)
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        debug!(
                            "Successfully posted transcription {} to {}",
                            id, self.endpoint
                        );
                        return Ok(());
                    } else {
                        let status = response.status();
                        let error_text = response
                            .text()
                            .await
                            .unwrap_or_else(|_| "Unknown error".to_string());
                        
                        if retry_count < MAX_RETRIES {
                            retry_count += 1;
                            let delay = Duration::from_secs(2_u64.pow(retry_count - 1));
                            warn!(
                                "HTTP POST failed with status {}: {}. Retrying in {:?} (attempt {}/{})",
                                status, error_text, delay, retry_count, MAX_RETRIES
                            );
                            sleep(delay).await;
                            continue;
                        } else {
                            return Err(anyhow::anyhow!(
                                "HTTP POST failed after {} retries: status {} - {}",
                                MAX_RETRIES,
                                status,
                                error_text
                            ));
                        }
                    }
                }
                Err(e) => {
                    if retry_count < MAX_RETRIES {
                        retry_count += 1;
                        let delay = Duration::from_secs(2_u64.pow(retry_count - 1));
                        warn!(
                            "HTTP POST error: {}. Retrying in {:?} (attempt {}/{})",
                            e, delay, retry_count, MAX_RETRIES
                        );
                        sleep(delay).await;
                        continue;
                    } else {
                        return Err(anyhow::anyhow!(
                            "HTTP POST failed after {} retries: {}",
                            MAX_RETRIES,
                            e
                        ));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_client_creation() {
        // This will fail at runtime if endpoint is invalid, but we can test creation
        let client = HttpClient::new("https://example.com/api".to_string());
        assert!(client.is_ok());
    }
}
