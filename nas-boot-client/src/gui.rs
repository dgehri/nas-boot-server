use crate::app::AppState;
use crate::config::Config;
use crate::nas::send_heartbeat;
use crate::system::{
    close_window, find_app_window, hide_window, is_auto_start_enabled, is_window_iconic,
    load_icon_from_resource, set_auto_start, show_window,
};
use crate::user_activity::is_user_active;
use crate::wol::wake_nas;
use anyhow::Result;
use eframe::{egui, Frame};
use log::info;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::time;
use tray_item::{IconSource, TrayItem};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum WakeMode {
    /// NAS won't be woken up or kept on
    #[default]
    Off,

    /// NAS will be woken up on user activity and kept on unless user is idle
    Auto,

    /// NAS will be kept on regardless of user activity
    AlwaysOn,
}

impl From<u8> for WakeMode {
    fn from(value: u8) -> Self {
        match value {
            0 => WakeMode::Off,
            1 => WakeMode::Auto,
            2 => WakeMode::AlwaysOn,
            _ => WakeMode::Off,
        }
    }
}

pub struct NasBootGui {
    app_state: Arc<Mutex<AppState>>,
    last_heartbeat_time: Arc<Mutex<Instant>>,
    auto_start_enabled: bool,
    last_check_time: Instant,
    window_visible: Arc<AtomicBool>,
    tray_item: Option<TrayItem>,
    egui_ctx: Option<egui::Context>,
    exit: Arc<AtomicBool>,
    wake_mode: Arc<AtomicU8>,
}

impl NasBootGui {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared_state: Arc<Mutex<AppState>>,
        last_heartbeat: Arc<Mutex<Instant>>,
        window_visible: Arc<AtomicBool>,
        wake_mode: Arc<AtomicU8>,
    ) -> Self {
        let auto_start_enabled = is_auto_start_enabled();

        // Store the egui context for viewport commands
        let egui_ctx = Some(cc.egui_ctx.clone());

        Self {
            app_state: shared_state,
            last_heartbeat_time: last_heartbeat,
            auto_start_enabled,
            last_check_time: Instant::now(),
            window_visible,
            tray_item: None,
            egui_ctx,
            exit: Arc::new(AtomicBool::new(false)),
            wake_mode,
        }
    }

    fn update_status_text(&self) -> String {
        let state = *self.app_state.lock().unwrap();
        match state {
            AppState::Unknown => "Status: Unknown".to_string(),
            AppState::UserIdle => match self.wake_mode.load(Ordering::SeqCst).into() {
                WakeMode::AlwaysOn => "Status: Keeping NAS always on".to_string(),
                WakeMode::Auto => "Status: User is idle, NAS will wake on activity".to_string(),
                WakeMode::Off => "Status: NAS will not wake".to_string(),
            },
            AppState::UserActive => match self.wake_mode.load(Ordering::SeqCst).into() {
                WakeMode::AlwaysOn => "Status: Keeping NAS always on".to_string(),
                WakeMode::Auto => "Status: User is active, waking NAS".to_string(),
                WakeMode::Off => "Status: NAS will not wake".to_string(),
            },
            AppState::NasAvailable => "Status: Connected to NAS".to_string(),
        }
    }

    fn last_heartbeat_ago(&self) -> String {
        let duration = self.last_heartbeat_time.lock().unwrap().elapsed();
        let seconds = duration.as_secs();

        if seconds < 60 {
            format!("{seconds} seconds ago")
        } else if seconds < 3600 {
            format!("{} minutes ago", seconds / 60)
        } else {
            format!("{} hours ago", seconds / 3600)
        }
    }

    fn setup_tray(&mut self, _ctx: &egui::Context) -> Result<()> {
        if self.tray_item.is_none() {
            let mut tray = TrayItem::new("NAS Boot Client", IconSource::Resource("nas_black_ico"))?;

            // Add "Show Window" menu item using Win32 API directly
            let window_visible = self.window_visible.clone();
            let egui_ctx = self.egui_ctx.clone();
            tray.add_menu_item("Show Window", move || {
                if let Ok(hwnd) = find_app_window() {
                    show_window(hwnd);
                    if is_window_iconic(hwnd) {
                        window_visible.store(true, Ordering::SeqCst);

                        // Also update egui state if possible
                        if let Some(ctx) = &egui_ctx {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                            ctx.request_repaint();
                        }
                    }
                }
            })?;

            // Add "Exit" menu item using Win32 API directly
            let exit_clone = self.exit.clone();
            tray.add_menu_item("Exit", move || {
                log::info!("Exiting application");
                exit_clone.store(true, Ordering::SeqCst);

                if let Ok(hwnd) = find_app_window() {
                    let () = show_window(hwnd);
                    let _ = close_window(hwnd);
                }
                std::process::exit(0);
            })?;

            // Update tray icon based on current state
            self.update_tray_icon()?;

            self.tray_item = Some(tray);
        }

        Ok(())
    }

    fn update_tray_icon(&mut self) -> Result<()> {
        if let Some(tray) = &mut self.tray_item {
            let current_state = *self.app_state.lock().unwrap();
            match current_state {
                AppState::Unknown => tray.set_icon(IconSource::Resource("nas_grey_ico"))?,
                AppState::UserIdle => tray.set_icon(IconSource::Resource("nas_grey_ico"))?,
                AppState::UserActive => match self.wake_mode.load(Ordering::SeqCst).into() {
                    WakeMode::Off => tray.set_icon(IconSource::Resource("nas_grey_ico"))?,
                    WakeMode::Auto | WakeMode::AlwaysOn => {
                        tray.set_icon(IconSource::Resource("nas_yellow_ico"))?
                    }
                },
                AppState::NasAvailable => tray.set_icon(IconSource::Resource("nas_green_ico"))?,
            }
        }
        Ok(())
    }

    // Helper method to hide window consistently using both approaches
    fn hide_to_tray(&self, ctx: &egui::Context) {
        // First try the Win32 API approach
        if let Ok(hwnd) = find_app_window() {
            hide_window(hwnd);
        } else {
            log::error!("Could not find application window for hiding");
        }

        // Then use the egui approach as well for good measure
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        self.window_visible.store(false, Ordering::SeqCst);

        // Force a repaint to apply changes
        ctx.request_repaint();
    }
}

