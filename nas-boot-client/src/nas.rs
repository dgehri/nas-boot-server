use anyhow::Result;
use chrono::Local;
use log::{error, info, warn};
use std::time::Duration;

use crate::config::Config;

pub async fn send_heartbeat(config: &Config) -> Result<bool> {
    let client = reqwest::Client::new();
    let timestamp = Local::now().to_rfc3339();
    let hostname = hostname::get()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    info!("Sending heartbeat from {hostname}");

    match client
        .post(&config.heartbeat_url)
        .json(&serde_json::json!({
            "timestamp": timestamp,
            "hostname": hostname
        }))
        .timeout(Duration::from_secs(config.heartbeat_timeout_secs))
        .send()
        .await
    {
        Ok(response) => {
            if response.status().is_success() {
                info!("Heartbeat sent successfully to {}", config.heartbeat_url);
                Ok(true)
            } else {
                error!("Heartbeat failed with status: {}", response.status());
                Ok(false)
            }
        }
        Err(e) => {
            warn!("Failed to send heartbeat: {e}");
            Err(anyhow::anyhow!("Failed to send heartbeat: {}", e))
        }
    }
}
