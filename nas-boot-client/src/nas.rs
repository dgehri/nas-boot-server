use anyhow::Result;
use chrono::Local;
use log::{error, info, warn};
use std::time::Duration;
use tokio::time::timeout;

use crate::config::Config;

// Reuse HTTP client to avoid connection overhead
static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

fn get_client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(3)) // Shorter default timeout
            .connect_timeout(Duration::from_secs(2)) // Connection timeout
            .tcp_keepalive(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

pub async fn send_heartbeat(config: &Config) -> Result<bool> {
    let client = get_client();
    let timestamp = Local::now().to_rfc3339();
    let hostname = hostname::get()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    info!("Sending heartbeat from {hostname}");

    // Add an additional timeout wrapper to prevent hanging
    let heartbeat_future = client
        .post(&config.heartbeat_url)
        .json(&serde_json::json!({
            "timestamp": timestamp,
            "hostname": hostname
        }))
        .timeout(Duration::from_secs(config.heartbeat_timeout_secs))
        .send();

    // Wrap with tokio timeout for extra safety
    match timeout(Duration::from_secs(config.heartbeat_timeout_secs + 1), heartbeat_future).await {
        Ok(Ok(response)) => {
            if response.status().is_success() {
                info!("Heartbeat sent successfully to {}", config.heartbeat_url);
                Ok(true)
            } else {
                error!("Heartbeat failed with status: {}", response.status());
                Ok(false)
            }
        }
        Ok(Err(e)) => {
            warn!("Failed to send heartbeat: {e}");
            Ok(false) // Don't return error, just indicate failure
        }
        Err(_) => {
            warn!("Heartbeat timed out after {}s", config.heartbeat_timeout_secs + 1);
            Ok(false) // Timeout - don't error, just indicate failure
        }
    }
}
