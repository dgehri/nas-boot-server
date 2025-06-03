use crate::app::AppState;
use crate::config::Config;
use crate::nas::{send_heartbeat, wake_nas};
use crate::system::{
    close_window, find_app_window, hide_window, is_auto_start_enabled, set_auto_start, show_window,
};
use crate::user_activity::is_user_active;
use anyhow::Result;
use eframe::{egui, Frame};
use log::info;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::time;
use tray_item::{IconSource, TrayItem};

pub struct NasBootGui {
    config: Config,
    app_state: Arc<Mutex<AppState>>,
    last_heartbeat_time: Arc<Mutex<Instant>>,
    auto_start_enabled: bool,
    last_check_time: Instant,
    runtime: Runtime,
    window_visible: Arc<AtomicBool>,
    tray_item: Option<TrayItem>,
    egui_ctx: Option<egui::Context>,
    exit: Arc<AtomicBool>,
    keep_nas_on: Arc<AtomicBool>,
}

impl NasBootGui {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: Config,
        shared_state: Arc<Mutex<AppState>>,
        last_heartbeat: Arc<Mutex<Instant>>,
        window_visible: Arc<AtomicBool>,
        keep_nas_on: Arc<AtomicBool>,
    ) -> Self {
        let auto_start_enabled = is_auto_start_enabled().unwrap_or(false);

        // Store the egui context for viewport commands
        let egui_ctx = Some(cc.egui_ctx.clone());

        Self {
            config,
            app_state: shared_state,
            last_heartbeat_time: last_heartbeat,
            auto_start_enabled,
            last_check_time: Instant::now(),
            runtime: Runtime::new().expect("Failed to create Tokio runtime"),
            window_visible,
            tray_item: None,
            egui_ctx,
            exit: Arc::new(AtomicBool::new(false)),
            keep_nas_on,
        }
    }

    fn update_status_text(&self) -> String {
        let state = *self.app_state.lock().unwrap();
        match state {
            AppState::Unknown => "Status: Unknown".to_string(),
            AppState::UserIdle => {
                if self.keep_nas_on.load(Ordering::SeqCst) {
                    "Status: User is idle (keeping NAS on)".to_string()
                } else {
                    "Status: User is idle".to_string()
                }
            },
            AppState::UserActive => {
                "Status: User active, attempting to connect to NAS...".to_string()
            }
            AppState::NasAvailable => "Status: Connected to NAS".to_string(),
        }
    }

    fn last_heartbeat_ago(&self) -> String {
        let duration = self.last_heartbeat_time.lock().unwrap().elapsed();
        let seconds = duration.as_secs();

        if seconds < 60 {
            format!("{} seconds ago", seconds)
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
                log::info!("Showing main window from tray using Win32 API");
                if let Ok(hwnd) = find_app_window() {
                    if show_window(hwnd).is_ok() {
                        window_visible.store(true, Ordering::SeqCst);
                        log::info!("Window successfully shown");

                        // Also update egui state if possible
                        if let Some(ctx) = &egui_ctx {
                            log::info!("Sending viewport visible command");
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                            ctx.request_repaint();
                        }
                    } else {
                        log::error!("Failed to show window");
                    }
                } else {
                    log::error!("Could not find application window");
                }
            })?;

            // Add "Exit" menu item using Win32 API directly
            let exit_clone = self.exit.clone();
            tray.add_menu_item("Exit", move || {
                log::info!("Exiting application");
                exit_clone.store(true, Ordering::SeqCst);

                if let Ok(hwnd) = find_app_window() {
                    let _ = show_window(hwnd); // Show window first to ensure it can process close message
                    if let Err(e) = close_window(hwnd) {
                        log::error!("Failed to close window: {:?}", e);
                        std::process::exit(0); // Force exit if message sending fails
                    }
                } else {
                    log::error!("Could not find application window, forcing exit");
                    std::process::exit(0);
                }
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
                AppState::UserIdle => {
                    // Use yellow icon if keep_nas_on is active
                    if self.keep_nas_on.load(Ordering::SeqCst) {
                        tray.set_icon(IconSource::Resource("nas_yellow_ico"))?
                    } else {
                        tray.set_icon(IconSource::Resource("nas_red_ico"))?
                    }
                },
                AppState::UserActive => tray.set_icon(IconSource::Resource("nas_yellow_ico"))?,
                AppState::NasAvailable => tray.set_icon(IconSource::Resource("nas_green_ico"))?,
            }
        }
        Ok(())
    }

    // Helper method to hide window consistently using both approaches
    fn hide_to_tray(&self, ctx: &egui::Context) {
        log::info!("Hiding window to tray");

        // First try the Win32 API approach
        if let Ok(hwnd) = find_app_window() {
            if let Err(e) = hide_window(hwnd) {
                log::error!("Failed to hide window with Win32 API: {:?}", e);
            } else {
                log::info!("Window hidden with Win32 API");
            }
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
                log::error!("Failed to set up tray icon: {}", e);
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.exit.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            log::info!("Close requested, hiding to tray instead");
            self.hide_to_tray(ctx);
        }

        // Check if we need to update the state (every second)
        if self.last_check_time.elapsed() > Duration::from_secs(1) {
            self.last_check_time = Instant::now();

            // Refresh auto-start status
            self.auto_start_enabled = is_auto_start_enabled().unwrap_or(self.auto_start_enabled);

            // Update tray icon if state changed
            if let Err(e) = self.update_tray_icon() {
                log::error!("Failed to update tray icon: {}", e);
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

            ui.horizontal(|ui| {
                ui.label(format!("Last heartbeat: {}", self.last_heartbeat_ago()));
            });

            ui.add_space(10.0);

            // Add checkbox for keeping NAS on
            ui.horizontal(|ui| {
                let mut keep_nas_on = self.keep_nas_on.load(Ordering::SeqCst);
                if ui.checkbox(&mut keep_nas_on, "Keep NAS on").changed() {
                    self.keep_nas_on.store(keep_nas_on, Ordering::SeqCst);
                    info!("Keep NAS on set to {}", keep_nas_on);
                }
            });

            ui.add_space(10.0);

            // Auto-start toggle
            ui.horizontal(|ui| {
                let mut auto_start = self.auto_start_enabled;
                if ui.checkbox(&mut auto_start, "Start with Windows").changed() {
                    if let Err(e) = set_auto_start(auto_start) {
                        info!("Failed to set auto-start: {}", e);
                    } else {
                        self.auto_start_enabled = auto_start;
                    }
                }
            });

            ui.add_space(10.0);

            // Wake NAS button
            if ui.button("Wake NAS").clicked() {
                let config = self.config.clone();
                // Use the runtime that we've created
                self.runtime.spawn(async move {
                    info!("Manually sending WOL packet to NAS");
                    wake_nas(&config).await;
                });
            }

            // Force status update button
            if ui.button("Check Connection").clicked() {
                let config = self.config.clone();
                let app_state = self.app_state.clone();
                let last_heartbeat_time = self.last_heartbeat_time.clone();

                // Use the runtime that we've created
                self.runtime.spawn(async move {
                    info!("Manually checking NAS status");
                    match send_heartbeat(&config).await {
                        Ok(true) => {
                            *app_state.lock().unwrap() = AppState::NasAvailable;
                            info!("State changed to NasAvailable (manual check)");
                        }
                        _ => {
                            if *app_state.lock().unwrap() == AppState::NasAvailable {
                                *app_state.lock().unwrap() = AppState::UserActive;
                                info!("State changed to UserActive (manual check)");
                            }
                        }
                    }
                    *last_heartbeat_time.lock().unwrap() = Instant::now();
                });
            }

            ui.add_space(10.0);

            // Hide to tray button using a combination of viewport commands and our state tracking
            if ui.button("Hide to Tray").clicked() {
                self.hide_to_tray(ctx);
            }
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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([350.0, 200.0])
            .with_resizable(false)
            .with_minimize_button(true)
            .with_always_on_top(),
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
            if let Err(e) = run_background_tasks(
                config_clone,
                app_state_clone,
                last_heartbeat_clone,
                window_visible_clone,
                keep_nas_on_clone,
            )
            .await
            {
                log::error!("Background task error: {}", e);
            }
        });
    });

    eframe::run_native(
        "NAS Boot Client",
        options,
        Box::new(move |cc| {
            Box::new(NasBootGui::new(
                cc,
                config,
                app_state,
                last_heartbeat,
                window_visible,
                keep_nas_on,
            ))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Failed to start GUI: {}", e))?;

    Ok(())
}

pub async fn run_background_tasks(
    config: Config,
    app_state: Arc<Mutex<AppState>>,
    last_heartbeat: Arc<Mutex<Instant>>,
    _window_visible: Arc<AtomicBool>,
    keep_nas_on: Arc<AtomicBool>,
) -> Result<()> {
    let mut interval = time::interval(Duration::from_secs(config.check_interval_secs));

    loop {
        interval.tick().await;
        let is_active = is_user_active(config.idle_threshold_mins);
        let current_state = *app_state.lock().unwrap();
        let keep_on = keep_nas_on.load(Ordering::SeqCst);

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
                *app_state.lock().unwrap() = if is_active { AppState::UserActive } else { AppState::UserIdle };
                info!("State changed from Unknown to {}", if is_active { "UserActive" } else { "UserIdle" });
            }

            // Continue sending wake packets when NAS is not yet available
            if current_state != AppState::NasAvailable {
                info!("Sending WOL packet to NAS");
                wake_nas(&config).await;
            }

            // Send heartbeat and update NAS connection state
            match send_heartbeat(&config).await {
                Ok(true) => {
                    // NAS responded successfully
                    if current_state != AppState::NasAvailable {
                        *app_state.lock().unwrap() = AppState::NasAvailable;
                        info!("State changed to NasAvailable");
                    }
                    // Update last heartbeat time
                    *last_heartbeat.lock().unwrap() = Instant::now();
                }
                _ => {
                    // NAS didn't respond or connection failed
                    if current_state == AppState::NasAvailable {
                        let new_state = if is_active { AppState::UserActive } else { AppState::UserIdle };
                        *app_state.lock().unwrap() = new_state;
                        info!("State changed to {} (heartbeat failed)", if is_active { "UserActive" } else { "UserIdle" });
                    }
                    // Still update the time of the attempt
                    *last_heartbeat.lock().unwrap() = Instant::now();
                }
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
