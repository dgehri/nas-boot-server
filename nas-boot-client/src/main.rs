use anyhow::{Context, Result};
use chrono::Local;
use clap::{Parser, Subcommand};
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time;
use tray_item::{IconSource, TrayItem};
use wake_on_lan::MagicPacket;
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, ShowWindow, SW_HIDE, SW_SHOW};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;
use yaml_rust2::YamlLoader;

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

    /// Enable auto-start with Windows login
    EnableAutoStart,

    /// Disable auto-start with Windows login
    DisableAutoStart,

    /// Run the application in the foreground (for debugging)
    Debug,
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

#[derive(Debug, Clone, Copy, PartialEq)]
enum AppState {
    Active,
    Idle,
    NoNas,
}

#[derive(Debug)]
enum Message {
    Quit,
    CheckState,
    ToggleAutoStart,
    ShowStatus,
    StateChanged(AppState),
    ShowWindow,
    HideWindow,
}

struct App {
    tray: TrayItem,
    tx: mpsc::Sender<Message>,
    state: Option<AppState>,
    config: Config,
    last_heartbeat: Instant,
    console_window: Option<windows::Win32::Foundation::HWND>,
    main_window: Option<windows::Win32::Foundation::HWND>,
    window_visible: bool,
}

impl App {
    fn new(config: Config) -> Result<(Self, mpsc::Receiver<Message>)> {
        // Get the console window handle if it exists
        let console_window = unsafe { GetConsoleWindow() };
        let main_window = unsafe { GetForegroundWindow() };

        // Set up the tray icon
        let mut tray = TrayItem::new("NAS Boot Client", IconSource::Resource("nas_black_ico"))
            .context("Failed to create tray icon")?;

        // Create a channel for message passing
        let (tx, rx) = mpsc::channel(100);

        // Status menu item
        let tx_clone = tx.clone();
        tray.add_menu_item("Show Status", move || {
            let _ = tx_clone.blocking_send(Message::ShowStatus);
        })
        .context("Failed to add status menu item")?;

        // Window show/hide menu items
        let tx_clone = tx.clone();
        tray.add_menu_item("Show Window", move || {
            let _ = tx_clone.blocking_send(Message::ShowWindow);
        })
        .context("Failed to add show window menu item")?;

        let tx_clone = tx.clone();
        tray.add_menu_item("Hide Window", move || {
            let _ = tx_clone.blocking_send(Message::HideWindow);
        })
        .context("Failed to add hide window menu item")?;

        // Auto-start menu item
        let tx_clone = tx.clone();
        tray.add_menu_item("Toggle Auto-start", move || {
            let _ = tx_clone.blocking_send(Message::ToggleAutoStart);
        })
        .context("Failed to add auto-start menu item")?;

        // Quit menu item
        let tx_clone = tx.clone();
        tray.add_menu_item("Quit", move || {
            let _ = tx_clone.blocking_send(Message::Quit);
        })
        .context("Failed to add quit menu item")?;

        let app = Self {
            tray,
            tx,
            state: None,
            config,
            last_heartbeat: Instant::now(),
            console_window: if console_window.is_invalid() {
                None
            } else {
                Some(console_window)
            },
            main_window: if main_window.is_invalid() {
                None
            } else {
                Some(main_window)
            },
            window_visible: true,
        };

        Ok((app, rx))
    }

    async fn run(mut self, mut rx: mpsc::Receiver<Message>) -> Result<()> {
        // Hide window by default at startup
        self.hide_window();

        // Set up state check interval
        let mut interval = time::interval(Duration::from_secs(self.config.check_interval_secs));

        // Set up heartbeat state
        let mut last_active = true;

        info!("App started, waiting for messages");

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Time to check state
                    self.tx.send(Message::CheckState).await?;
                }

