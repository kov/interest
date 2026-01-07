//! TUI foundation module
//!
//! Provides the building blocks for the interactive terminal UI: rendering
//! helpers, readline wrapper, overlays, and a lightweight event loop skeleton.

pub mod crossterm_engine;
pub mod event_loop;
pub mod overlays;
pub mod readline;

use crate::commands::parse_command;
use crate::dispatcher::dispatch_command;
use anyhow::Result;
use colored::Colorize;
use rustyline::error::ReadlineError;

const COMMAND_PATTERNS: &[&[&str]] = &[
    &["import"],
    &["portfolio", "show"],
    &["tax", "report"],
    &["tax", "summary"],
    &["help"],
    &["exit"],
    &["quit"],
];

/// Launch the interactive TUI REPL.
pub async fn launch_tui() -> Result<()> {
    println!("{}", "Interest - Interactive Mode".bold());
    println!(
        "Type {} for help, {} to exit\n",
        "/help".cyan(),
        "/exit".cyan()
    );

    let mut rl = readline::Readline::new(COMMAND_PATTERNS, None)?;

    loop {
        match rl.readline("interest> ") {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Handle quit/exit shortcuts
                if trimmed == "/quit" || trimmed == "quit" {
                    println!("Goodbye!");
                    break;
                }

                match parse_command(trimmed) {
                    Ok(cmd) => {
                        if let Err(e) = dispatch_command(cmd, false).await {
                            eprintln!("{} {}", "Error:".red().bold(), e);
                        }
                    }
                    Err(e) => {
                        eprintln!("{} {}", "Parse error:".yellow().bold(), e.message);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl+C
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                // Ctrl+D
                println!("Goodbye!");
                break;
            }
            Err(err) => {
                eprintln!("{} {}", "Error:".red().bold(), err);
                break;
            }
        }
    }

    Ok(())
}
