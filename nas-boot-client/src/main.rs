#![windows_subsystem = "windows"]

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::{generate_config, load_config};
use log::info;
use std::io::Write;
use system::set_auto_start;
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};

mod app;
mod config;
mod gui;
mod nas;
mod system;
mod user_activity;
mod wol;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(flatten)]
    verbose: clap_verbosity::Verbosity,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
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

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    env_logger::builder()
        .format_timestamp_secs()
        .format_target(false)
        .format_module_path(false)
        .format(|buf, record| {
            writeln!(
                buf,
                "{} [{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.args()
            )
        })
        .filter_level(cli.verbose.log_level_filter())
        .init();

    info!("NAS Boot Client starting...");

    match cli.command {
        Some(Commands::GenerateConfig) => generate_config(),
        Some(Commands::EnableAutoStart) => set_auto_start(true).map(|()| {
            info!("Auto-start enabled");
        }),
        Some(Commands::DisableAutoStart) => set_auto_start(false).map(|()| {
            info!("Auto-start disabled");
        }),
        Some(Commands::Debug) => run_app_gui(true),
        None => run_app_gui(false),
    }
}

fn run_app_gui(debug_mode: bool) -> Result<()> {
    // Allocate a Win32 console if running in debug mode
    if debug_mode {
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS)
                .map_err(|e| anyhow::anyhow!("Failed to attach console: {:?}", e))?;
        }
    }

    // Load configuration
    let config = match load_config() {
        Ok(config) => config,
        Err(err) => {
            if debug_mode {
                eprintln!("Error loading configuration: {}", err);
                return Err(err);
            } else {
                // Try to generate a default config if running in normal mode
                generate_config()?;
                load_config()?
            }
        }
    };

    // Run the GUI app directly - it will spawn its own background tasks
    gui::run_gui_app(config)?;

    Ok(())
}
