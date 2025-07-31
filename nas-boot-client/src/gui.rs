use crate::app_state::AppState;
use crate::config::{save_config, Config};
use crate::nas::send_heartbeat;
use crate::system::{
    close_window, find_app_window, hide_window, is_auto_start_enabled, is_window_minimized,
    is_window_visible, load_icon_from_resource, set_auto_start, show_window,
};
use crate::user_activity::is_user_active;
use crate::wake_mode::WakeMode;
use crate::wol::wake_nas;
use anyhow::Result;
use eframe::{egui, Frame};
use egui::Margin;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time;
use tray_item::{IconSource, TrayItem};

pub struct NasBootGui {
    config: Arc<Mutex<Config>>,
    app_state: Arc<Mutex<AppState>>,
    last_heartbeat_time: Arc<Mutex<Instant>>,
    auto_start_enabled: bool,
    last_check_time: Instant,
    tray_item: Option<TrayItem>,
    egui_ctx: Option<egui::Context>,
    cancel_token: tokio_util::sync::CancellationToken,
    last_known_state: AppState,
    last_wake_mode: WakeMode,
}

impl NasBootGui {
    pub fn new(
        config: Arc<Mutex<Config>>,
        cc: &eframe::CreationContext<'_>,
        shared_state: Arc<Mutex<AppState>>,
        last_heartbeat: Arc<Mutex<Instant>>,
        cancel_token: tokio_util::sync::CancellationToken,
    ) -> Self {
        let auto_start_enabled = is_auto_start_enabled();

        // Store the egui context for viewport commands
        let egui_ctx = Some(cc.egui_ctx.clone());

        // Set light theme explicitly - this forces a light theme regardless of system settings
        cc.egui_ctx.set_visuals(egui::style::Visuals::light());

        // Configure the style to ensure light theme is used consistently
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::style::Visuals::light();
        cc.egui_ctx.set_style(style);

        Self {
            app_state: shared_state,
            last_heartbeat_time: last_heartbeat,
            auto_start_enabled,
            last_check_time: Instant::now(),
            tray_item: None,
            egui_ctx,
            config,
            cancel_token,
            last_known_state: AppState::Unknown,
            last_wake_mode: WakeMode::default(),
        }
    }

    fn update_status_text(&self) -> String {
        let state = *self.app_state.lock();
        match state {
            AppState::Unknown => "Status: Unknown".to_string(),
            AppState::Idle => match self.config.lock().wake_mode {
                WakeMode::AlwaysOn => "Status: NAS Needed".to_string(),
                WakeMode::Auto => "Status: Idle".to_string(),
                WakeMode::Off => "Status: NAS Not Needed".to_string(),
            },
            AppState::WakeUp => match self.config.lock().wake_mode {
                WakeMode::AlwaysOn => "Status: Waking NAS".to_string(),
                WakeMode::Auto => "Status: Waking NAS".to_string(),
                WakeMode::Off => "Status: NAS Not Needed".to_string(),
            },
            AppState::NasReady => "Status: NAS Ready".to_string(),
        }
    }

