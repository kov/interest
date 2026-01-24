//! Interactive TUI REPL implementation.

use anyhow::Result;
use colored::Colorize;
use rustyline::error::ReadlineError;

use crate::dispatcher::dispatch_command;
use crate::ui::readline;

/// Parse TUI-style command input into clap Commands
fn parse_tui_command(input: &str) -> Result<crate::cli::Commands> {
    // Strip optional leading slash
    let input = input.strip_prefix('/').unwrap_or(input);

    // Simple approach: build argv from input and use clap to parse it
    // This works because clap already knows how to parse all the commands
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.is_empty() {
        return Err(anyhow::anyhow!("Empty command"));
    }

    // Build argv for clap (prepend program name)
    let mut argv = vec!["interest"];
    argv.extend_from_slice(&parts);

    // Use clap to parse
    use clap::Parser;
    match crate::cli::Cli::try_parse_from(argv) {
        Ok(cli) => {
            if let Some(cmd) = cli.command {
                Ok(cmd)
            } else {
                Err(anyhow::anyhow!("No command specified"))
            }
        }
        Err(e) => {
            // Convert clap error to anyhow
            Err(anyhow::anyhow!("{}", e))
        }
    }
}

const COMMAND_PATTERNS: &[&[&str]] = &[
    // View & inspect
    &["portfolio", "show"],
    &["performance", "show"],
    &["income", "show"],
    &["income", "detail"],
    &["income", "summary"],
    &["income", "add"],
    &["assets", "show"],
    &["inspect"],
    // Import & sync
    &["import"],
    &["import-irpf"],
    &["prices", "update"],
    &["prices", "import-b3"],
    &["prices", "import-b3-file"],
    &["prices", "history"],
    &["assets", "sync-maisretorno"],
    // Resolve & reconcile
    &["inconsistencies", "list"],
    &["inconsistencies", "resolve"],
    &["tickers", "list-unknown"],
    &["tickers", "resolve"],
    // Manage & maintain
    &["assets", "list"],
    &["assets", "add"],
    &["assets", "set-type"],
    &["assets", "set-name"],
    &["transactions", "add"],
    &["transactions", "list"],
    &["process-terms"],
    &["actions", "split"],
    &["actions", "apply"],
    // Reports & tax
    &["tax", "report"],
    &["tax", "summary"],
    &["tax", "calculate"],
    // Utilities & session
    &["prices", "clear-cache"],
    &["tickers", "status"],
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

                match parse_tui_command(trimmed) {
                    Ok(cmd) => {
                        if let Err(e) = dispatch_command(&cmd, false).await {
                            eprintln!("{} {}", "Error:".red().bold(), e);
                        }
                    }
                    Err(e) => {
                        eprintln!("{} {}", "Parse error:".yellow().bold(), e);
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