                Some(msg) = rx.recv() => {
                    match msg {
                        Message::Quit => {
                            info!("Quit message received");
                            break;
                        }
                        Message::CheckState => {
                            let is_active = is_user_active(self.config.idle_threshold_mins);
                            let new_state = if is_active {
                                Some(AppState::Active)
                            } else {
                                Some(AppState::Idle)
                            };

                            if new_state != self.state {
                                if let Some(new_state) = new_state {
                                    self.tx.send(Message::StateChanged(new_state)).await?;
                                }
                            }

                            // Handle heartbeat logic
                            if is_active {
                                if !last_active {
                                    info!("User became active, waking NAS");
                                    self.wake_nas().await;
                                }

                                // Send heartbeat if we're active
                                self.send_heartbeat().await;
                            }

                            last_active = is_active;
                        }
                        Message::StateChanged(new_state) => {
                            info!("State changed from {:?} to {:?}", self.state, new_state);
                            self.state = Some(new_state);
                            self.update_tray_icon()?;
                        }
                        Message::ToggleAutoStart => {
                            self.toggle_auto_start()?;
                        }
                        Message::ShowStatus => {
                            self.show_status()?;
                        }
                        Message::ShowWindow => {
                            self.show_window();
                        }
                        Message::HideWindow => {
                            self.hide_window();
                        }
                    }
                }
            }
        }

        info!("Exiting application");
        Ok(())
    }

    fn update_tray_icon(&mut self) -> Result<()> {
        match self.state {
            Some(AppState::Active) => self.tray.set_icon(IconSource::Resource("nas_green_ico")),
            Some(AppState::Idle) => self.tray.set_icon(IconSource::Resource("nas_black_ico")),
            Some(AppState::NoNas) => self.tray.set_icon(IconSource::Resource("nas_red_ico")),
            None => self.tray.set_icon(IconSource::Resource("nas_black_ico")),
        }
        .context("Failed to update tray icon")
    }

    fn show_window(&mut self) {
        if !self.window_visible {
            // Show the console window if it exists
            if let Some(hwnd) = self.console_window {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                }
            }

            // Show the main window if it exists
            if let Some(hwnd) = self.main_window {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                }
            }

            self.window_visible = true;
            info!("Showed application window");
        }
    }

    fn hide_window(&mut self) {
        if self.window_visible {
            // Hide the console window if it exists
            if let Some(hwnd) = self.console_window {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }

            // Hide the main window if it exists
            if let Some(hwnd) = self.main_window {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }

            self.window_visible = false;
            info!("Hid application window");
        }
    }

    fn toggle_auto_start(&self) -> Result<()> {
        let auto_start_enabled = is_auto_start_enabled()?;
        set_auto_start(!auto_start_enabled)?;

        info!(
            "Auto-start is now {}",
            if auto_start_enabled {
                "disabled"
            } else {
                "enabled"
            }
        );
        show_balloon_tip(
            "Auto-start Setting",
            &format!(
                "Auto-start has been {}",
                if auto_start_enabled {
                    "disabled"
                } else {
                    "enabled"
                }
            ),
        );
        Ok(())
    }

    fn show_status(&self) -> Result<()> {
        let message = format!(
            "NAS Boot Client Status\n\n\
             Connection: {}\n\
             Current state: {:?}\n\
             Last activity check: {}s ago\n\
             Auto-start: {}",
            if self.state == Some(AppState::NoNas) {
                "Disconnected"
            } else {
                "Connected"
            },
            self.state,
            self.last_heartbeat.elapsed().as_secs(),
            if is_auto_start_enabled().unwrap_or(false) {
                "Enabled"
            } else {
                "Disabled"
            }
        );

        show_balloon_tip("NAS Boot Client", &message);
        info!("Displayed status message");
        Ok(())
    }

    async fn wake_nas(&mut self) {
        // Parse MAC address
        let mac_parts: Vec<u8> = self
            .config
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
                    show_balloon_tip("NAS Boot Client", "Waking up NAS...");
                }
                Err(e) => error!("Failed to send WOL packet: {e}"),
            }

            // Send via router
            let router_addr = format!("{}:9", self.config.router_ip);
            match MagicPacket::new(&mac).send_to(&router_addr as &str, "0.0.0.0:0") {
                Ok(()) => info!("Sent WOL packet via router"),
                Err(e) => error!("Failed to send WOL packet via router: {e}"),
            }
        } else {
            error!("Invalid MAC address format");
        }
    }

    async fn send_heartbeat(&mut self) {
        let client = reqwest::Client::new();
        let timestamp = Local::now().to_rfc3339();
        let hostname = hostname::get()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        info!("Sending heartbeat from {hostname}");
        self.last_heartbeat = Instant::now();

        match client
            .post(&self.config.heartbeat_url)
            .json(&serde_json::json!({
                "timestamp": timestamp,
                "hostname": hostname
            }))
            .timeout(Duration::from_secs(self.config.heartbeat_timeout_secs))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    info!(
                        "Heartbeat sent successfully to {}",
                        self.config.heartbeat_url
                    );
                    if self.state == Some(AppState::NoNas) {
                        self.tx
                            .send(Message::StateChanged(AppState::Active))
                            .await
                            .ok();
                    }
                } else {
                    error!("Heartbeat failed with status: {}", response.status());
                    if self.state != Some(AppState::NoNas) {
                        self.tx
                            .send(Message::StateChanged(AppState::NoNas))
                            .await
                            .ok();
                    }
                }
            }
            Err(e) => {
                error!("Failed to send heartbeat: {e}");
                if self.state != Some(AppState::NoNas) {
                    self.tx
                        .send(Message::StateChanged(AppState::NoNas))
                        .await
                        .ok();
                }
            }
        }
    }
}

