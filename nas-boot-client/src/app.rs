use anyhow::{Context, Result};
use log::info;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time;
use tray_item::{IconSource, TrayItem};
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use crate::config::Config;
use crate::nas::{send_heartbeat, wake_nas};
use crate::system::{hide_window, is_auto_start_enabled, set_auto_start, show_window};
use crate::user_activity::is_user_active;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AppState {
    #[default]
    Unknown,
    Active,
    Idle,
    NoNas,
}

#[derive(Debug)]
pub enum Message {
    Quit,
    CheckState,
    ToggleAutoStart,
    ShowStatus,
    StateChanged(AppState),
    ShowWindow,
    HideWindow,
}

pub struct App {
    tray: TrayItem,
    tx: mpsc::Sender<Message>,
    state: AppState,
    config: Config,
    last_heartbeat: Instant,
    console_window: Option<windows::Win32::Foundation::HWND>,
    main_window: Option<windows::Win32::Foundation::HWND>,
    window_visible: bool,
}

impl App {
    pub fn new(config: Config) -> Result<(Self, mpsc::Receiver<Message>)> {
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

        let console_window = if console_window.is_invalid() {
            None
        } else {
            Some(console_window)
        };

        let main_window = if main_window.is_invalid() {
            None
        } else {
            Some(main_window)
        };

        let app = Self {
            tray,
            tx,
            state: AppState::Unknown,
            config,
            last_heartbeat: Instant::now(),
            console_window,
            main_window,
            window_visible: true,
        };

        Ok((app, rx))
    }

    pub async fn run(mut self, mut rx: mpsc::Receiver<Message>) -> Result<()> {
        // Hide window by default at startup
        self.hide_window();

        // Set up state check interval
        let mut interval = time::interval(Duration::from_secs(self.config.check_interval_secs));

        // Set up heartbeat state
        let mut was_active = true;

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
                            was_active = self.check_state(was_active).await?;
                        }
                        Message::StateChanged(new_state) => {
                            info!("State changed from {:?} to {:?}", self.state, new_state);
                            self.state = new_state;
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

    // Checks the current state and handles activity changes
    async fn check_state(&mut self, was_active: bool) -> Result<bool> {
        let is_active = is_user_active(self.config.idle_threshold_mins);

        // Update app state if needed
        let new_state = if is_active {
            AppState::Active
        } else {
            AppState::Idle
        };

        if new_state != self.state {
            self.tx.send(Message::StateChanged(new_state)).await?;
        }

        // Handle active user state
        if is_active {
            // Handle wake actions if user just became active
            if !was_active {
                self.handle_user_activation().await;
            }

            // Send heartbeat and update NAS connection state
            self.send_heartbeat_and_update_state().await;
        }

        Ok(is_active)
    }

    // Handles actions when user becomes active
    async fn handle_user_activation(&self) {
        info!("User became active, waking NAS");
        wake_nas(&self.config).await;
    }

    // Sends heartbeat and updates NAS connection state
    async fn send_heartbeat_and_update_state(&mut self) {
        let result = send_heartbeat(&self.config).await;
        self.last_heartbeat = Instant::now();

        // Update state based on heartbeat result
        match result {
            Ok(true) => {
                if self.state == AppState::NoNas {
                    self.tx
                        .send(Message::StateChanged(AppState::Active))
                        .await
                        .ok();
                }
            }
            Ok(false) | Err(_) => {
                if self.state != AppState::NoNas {
                    self.tx
                        .send(Message::StateChanged(AppState::NoNas))
                        .await
                        .ok();
                }
            }
        }
    }

    fn update_tray_icon(&mut self) -> Result<()> {
        match self.state {
            AppState::Active => self.tray.set_icon(IconSource::Resource("nas_green_ico")),
            AppState::Idle => self.tray.set_icon(IconSource::Resource("nas_yellow_ico")),
            AppState::NoNas => self.tray.set_icon(IconSource::Resource("nas_red_ico")),
            AppState::Unknown => self.tray.set_icon(IconSource::Resource("nas_gray_ico")),
        }
        .context("Failed to update tray icon")
    }

    fn show_window(&mut self) {
        if !self.window_visible {
            // Show the console and main windows
            if let Some(hwnd) = self.console_window {
                show_window(hwnd);
            }

            if let Some(hwnd) = self.main_window {
                show_window(hwnd);
            }

            self.window_visible = true;
            info!("Showed application window");
        }
    }

    fn hide_window(&mut self) {
        if self.window_visible {
            // Hide the console and main windows
            if let Some(hwnd) = self.console_window {
                hide_window(hwnd);
            }

            if let Some(hwnd) = self.main_window {
                hide_window(hwnd);
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
        Ok(())
    }

    fn show_status(&self) -> Result<()> {
        let message = format!(
            "NAS Boot Client Status\n\n\
             Connection: {}\n\
             Current state: {:?}\n\
             Last activity check: {}s ago\n\
             Auto-start: {}",
            if self.state == AppState::NoNas {
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

        info!("{}", message);

        Ok(())
    }
}
