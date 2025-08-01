use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::wake_mode::WakeMode;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub nas_mac: String,
    pub nas_ip: String,
    pub router_ip: String,
    pub heartbeat_url: String,
    pub check_interval_secs: u64,
    pub idle_threshold_mins: u32,
    pub heartbeat_timeout_secs: u64,
    #[serde(default)]
    pub wake_mode: WakeMode,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            nas_mac: "00:08:9B:DB:EF:9A".to_string(),
            nas_ip: "192.168.42.2".to_string(),
            router_ip: "192.168.42.1".to_string(),
            heartbeat_url: "http://192.168.42.2:8090/heartbeat".to_string(),
            check_interval_secs: 60, // Increased from 30 to 60 seconds to reduce CPU usage
            idle_threshold_mins: 5,
            heartbeat_timeout_secs: 5,
            wake_mode: WakeMode::default(),
        }
    }
}

pub fn get_config_path() -> PathBuf {
    // Use system-wide config location instead of user home directory
    let program_data_dir =
        std::env::var("ProgramData").unwrap_or_else(|_| String::from("C:\\ProgramData"));
    let mut path = PathBuf::from(program_data_dir);
    path.push("NASBootClient");
    path.push("nas-boot-client-config.yaml");
    path
}

pub fn load_config() -> Result<Config> {
    let config_path = get_config_path();

    if !config_path.exists() {
        return Err(anyhow::anyhow!(
            "Configuration file not found at: {}. Run with 'generate-config' to create it.",
            config_path.display()
        ));
    }

    let config: Config = serde_yaml::from_reader(
        &fs::File::open(&config_path)
            .with_context(|| format!("Failed to open config from {}", config_path.display()))?,
    )
    .with_context(|| format!("Failed to parse config from {}", config_path.display()))?;

    Ok(config)
}

pub fn save_config(config: &Config) -> Result<()> {
    let config_path = get_config_path();

    let yaml_content = serde_yaml::to_string(config).with_context(|| {
        format!(
            "Failed to serialize config to YAML for {}",
            config_path.display()
        )
    })?;

    fs::write(&config_path, yaml_content)
        .with_context(|| format!("Failed to write config to {}", config_path.display()))?;

    println!("Configuration saved to: {}", config_path.display());
    Ok(())
}

pub fn generate_config() -> Result<()> {
    let config_path = get_config_path();

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    let default_config = Config::default();

    // Create YAML manually
    let yaml_content = format!(
        r#"nas_mac: "{}"
nas_ip: "{}"
router_ip: "{}"
heartbeat_url: "{}"
check_interval_secs: {}
idle_threshold_mins: {}
heartbeat_timeout_secs: {}
"#,
        default_config.nas_mac,
        default_config.nas_ip,
        default_config.router_ip,
        default_config.heartbeat_url,
        default_config.check_interval_secs,
        default_config.idle_threshold_mins,
        default_config.heartbeat_timeout_secs
    );

    fs::write(&config_path, yaml_content)
        .with_context(|| format!("Failed to write config to {}", config_path.display()))?;

    println!(
        "Generated default configuration at: {}",
        config_path.display()
    );
    Ok(())
}
