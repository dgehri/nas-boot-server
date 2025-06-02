use anyhow::{Context, Result};
use chrono::Local;
use clap::{Parser, Subcommand};
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{self, File};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use wake_on_lan::MagicPacket;
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use yaml_rust2::YamlLoader;

const SERVICE_NAME: &str = "NASBootClient";
const SERVICE_DISPLAY_NAME: &str = "NAS Boot Client";
const SERVICE_DESCRIPTION: &str = "Monitors user activity and wakes NAS when needed";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Install the Windows service
    Install,
    /// Uninstall the Windows service
    Uninstall,
    /// Generate default configuration file
    GenerateConfig,
    /// Run in console mode (for testing)
    Run,
}

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    nas_mac: String,
    nas_ip: String,
    router_ip: String,
    heartbeat_url: String,
    check_interval_secs: u64,
    idle_threshold_mins: u32,
    heartbeat_timeout_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            nas_mac: "00:08:9B:DB:EF:9A".to_string(),
            nas_ip: "192.168.42.2".to_string(),
            router_ip: "192.168.42.1".to_string(),
            heartbeat_url: "http://192.168.42.2:8090/heartbeat".to_string(),
            check_interval_secs: 30,
            idle_threshold_mins: 5,
            heartbeat_timeout_secs: 5,
        }
    }
}

define_windows_service!(ffi_service_main, service_main);

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Install) => install_service()?,
        Some(Commands::Uninstall) => uninstall_service()?,
        Some(Commands::GenerateConfig) => generate_config()?,
        Some(Commands::Run) => run_console_mode()?,
        None => {
            // Run as Windows service
            info!("Starting service dispatcher");
            service_dispatcher::start(SERVICE_NAME, ffi_service_main)
                .map_err(|e| anyhow::anyhow!("Service dispatcher error: {:?}", e))?;
        }
    }

    Ok(())
}

fn get_config_path() -> PathBuf {
    // Use system-wide config location instead of user home directory
    // Determine path from env variable:
    let program_data_dir =
        std::env::var("ProgramData").unwrap_or_else(|_| String::from("C:\\ProgramData"));
    let mut path = PathBuf::from(program_data_dir);
    path.push("NASBootClient");
    path.push("nas-boot-client-config.yaml");
    path
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
        nas_mac: doc["nas_mac"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing nas_mac"))?
            .to_string(),
        nas_ip: doc["nas_ip"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing nas_ip"))?
            .to_string(),
        router_ip: doc["router_ip"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing router_ip"))?
            .to_string(),
        heartbeat_url: doc["heartbeat_url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing heartbeat_url"))?
            .to_string(),
        check_interval_secs: doc["check_interval_secs"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("Missing check_interval_secs"))?
            as u64,
        idle_threshold_mins: doc["idle_threshold_mins"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("Missing idle_threshold_mins"))?
            as u32,
        heartbeat_timeout_secs: doc["heartbeat_timeout_secs"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("Missing heartbeat_timeout_secs"))?
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

fn install_service() -> Result<()> {
    let manager =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)?;

    let service_binary_path = std::env::current_exe()?;

    let service_info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: service_binary_path,
        launch_arguments: vec![],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };

    let service = manager.create_service(&service_info, ServiceAccess::ALL_ACCESS)?;

    service.set_description(SERVICE_DESCRIPTION)?;

    println!("Service '{SERVICE_NAME}' installed successfully");
    println!("Start it with: sc start {SERVICE_NAME}");
    Ok(())
}

fn uninstall_service() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(SERVICE_NAME, ServiceAccess::DELETE)?;

    service.delete()?;

    println!("Service '{SERVICE_NAME}' uninstalled successfully");
    Ok(())
}

fn run_console_mode() -> Result<()> {
    // Initialize the logger for console mode
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    info!("Starting in console mode");

    let config = load_config()?;
    let rt = tokio::runtime::Runtime::new()?;

    // Create a Ctrl+C handler for console mode
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    rt.block_on(async {
        // Set up Ctrl-C handler for graceful shutdown in console mode
        let r = r.clone();
        tokio::spawn(async move {
            match tokio::signal::ctrl_c().await {
                Ok(()) => {
                    info!("Received Ctrl+C, shutting down...");
                    r.store(false, Ordering::Relaxed);
                }
                Err(err) => error!("Unable to listen for Ctrl+C: {err}"),
            }
        });

        monitor_loop(running, config).await;
    });

    info!("Console mode exited");

    Ok(())
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service() {
        error!("Service error: {e}");
    }
}

