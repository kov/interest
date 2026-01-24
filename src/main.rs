mod cli;
mod commands;
mod corporate_actions;
mod db;
mod dispatcher;
mod importers;
mod pricing;
mod reports;
mod scraping;
mod tax;
mod term_contracts;
mod tesouro;
mod tickers;
mod ui;
mod utils;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use std::io::IsTerminal;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // If user requested top-level help exactly (e.g., `interest -h` or
    // `interest --help`), render the shared help and exit. Do NOT
    // intercept help when a subcommand is present (e.g., `interest tax --help`)
    // so clap can show subcommand-specific help.
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.len() == 2 && (raw_args[1] == "-h" || raw_args[1] == "--help") {
        let opts = crate::cli::help::RenderOpts::default();
        crate::cli::help::render_help(std::io::stdout(), &opts)?;
        return Ok(());
    }

    // Support legacy-style `interest help` (no subcommand) and `interest ?`.
    if raw_args.len() == 2 && (raw_args[1] == "help" || raw_args[1] == "?") {
        let opts = crate::cli::help::RenderOpts::default();
        crate::cli::help::render_help(std::io::stdout(), &opts)?;
        return Ok(());
    }

    // Parse CLI first to configure logging and color
    let cli = Cli::parse();

    // Determine color usage: disable when requested or when stdout is not a TTY (piped)
    let stdout_is_tty = std::io::stdout().is_terminal();
    let disable_color = cli.no_color || !stdout_is_tty || cli.json;

    // Initialize logging - always write to stderr to keep stdout clean
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn"))
        .add_directive("headless_chrome=error".parse().unwrap());

    tracing_subscriber::fmt()
        .with_ansi(!disable_color)
        .with_writer(std::io::stderr)
        .with_env_filter(env_filter)
        .init();

    // Disable colored crate globally when needed
    if disable_color {
        colored::control::set_override(false);
    }

    // If no command is given, print the top-level help instead of
    // automatically launching the interactive TUI.
    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            let opts = crate::cli::help::RenderOpts::default();
            crate::cli::help::render_help(std::io::stdout(), &opts)?;
            return Ok(());
        }
    };

    if matches!(command, Commands::Interactive) {
        return crate::ui::launch_tui().await;
    }

    dispatcher::dispatch_command(&command, cli.json).await
}
