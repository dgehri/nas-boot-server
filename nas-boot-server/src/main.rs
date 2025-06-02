use anyhow::{Context, Result};
use axum::{routing::post, Json, Router};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use log::{debug, error, info, Level, Log, Metadata, Record};
use multi_log::MultiLogger;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time;
use yaml_rust2::YamlLoader;

// Custom QNAP Logger
pub struct QnapLogger;

impl Log for QnapLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let level_code = match record.level() {
                Level::Error => "2",
                Level::Warn => "1",
                Level::Info | Level::Debug | Level::Trace => "0",
            };

            let message = format!("[NAS Boot Server] {}", record.args());

            // Execute log_tool command
            let _ = Command::new("/sbin/log_tool")
                .arg("-a")
                .arg(&message)
                .arg("-t")
                .arg(level_code)
                .output();
        }
    }

    fn flush(&self) {
        // QNAP log_tool doesn't need flushing
    }
}

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
#[cfg(windows)]
use std::os::windows::process::ExitStatusExt;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate default configuration file
    GenerateConfig,
    /// Run the server
    Run,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    bind_address: String,
    shutdown_delay_mins: i64,
    keepalive_file: String,
    backup_process_pattern: String,
    heartbeat_timeout_mins: i64,
    check_interval_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0:8090".to_string(),
            shutdown_delay_mins: 10,
            keepalive_file: "/share/Public/keepalive.txt".to_string(),
            backup_process_pattern:
                "python /share/CACHEDEV1_DATA/.qpkg/AzureStorage/bin/engine.pyc backup".to_string(),
            heartbeat_timeout_mins: 2,
            check_interval_secs: 60,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Heartbeat {
    timestamp: String,
    hostname: String,
}

#[derive(Clone)]
struct AppState {
    clients: Arc<Mutex<HashMap<String, DateTime<Utc>>>>,
    config: Arc<Config>,
}

fn get_config_path() -> PathBuf {
    PathBuf::from("/share/CACHEDEV1_DATA/.config/nas-boot/nas-boot-server-config.yaml")
}

