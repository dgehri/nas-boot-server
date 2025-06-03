use anyhow::Result;
use chrono::Local;
use log::{error, info, warn};
use std::time::Duration;
use wake_on_lan::MagicPacket;

use crate::config::Config;

pub async fn wake_nas(config: &Config) {
    // Parse MAC address
    let mac_parts: Vec<u8> = config
        .nas_mac
        .split(':')
        .filter_map(|s| u8::from_str_radix(s, 16).ok())
        .collect();

    if mac_parts.len() == 6 {
        let mac: [u8; 6] = [
            mac_parts[0],
            mac_parts[1],
            mac_parts[2],
            mac_parts[3],
            mac_parts[4],
            mac_parts[5],
        ];

        match MagicPacket::new(&mac).send() {
            Ok(()) => {
                info!("Sent WOL packet to NAS");
            }
            Err(e) => warn!("Failed to send WOL packet: {e}"),
        }

        // Send via router
        let router_addr = format!("{}:9", config.router_ip);
        match MagicPacket::new(&mac).send_to(&router_addr as &str, "0.0.0.0:0") {
            Ok(()) => info!("Sent WOL packet via router"),
            Err(e) => warn!("Failed to send WOL packet via router: {e}"),
        }
    } else {
        error!("Invalid MAC address format");
    }
}

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