    fn last_heartbeat_ago(&self) -> String {
        let duration = self.last_heartbeat_time.lock().elapsed();
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
            let egui_ctx = self.egui_ctx.clone();
            tray.add_menu_item("Show Window", move || {
                if let Ok(hwnd) = find_app_window() {
                    show_window(hwnd);

                    // Also update egui state if possible
                    if let Some(ctx) = &egui_ctx {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.request_repaint();
                    }
                }
            })?;

            // Add "Open NAS Web Page" menu item
            let config_for_web = self.config.clone();
            tray.add_menu_item("Open NAS Web Page", move || {
                let config = config_for_web.lock();
                let web_url = format!("http://{}", config.nas_ip);
                let _ = open_url(&web_url); // Non-blocking, handles its own errors
            })?;

            // Add "Open NAS Drive" menu item
            let config_for_drive = self.config.clone();
            tray.add_menu_item("Open NAS Drive", move || {
                let config = config_for_drive.lock();
                // Use IP address for UNC path - user can modify config if hostname needed
                let unc_path = format!("\\\\{}", config.nas_ip);

                // Try primary path, with fallback in the same thread
                std::thread::spawn(move || {
                    if let Err(e) = open::that(&unc_path) {
                        log::warn!("Failed to open NAS drive at {}: {}", unc_path, e);
                        // Try with fajita.local as fallback
                        let fallback_path = "\\\\fajita.local";
                        if let Err(e2) = open::that(fallback_path) {
                            log::error!("Fallback path {} also failed: {}", fallback_path, e2);
                        } else {
                            log::info!("Successfully opened fallback path: {}", fallback_path);
                        }
                    }
                });
            })?;

            // Add "Exit" menu item using Win32 API directly
            let cancel_token_clone = self.cancel_token.clone();
            tray.add_menu_item("Exit", move || {
                cancel_token_clone.cancel();

                if let Ok(hwnd) = find_app_window() {
                    let () = show_window(hwnd);
                    let _ = close_window(hwnd);
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
            let current_state = *self.app_state.lock();
            match current_state {
                AppState::Unknown => tray.set_icon(IconSource::Resource("nas_grey_ico"))?,
                AppState::Idle => tray.set_icon(IconSource::Resource("nas_grey_ico"))?,
                AppState::WakeUp => tray.set_icon(IconSource::Resource("nas_yellow_ico"))?,
                AppState::NasReady => tray.set_icon(IconSource::Resource("nas_green_ico"))?,
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

        // Force a repaint to apply changes
        ctx.request_repaint();
    }
}

impl eframe::App for NasBootGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Ensure the visuals are set to light mode every frame
        ctx.set_visuals(egui::style::Visuals::light());

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

        // Check if state has changed
        let current_state = *self.app_state.lock();
        let current_wake_mode = self.config.lock().wake_mode;
        let state_changed =
            current_state != self.last_known_state || current_wake_mode != self.last_wake_mode;

        // Only update status periodically or if state changed - increased interval to reduce CPU usage
        if state_changed || self.last_check_time.elapsed() > Duration::from_secs(30) {
            self.last_check_time = Instant::now();
            self.last_known_state = current_state;
            self.last_wake_mode = current_wake_mode;

            // Only refresh auto-start status when state actually changed, not on timer
            if state_changed {
                self.auto_start_enabled = is_auto_start_enabled();
            }

            // Update tray icon if state changed
            if state_changed {
                if let Err(e) = self.update_tray_icon() {
                    log::error!("Failed to update tray icon: {e}");
                }
            }

            // Request repaint only when we've updated something
            ctx.request_repaint();
        }

        // Create a frame with light background color
        let frame = egui::Frame::default()
            .inner_margin(Margin::symmetric(10, 10))
            .fill(egui::Color32::from_rgb(240, 240, 240))
            .stroke(egui::Stroke::default());

        // Use central panel with auto-sizing layout
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            // Use a vertical layout with tight spacing for content
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                ui.heading("NAS Boot Client");

                // Status display
                ui.horizontal(|ui| {
                    let status_text = self.update_status_text();
                    ui.label(status_text);
                });

                ui.add_space(5.0);

                // Use radio buttons for wake mode selection
                {
                    let mut wake_mode = self.config.lock().wake_mode;
                    let old_wake_mode = wake_mode;
                    ui.horizontal(|ui| {
                        ui.label("Wake Mode:");
                        ui.radio_value(&mut wake_mode, WakeMode::Off, "Off");
                        ui.radio_value(&mut wake_mode, WakeMode::Auto, "Auto");
                        ui.radio_value(&mut wake_mode, WakeMode::AlwaysOn, "Always On");
                    });
                    if wake_mode != old_wake_mode {
                        self.config.lock().wake_mode = wake_mode;
                        // Request immediate repaint when user changes settings
                        ctx.request_repaint();
                    }
                }

                ui.add_space(5.0);

                // Auto-start toggle
                ui.horizontal(|ui| {
                    let mut auto_start = self.auto_start_enabled;
                    if ui.checkbox(&mut auto_start, "Start with Windows").changed() {
                        if let Err(e) = set_auto_start(auto_start) {
                            log::info!("Failed to set auto-start: {e}");
                        } else {
                            self.auto_start_enabled = auto_start;
                        }
                        // Request immediate repaint when user changes settings
                        ctx.request_repaint();
                    }
                });

                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label(format!("Last heartbeat: {}", self.last_heartbeat_ago()));
                });
            });
        });

        // Only request periodic repaints if window is visible
        if ctx.input(|i| !i.viewport().minimized.unwrap_or(false)) {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Clean up resources if needed
        self.tray_item = None;

        // Save config
        let _ = save_config(&self.config.lock()).ok();

        // Set cancellation token to signal background tasks to stop
        self.cancel_token.cancel();
    }
}