impl eframe::App for NasBootGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Store ctx reference if not already stored
        if self.egui_ctx.is_none() {
            self.egui_ctx = Some(ctx.clone());
        }

        // Set up tray icon if not already done
        if self.tray_item.is_none() {
            if let Err(e) = self.setup_tray(ctx) {
                log::error!("Failed to set up tray icon: {e}");
            }

            // and hide this window to the tray
            self.hide_to_tray(ctx);
        }

        // Check if we need to update the state (every second)
        if self.last_check_time.elapsed() > Duration::from_secs(1) {
            self.last_check_time = Instant::now();

            // Refresh auto-start status
            self.auto_start_enabled = is_auto_start_enabled();

            // Update tray icon if state changed
            if let Err(e) = self.update_tray_icon() {
                log::error!("Failed to update tray icon: {e}");
            }

            // Request repaint to show latest state
            ctx.request_repaint();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("NAS Boot Client");

            // Status display
            ui.horizontal(|ui| {
                let status_text = self.update_status_text();
                ui.label(status_text);
            });

            ui.add_space(10.0);

            // Use radio buttons for wake mode selection
            ui.horizontal(|ui| {
                ui.label("Wake Mode:");
                let mut wake_mode = self.wake_mode.load(Ordering::SeqCst);
                ui.radio_value(&mut wake_mode, WakeMode::Off as u8, "Off");
                ui.radio_value(&mut wake_mode, WakeMode::Auto as u8, "Auto");
                ui.radio_value(&mut wake_mode, WakeMode::AlwaysOn as u8, "Always On");
                self.wake_mode.store(wake_mode, Ordering::SeqCst);
            });

            ui.add_space(10.0);

            // Auto-start toggle
            ui.horizontal(|ui| {
                let mut auto_start = self.auto_start_enabled;
                if ui.checkbox(&mut auto_start, "Start with Windows").changed() {
                    if let Err(e) = set_auto_start(auto_start) {
                        info!("Failed to set auto-start: {e}");
                    } else {
                        self.auto_start_enabled = auto_start;
                    }
                }
            });

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                ui.label(format!("Last heartbeat: {}", self.last_heartbeat_ago()));
            });
        });

        // Request repaint once per second
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Clean up resources if needed
        self.tray_item = None;
    }
}

