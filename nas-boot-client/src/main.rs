mod app;
mod config;
mod nas;
mod system;
mod user_activity;

use anyhow::Result;
use app::App;
use clap::{Parser, Subcommand};
use config::{generate_config, load_config};
use log::info;
use system::{hide_window_console, set_auto_start};

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

fn run_app() -> Result<()> {
    // Hide the console window early
    hide_window_console();

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
