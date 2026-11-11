use clap::Parser;
use eyre::{Context, Result};
use log::info;
use std::env;
use std::fs;
use std::path::PathBuf;

use whitespace::{Cli, RuntimeConfig};

fn setup_logging() -> Result<()> {
    // Create log directory
    let log_dir = whitespace::config::xdg_data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("whitespace")
        .join("logs");

    fs::create_dir_all(&log_dir).context("Failed to create log directory")?;

    let log_file = log_dir.join("whitespace.log");

    // Setup env_logger with file output
    let target = Box::new(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
            .context("Failed to open log file")?,
    );

    // Check for RUST_LOG environment variable, default to INFO
    let log_level = env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&log_level))
        .target(env_logger::Target::Pipe(target))
        .init();

    info!("Logging initialized, writing to: {}", log_file.display());
    Ok(())
}

fn main() -> Result<()> {
    // Setup logging first
    setup_logging().context("Failed to setup logging")?;

    // Parse CLI arguments
    let cli = Cli::parse();

    info!(
        "Starting with config from: {:?}",
        cli.config
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "defaults".to_string())
    );

    // Build validated runtime configuration
    let runtime_config = RuntimeConfig::from_cli(&cli).context("Failed to build runtime configuration")?;

    // Run the main application logic
    let files_with_changes = whitespace::run(&runtime_config).context("Application failed")?;

    // `--check` is the gate mode: report, change nothing, and fail. Without a
    // non-zero exit here the tool could not gate anything, because both the
    // normal run and `--dry-run` exit 0 whether they found trailing whitespace
    // or not.
    if runtime_config.check && files_with_changes > 0 {
        eprintln!(
            "whitespace: {} file(s) carry trailing whitespace; run `whitespace -r` to fix them",
            files_with_changes
        );
        std::process::exit(1);
    }

    Ok(())
}