pub fn run_gui_app(config: Config) -> Result<()> {
    // Create shared state objects
    let config = Arc::new(Mutex::new(config));
    let app_state = Arc::new(Mutex::new(AppState::Unknown));
    let last_heartbeat = Arc::new(Mutex::new(Instant::now()));
    let icon = load_icon_from_resource();

    // Create viewport with auto-sizing properties
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([280.0, 180.0]) // Initial size that works well for the content
        .with_resizable(false)
        .with_minimize_button(true)
        .with_maximize_button(false)
        .with_close_button(false)
        .with_always_on_top()
        .with_icon(icon.unwrap_or_default());

    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    // Create cancellation token for graceful shutdown
    let cancel_token = tokio_util::sync::CancellationToken::new();

    // Pass shared state to background tasks
    {
        let config = config.clone();
        let app_state = app_state.clone();
        let last_heartbeat = last_heartbeat.clone();
        let cancel_token = cancel_token.clone();

        // Start background task in its own thread - this will continue running even when window is hidden
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // Start the main background task
                let background_task = {
                    let cancel_token = cancel_token.clone();
                    tokio::spawn(async move {
                        if let Err(e) =
                            run_background_task(config, app_state, last_heartbeat, cancel_token)
                                .await
                        {
                            log::error!("Background task error: {e}");
                        }
                    })
                };

                // Start the window visibility monitoring task
                let window_monitor_task = {
                    let cancel_token = cancel_token.clone();
                    tokio::spawn(async move { run_minimizer_task(cancel_token).await })
                };

                // Wait for either task to complete
                tokio::select! {
                    _ = background_task => {},
                    _ = window_monitor_task => {},
                    _ = tokio::signal::ctrl_c() => {
                        log::info!("Received Ctrl+C, shutting down...");
                        cancel_token.cancel();
                    },
                }

                if let Ok(hwnd) = find_app_window() {
                    show_window(hwnd);
                    let _ = close_window(hwnd);
                }
            });
        });
    }

    eframe::run_native(
        "NAS Boot Client",
        options,
        Box::new(move |cc| {
            Ok(Box::new(NasBootGui::new(
                config,
                cc,
                app_state,
                last_heartbeat,
                cancel_token,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Failed to start GUI: {}", e))?;

    Ok(())
}

async fn run_minimizer_task(cancel_token: tokio_util::sync::CancellationToken) {
    let mut interval = time::interval(Duration::from_millis(500)); // Fast enough for responsive minimize-to-tray

    loop {
        tokio::select! {
            () = cancel_token.cancelled() => {
                log::info!("Minimizer task cancelled");
                break;
            }

            _ = interval.tick() => {
                // Find the window each time instead of caching it
                if let Ok(hwnd) = find_app_window() {
                    // Check if window should be hidden but isn't in tray
                    if is_window_visible(hwnd) && is_window_minimized(hwnd) {
                        hide_window(hwnd);
                    }
                }
            }
        }
    }
}

pub async fn run_background_task(
    config: Arc<Mutex<Config>>,
    app_state: Arc<Mutex<AppState>>,
    last_heartbeat: Arc<Mutex<Instant>>,
    cancel_token: tokio_util::sync::CancellationToken,
) -> Result<()> {
    let mut interval = time::interval(Duration::from_secs(config.lock().check_interval_secs));

    // Create a channel for state change notifications
    let state_change_tx = Arc::new(tokio::sync::watch::channel(()).0);
    let _state_change_rx = state_change_tx.subscribe(); // Unused for now, but ready for future event-driven updates

    loop {
        tokio::select! {
            () = cancel_token.cancelled() => {
                log::info!("Background task cancelled");
                break;
            }

            _ = interval.tick() => {
                // Continue checking user activity
            }
        }

        let config = config.lock().clone();
        let is_user_active = is_user_active(config.idle_threshold_mins);

        // AppState Matrix:
        //
        // | Mode               | !is_user_active  | is_user_active   |
        // |--------------------|------------------|------------------|
        // | WakeMode::Off      | Idle             | Idle             |
        // | WakeMode::Auto     | Idle             | WakeUp/NasReady  |
        // | WakeMode::AlwaysOn | WakeUp/NasReady  | WakeUp/NasReady  |

        let need_nas = match (config.wake_mode, is_user_active) {
            (WakeMode::Off, _) => false,
            (WakeMode::Auto, false) => false,
            (WakeMode::Auto, true) => true,
            (WakeMode::AlwaysOn, _) => true,
        };

        let next_state = if need_nas {
            if let Ok(true) = send_heartbeat(&config).await {
                log::info!("Heartbeat successful, NAS is ready");
                *last_heartbeat.lock() = Instant::now();
                AppState::NasReady
            } else {
                // Send WOL packet if heartbeat failed
                log::info!("Heartbeat failed, sending WOL packet");

                if let Err(e) = wake_nas(&config).await {
                    log::error!("Failed to send WOL packet: {e}");
                }

                AppState::WakeUp
            }
        } else {
            AppState::Idle
        };

        let old_state = *app_state.lock();
        if old_state != next_state {
            *app_state.lock() = next_state;
            // Notify about state change
            let _ = state_change_tx.send(());
        }
    }

    Ok(())
}

// Helper function to open a URL in the default browser (non-blocking)
fn open_url(url: &str) -> Result<()> {
    let url = url.to_string();
    std::thread::spawn(move || {
        if let Err(e) = open::that(&url) {
            log::error!("Failed to open URL {}: {}", url, e);
        }
    });
    Ok(())
}
