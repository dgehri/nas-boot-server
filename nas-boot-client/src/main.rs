#![windows_subsystem = "windows"]

use std::io::Write;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::{generate_config, load_config};
use log::info;
use system::set_auto_start;

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

    /// Run the application with attached console
    WithConsole,
}

fn main() -> Result<()> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            attach_console();
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

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
        Some(Commands::WithConsole) => {
            attach_console();
            run_app()
        }
        None => run_app(),
    }
}

fn run_app() -> Result<()> {
    // Load configuration
    let config = if let Ok(config) = load_config() { config } else {
        generate_config()?;
        load_config()?
    };

    // Run the GUI app directly - it will spawn its own background tasks
    gui::run_gui_app(config)?;

    Ok(())
}

#[cfg(windows)]
fn attach_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};

    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_err() {
        eprintln!("Failed to attach to parent console. Running without console.");
    } else {
        info!("Attached to parent console successfully.");
    }
}

#[cfg(not(windows))]
fn attach_console() {
    // No-op on non-Windows platforms
}