fn run_service() -> windows_service::Result<()> {
    // For debugging purposes, write to a log file in the ProgramData directory
    let log_file_path = get_config_path().with_extension("log");
    let log_target = Box::new(File::create(&log_file_path).map_err(|e| {
        windows_service::Error::Winapi(std::io::Error::other(format!(
            "Failed to create log file: {e}"
        )))
    })?);
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Pipe(log_target))
        .init();
    info!("Service starting...");

    // Create shutdown signal FIRST
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    // Register the service control handler
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                info!("Service stop requested");
                r.store(false, Ordering::Relaxed);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    info!("Registering service control handler for {SERVICE_NAME}");
    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    info!("Service handler registered successfully");

    // Update the service status to running
    if let Err(e) = status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    }) {
        error!("Failed to set service status to running: {e}");
        return Err(e);
    }

    info!("Service status set to running");

    // Load configuration with detailed error reporting
    let config = match load_config() {
        Ok(c) => {
            info!("Configuration loaded successfully");
            c
        }
        Err(e) => {
            error!("Failed to load configuration: {e}");
            // Update service status to stopped with error
            let _ = status_handle.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::ServiceSpecific(1),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            });
            return Err(windows_service::Error::Winapi(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Config load failed: {e}"),
            )));
        }
    };

    info!("Creating Tokio runtime");

    // Create a runtime for async operations
    let rt = match tokio::runtime::Runtime::new() {
        Ok(runtime) => {
            info!("Tokio runtime created successfully");
            runtime
        }
        Err(e) => {
            error!("Failed to create Tokio runtime: {e}");
            let _ = status_handle.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::ServiceSpecific(2),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            });
            return Err(windows_service::Error::Winapi(std::io::Error::other(
                format!("Runtime creation failed: {e}"),
            )));
        }
    };

    info!("Starting monitor loop");

    // Run the monitor loop
    rt.block_on(async {
        monitor_loop(running.clone(), config).await;
    });

    info!("Monitor loop completed, shutting down service");

    // Update the service status to stopped
    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });

    info!("Service stopped successfully");
    Ok(())
}

async fn monitor_loop(running: Arc<AtomicBool>, config: Config) {
    let mut interval = time::interval(Duration::from_secs(config.check_interval_secs));
    let mut last_active = false;

    info!(
        "Monitor loop started with config: server={}",
        config.heartbeat_url
    );

    while running.load(Ordering::Relaxed) {
        interval.tick().await;

        let is_active = is_user_active(config.idle_threshold_mins);

        if is_active && !last_active {
            info!("User became active, waking NAS");
            wake_nas(&config).await;
        }

        if is_active {
            send_heartbeat(&config).await;
        }

        last_active = is_active;
    }

    info!("Monitor loop exiting");
}

fn is_user_active(idle_threshold_mins: u32) -> bool {
    // First check if anyone is logged in
    if !has_active_user_sessions() {
        info!("No active user sessions");
        return false;
    }

    // TODO: detect user activity by checking file system changes

    true
}

fn has_active_user_sessions() -> bool {
    match Command::new("query").args(["session"]).output() {
        Ok(output) => {
            let output_str = String::from_utf8_lossy(&output.stdout);

            // Look for active sessions
            for line in output_str.lines() {
                let line = line.trim();

                // Skip header lines
                if line.starts_with("SESSIONNAME") || line.starts_with('-') || line.is_empty() {
                    continue;
                }

                // Check for active console sessions
                if line.contains("console") && line.contains("Active") {
                    info!("Active console session detected: {line}");
                    return true;
                }

                // Check for active RDP sessions
                if line.contains("rdp-tcp") && line.contains("Active") {
                    info!("Active RDP session detected: {line}");
                    return true;
                }

                // Check for any other active sessions
                if line.contains("Active") {
                    info!("Active session detected: {line}");
                    return true;
                }
            }

            info!("No active user sessions found");
            false
        }
        Err(e) => {
            error!("Failed to query sessions: {e}");
            // Fallback: assume user is active if we can't determine
            true
        }
    }
}

async fn wake_nas(config: &Config) {
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
            Ok(()) => info!("Sent WOL packet to NAS"),
            Err(e) => error!("Failed to send WOL packet: {e}"),
        }

        // Send via router
        let router_addr = format!("{}:9", config.router_ip);
        match MagicPacket::new(&mac).send_to(&router_addr as &str, "0.0.0.0:0") {
            Ok(()) => info!("Sent WOL packet via router"),
            Err(e) => error!("Failed to send WOL packet via router: {e}"),
        }
    } else {
        error!("Invalid MAC address format");
    }
}

async fn send_heartbeat(config: &Config) {
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
            } else {
                error!("Heartbeat failed with status: {}", response.status());
            }
        }
        Err(e) => error!("Failed to send heartbeat: {e}"),
    }
}