fn main() -> Result<()> {
    // Initialize logging
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    info!("NAS Boot Client starting...");

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::GenerateConfig) => generate_config(),
        Some(Commands::EnableAutoStart) => set_auto_start(true).map(|()| {
            println!("Auto-start enabled");
        }),
        Some(Commands::DisableAutoStart) => set_auto_start(false).map(|()| {
            println!("Auto-start disabled");
        }),
        Some(Commands::Debug) => run_app_with_console(),
        None => run_app(),
    }
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

fn run_app() -> Result<()> {
    // Hide the console window early
    let console_window = unsafe { GetConsoleWindow() };
    if !console_window.is_invalid() {
        unsafe {
            let _ = ShowWindow(console_window, SW_HIDE);
        }
    }

    // Load configuration
    let config = load_config()?;

    // Run the Tokio runtime
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        // Create the app
        let (app, rx) = App::new(config)?;

        // Run the app
        app.run(rx).await
    })
}

fn run_app_with_console() -> Result<()> {
    // In console mode, we don't hide the console window initially
    // but still allow it to be toggled via the tray menu

    // Load configuration
    let config = load_config()?;

    // Run the Tokio runtime
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        // Create the app
        let (app, rx) = App::new(config)?;

        // Run the app
        app.run(rx).await
    })
}

fn is_user_active(idle_threshold_mins: u32) -> bool {
    // Calculate idle threshold in milliseconds
    let idle_threshold_ms = u64::from(idle_threshold_mins) * 60 * 1000;

    // Get current tick count
    let current_tick_count = unsafe { GetTickCount() };

    // Initialize LASTINPUTINFO structure
    let mut last_input_info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };

    // Get the last input info
    let result = unsafe { GetLastInputInfo(&mut last_input_info) };

    if result.as_bool() == false {
        error!("Failed to get last input info");
        return true; // Assume user is active if we can't determine
    }

    // Calculate idle time in milliseconds
    let idle_time = current_tick_count.wrapping_sub(last_input_info.dwTime);

    // Consider user active if idle time is less than threshold
    idle_time < idle_threshold_ms as u32
}

fn set_auto_start(enable: bool) -> Result<()> {
    let run_key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
            KEY_WRITE,
        )
        .context("Failed to open registry key")?;

    let app_path = std::env::current_exe()?.to_string_lossy().to_string();

    if enable {
        run_key
            .set_value("NASBootClient", &app_path)
            .context("Failed to set registry value")?;
        info!("Auto-start enabled");
    } else {
        // Ignore errors if the key doesn't exist
        let _ = run_key.delete_value("NASBootClient");
        info!("Auto-start disabled");
    }

    Ok(())
}

fn is_auto_start_enabled() -> Result<bool> {
    let run_key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        KEY_READ,
    ) {
        Ok(key) => key,
        Err(_) => return Ok(false), // If we can't open the key, assume it's not enabled
    };

    match run_key.get_value::<String, _>("NASBootClient") {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

fn show_balloon_tip(title: &str, message: &str) {
    // This is a stub implementation
    // A real implementation would use Shell_NotifyIcon with a proper window handle

    // Log the notification since we're not actually showing it
    info!("NOTIFICATION - {title}: {message}");
}