pub fn run_gui_app(config: Config) -> Result<()> {
    // Create shared state objects
    let app_state = Arc::new(Mutex::new(AppState::Unknown));
    let last_heartbeat = Arc::new(Mutex::new(Instant::now()));
    let window_visible = Arc::new(AtomicBool::new(true));
    let keep_nas_on = Arc::new(AtomicBool::new(false));
    let icon = load_icon_from_resource();

    let viewport = egui::ViewportBuilder::default()
        .with_resizable(false)
        .with_minimize_button(true)
        .with_always_on_top()
        .with_icon(icon.unwrap_or_default());

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    // Pass shared state to background tasks
    let config_clone = config.clone();
    let app_state_clone = app_state.clone();
    let last_heartbeat_clone = last_heartbeat.clone();
    let window_visible_clone = window_visible.clone();
    let keep_nas_on_clone = keep_nas_on.clone();

    // Start background task in its own thread - this will continue running even when window is hidden
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Start the main background task
            let background_task = tokio::spawn(async move {
                if let Err(e) = run_background_task(
                    config_clone,
                    app_state_clone,
                    last_heartbeat_clone,
                    keep_nas_on_clone,
                )
                .await
                {
                    log::error!("Background task error: {e}");
                }
            });

            // Start the window visibility monitoring task
            let window_monitor_task =
                tokio::spawn(async move { run_minimizer_task(window_visible_clone).await });

            // Wait for either task to complete (they should run indefinitely)
            tokio::select! {
                _ = background_task => {},
                _ = window_monitor_task => {},
            }
        });
    });

    let wake_mode = Arc::new(AtomicU8::new(WakeMode::Auto as u8));

    eframe::run_native(
        "NAS Boot Client",
        options,
        Box::new(move |cc| {
            Ok(Box::new(NasBootGui::new(
                cc,
                app_state,
                last_heartbeat,
                window_visible,
                wake_mode,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Failed to start GUI: {}", e))?;

    Ok(())
}

async fn run_minimizer_task(window_visible: Arc<AtomicBool>) -> ! {
    let mut interval = time::interval(Duration::from_millis(100));

    loop {
        interval.tick().await;

        // Find the window each time instead of caching it
        if let Ok(hwnd) = find_app_window() {
            // Check if window should be hidden but isn't in tray
            if window_visible.load(Ordering::SeqCst) && is_window_iconic(hwnd) {
                log::info!("Window is visible but should be hidden - hiding to tray");
                hide_window(hwnd);
                window_visible.store(false, Ordering::SeqCst);
            }
        }
    }
}

pub async fn run_background_task(
    config: Config,
    app_state: Arc<Mutex<AppState>>,
    last_heartbeat: Arc<Mutex<Instant>>,
    keep_nas_on: Arc<AtomicBool>,
) -> Result<()> {
    let mut interval = time::interval(Duration::from_secs(config.check_interval_secs));

    loop {
        interval.tick().await;
        let is_active = is_user_active(config.idle_threshold_mins);
        let current_state = *app_state.lock().unwrap();
        let keep_on = keep_nas_on.load(Ordering::SeqCst);

        log::debug!("User active: {is_active}, Keep NAS on: {keep_on}");

        // Update based on user activity or keep_nas_on setting
        if is_active || keep_on {
            // If user is active or we're keeping the NAS on
            if current_state == AppState::UserIdle && !keep_on {
                *app_state.lock().unwrap() = AppState::UserActive;
                info!("State changed to UserActive");
            } else if current_state == AppState::UserIdle && keep_on {
                // Stay in UserIdle state but still send heartbeats
                info!("User is idle but 'Keep NAS on' is active - maintaining connection");
            } else if current_state == AppState::Unknown {
                *app_state.lock().unwrap() = if is_active {
                    AppState::UserActive
                } else {
                    AppState::UserIdle
                };
                info!(
                    "State changed from Unknown to {}",
                    if is_active { "UserActive" } else { "UserIdle" }
                );
            }

            // Continue sending wake packets when NAS is not yet available
            if current_state != AppState::NasAvailable {
                info!("Sending WOL packet to NAS");
                if let Err(e) = wake_nas(&config).await {
                    log::error!("Failed to send WOL packet: {e}");
                }
            }

            // Send heartbeat and update NAS connection state
            if let Ok(true) = send_heartbeat(&config).await {
                // NAS responded successfully
                if current_state != AppState::NasAvailable {
                    *app_state.lock().unwrap() = AppState::NasAvailable;
                    info!("State changed to NasAvailable");
                }
                // Update last heartbeat time
                *last_heartbeat.lock().unwrap() = Instant::now();
            } else {
                // NAS didn't respond or connection failed
                if current_state == AppState::NasAvailable {
                    let new_state = if is_active {
                        AppState::UserActive
                    } else {
                        AppState::UserIdle
                    };
                    *app_state.lock().unwrap() = new_state;
                    info!(
                        "State changed to {} (heartbeat failed)",
                        if is_active { "UserActive" } else { "UserIdle" }
                    );
                }
                // Still update the time of the attempt
                *last_heartbeat.lock().unwrap() = Instant::now();
            }
        } else {
            // User is idle and keep_nas_on is false
            if current_state != AppState::UserIdle {
                *app_state.lock().unwrap() = AppState::UserIdle;
                info!("State changed to UserIdle");
            }
            // No heartbeat in idle state when keep_nas_on is false
        }
    }
}
