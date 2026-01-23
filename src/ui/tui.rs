//! Interactive TUI REPL implementation.

use anyhow::Result;
use colored::Colorize;
use rustyline::error::ReadlineError;

use crate::commands::parse_command;
use crate::dispatcher::dispatch_command;
use crate::ui::readline;

const COMMAND_PATTERNS: &[&[&str]] = &[
    // View & inspect
    &["portfolio", "show"],
    &["performance", "show"],
    &["income", "show"],
    &["income", "detail"],
    &["assets", "show"],
    &["inspect"],
    // Import & sync
    &["import"],
    &["import-irpf"],
    &["prices", "import-b3"],
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
    &["actions", "split"],
    // Reports & tax
    &["tax", "report"],
    &["tax", "summary"],
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