fn load_config() -> Result<Config> {
    let config_path = get_config_path();

    if !config_path.exists() {
        return Err(anyhow::anyhow!(
            "Configuration file not found at: {}. Run with 'generate-config' to create it.",
            config_path.display()
        ));
    }

    let config_str = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config from {}", config_path.display()))?;

    let docs = YamlLoader::load_from_str(&config_str).context("Failed to parse YAML")?;

    if docs.is_empty() {
        return Err(anyhow::anyhow!("Empty configuration file"));
    }

    let doc = &docs[0];

    let config = Config {
        bind_address: doc["bind_address"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing bind_address"))?
            .to_string(),
        shutdown_delay_mins: doc["shutdown_delay_mins"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("Missing shutdown_delay_mins"))?,
        keepalive_file: doc["keepalive_file"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing keepalive_file"))?
            .to_string(),
        backup_process_pattern: doc["backup_process_pattern"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing backup_process_pattern"))?
            .to_string(),
        heartbeat_timeout_mins: doc["heartbeat_timeout_mins"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("Missing heartbeat_timeout_mins"))?,
        check_interval_secs: doc["check_interval_secs"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("Missing check_interval_secs"))?
            as u64,
    };

    Ok(config)
}

fn generate_config() -> Result<()> {
    let config_path = get_config_path();

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    let default_config = Config::default();

    // Create YAML manually
    let yaml_content = format!(
        r#"bind_address: "{}"
shutdown_delay_mins: {}
keepalive_file: "{}"
backup_process_pattern: "{}"
heartbeat_timeout_mins: {}
check_interval_secs: {}
"#,
        default_config.bind_address,
        default_config.shutdown_delay_mins,
        default_config.keepalive_file,
        default_config.backup_process_pattern,
        default_config.heartbeat_timeout_mins,
        default_config.check_interval_secs
    );

    fs::write(&config_path, yaml_content)
        .with_context(|| format!("Failed to write config to {}", config_path.display()))?;

    println!(
        "Generated default configuration at: {}",
        config_path.display()
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Create console logger
    let console_logger = env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Debug)
        .build();

    // Create QNAP logger
    let qnap_logger = QnapLogger;

    let mut loggers: Vec<Box<dyn Log>> = vec![];
    loggers.push(Box::new(console_logger));
    loggers.push(Box::new(qnap_logger));

    // Combine both loggers
    MultiLogger::init(loggers, log::Level::Debug)?;

    let cli = Cli::parse();

    let result = match cli.command {
        Some(Commands::GenerateConfig) => generate_config(),
        Some(Commands::Run) | None => run_server().await,
    };

    match result {
        Ok(_) => info!("Operation completed successfully"),
        Err(e) => error!("Operation failed: {}", e),
    }

    Ok(())
}

async fn run_server() -> Result<()> {
    info!("NAS Boot Server starting up");

    let config = load_config()?;

    let state = AppState {
        clients: Arc::new(Mutex::new(HashMap::new())),
        config: Arc::new(config.clone()),
    };

    // Start shutdown monitor
    let monitor_state = state.clone();
    tokio::spawn(async move {
        shutdown_monitor(monitor_state).await;
    });

    // Start web server
    let app = Router::new()
        .route("/heartbeat", post(handle_heartbeat))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind_address)
        .await
        .with_context(|| format!("Failed to bind to {}", config.bind_address))?;

    info!("NAS Boot Server listening on {}", config.bind_address);
    axum::serve(listener, app).await?;

    Ok(())
}

async fn handle_heartbeat(
    state: axum::extract::State<AppState>,
    Json(heartbeat): Json<Heartbeat>,
) -> &'static str {
    let mut clients = state.clients.lock().await;

    match DateTime::parse_from_rfc3339(&heartbeat.timestamp) {
        Ok(dt) => {
            let hostname = heartbeat.hostname.clone();
            clients.insert(heartbeat.hostname, dt.with_timezone(&Utc));
            debug!("Heartbeat from {}", hostname);
        }
        Err(e) => error!("Invalid timestamp: {}", e),
    }

    "OK"
}

async fn shutdown_monitor(state: AppState) {
    let mut interval = time::interval(Duration::from_secs(state.config.check_interval_secs));
    let mut shutdown_timer: Option<DateTime<Utc>> = None;

    loop {
        interval.tick().await;

        let now = Utc::now();
        let mut active_clients = false;

        {
            let mut clients = state.clients.lock().await;

            clients.retain(|hostname, last_seen| {
                let age = now.signed_duration_since(*last_seen);
                if age.num_minutes() < state.config.heartbeat_timeout_mins {
                    active_clients = true;
                    true
                } else {
                    info!("Client {} timed out", hostname);
                    false
                }
            });
        }

        if active_clients {
            if shutdown_timer.is_some() {
                info!("Active clients detected, cancelling shutdown timer");
                shutdown_timer = None;
            }
        } else {
            match shutdown_timer {
                None => {
                    info!("No active clients, starting shutdown timer");
                    shutdown_timer = Some(now);
                }
                Some(timer_start) => {
                    let elapsed = now.signed_duration_since(timer_start);
                    if elapsed.num_minutes() >= state.config.shutdown_delay_mins {
                        if should_shutdown(&state.config) {
                            info!("Shutdown timer expired, initiating shutdown");
                            initiate_shutdown();
                            break;
                        }
                        shutdown_timer = None;
                    }
                }
            }
        }
    }
}

fn should_shutdown(config: &Config) -> bool {
    // Check keepalive file
    if Path::new(&config.keepalive_file).exists() {
        info!("Keepalive file exists, not shutting down");
        return false;
    }

    // Check for backup process
    let output = Command::new("ps").arg("aux").output().unwrap_or_else(|_| {
        error!("Failed to execute ps command");
        std::process::Output {
            stdout: Vec::new(),
            stderr: Vec::new(),
            status: std::process::ExitStatus::from_raw(1),
        }
    });

    if String::from_utf8_lossy(&output.stdout).contains(&config.backup_process_pattern) {
        info!("Backup process running, not shutting down");
        return false;
    }

    true
}

fn initiate_shutdown() {
    info!("Initiating system shutdown");

    match Command::new("/sbin/poweroff").spawn() {
        Ok(_) => info!("Shutdown command issued"),
        Err(e) => error!("Failed to issue shutdown command: {}", e),
    }
}
