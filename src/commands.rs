//! Command parsing and routing layer
//!
//! Provides a simple, custom command parser that works with both CLI arguments
//! and interactive readline input. Replaces clap for command dispatching.
//!
//! This allows the same command logic to be used by both the traditional CLI
//! and the interactive TUI.

/// Parsed command from user input
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Variants constructed via parse_command() runtime string matching
pub enum Command {
    /// Import transactions from file: `import <path> [--dry-run]`
    Import { path: String, dry_run: bool },
    /// Show portfolio: `portfolio show [--filter stock|fii|...]`
    PortfolioShow { filter: Option<String> },
    /// Show performance: `performance show <MTD|QTD|YTD|1Y|ALL|from:to>`
    PerformanceShow { period: String },
    /// Show tax report: `tax report <year> [--export]`
    TaxReport { year: i32, export_csv: bool },
    /// Show tax summary: `tax summary <year>`
    TaxSummary { year: i32 },
    /// Price management: `prices import-b3 <year> [--nocache]` or `prices clear-cache [year]`
    Prices { action: PricesAction },
    /// Show help
    Help,
    /// Exit/quit
    Exit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Variants constructed via parse_command() runtime string matching
pub enum PricesAction {
    /// Import COTAHIST from B3: `prices import-b3 <year> [--nocache]`
    ImportB3 { year: i32, no_cache: bool },
    /// Import COTAHIST from a local ZIP file: `prices import-b3-file <path>`
    ImportB3File { path: String },
    /// Clear B3 cache: `prices clear-cache [year]`
    ClearCache { year: Option<i32> },
}

/// Error type for command parsing
#[derive(Debug, Clone)]
#[allow(dead_code)] // Kept for Phase 3+ TUI implementation
pub struct CommandParseError {
    pub message: String,
}

impl std::fmt::Display for CommandParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CommandParseError {}

/// Parse a command string into a Command enum
///
/// Supports both "long form" (with full keywords) and "short form" (slash commands).
/// Examples:
/// - `import file.xlsx` or `/import file.xlsx`
/// - `portfolio show stock` or `/portfolio show --filter stock`
/// - `tax report 2024` or `/tax report 2024`
#[allow(dead_code)] // Kept for Phase 3+ TUI implementation
pub fn parse_command(input: &str) -> Result<Command, CommandParseError> {
    let input = input.trim();

    // Handle empty input
    if input.is_empty() {
        return Err(CommandParseError {
            message: "Empty command. Type `/help` for commands.".to_string(),
        });
    }

    // Remove leading slash if present
    let input = input.strip_prefix('/').unwrap_or(input);

    let mut parts = input.split_whitespace();
    let cmd = parts.next().ok_or_else(|| CommandParseError {
        message: "No command provided".to_string(),
    })?;

    match cmd.to_lowercase().as_str() {
        "import" => {
            let path = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message: "import requires a file path. Usage: import <path> [--dry-run]"
                        .to_string(),
                })?
                .to_string();

            let dry_run = parts.any(|p| p == "--dry-run");

            Ok(Command::Import { path, dry_run })
        }
        "portfolio" => {
            let action = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message:
                        "portfolio requires action (show). Usage: portfolio show [-a|--asset-type <type>]"
                            .to_string(),
                })?
                .to_lowercase();

            match action.as_str() {
                "show" => {
                    // Look for --asset-type or -a option
                    let mut filter = None;
                    let collected: Vec<_> = parts.collect();

                    for i in 0..collected.len() {
                        if (collected[i] == "--asset-type" || collected[i] == "-a")
                            && i + 1 < collected.len()
                        {
                            filter = Some(collected[i + 1].to_string());
                            break;
                        }
                    }

                    Ok(Command::PortfolioShow { filter })
                }
                _ => Err(CommandParseError {
                    message: format!("Unknown portfolio action: {}. Use: portfolio show", action),
                }),
            }
        }
        "performance" => {
            let action = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message:
                        "performance requires action (show). Usage: performance show <MTD|QTD|YTD|1Y|ALL|from:to>"
                            .to_string(),
                })?
                .to_lowercase();

            match action.as_str() {
                "show" => {
                    let period = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "performance show requires a period. Usage: performance show <MTD|QTD|YTD|1Y|ALL|from:to>".to_string(),
                        })?
                        .to_string();

                    Ok(Command::PerformanceShow { period })
                }
                _ => Err(CommandParseError {
                    message: format!(
                        "Unknown performance action: {}. Use: performance show",
                        action
                    ),
                }),
            }
        }
        "tax" => {
            let action = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message:
                        "tax requires action (report, summary). Usage: tax <report|summary> <year>"
                            .to_string(),
                })?
                .to_lowercase();

            let year = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message: format!(
                        "tax {} requires a year. Usage: tax {} <year> [--export]",
                        action, action
                    ),
                })?
                .parse::<i32>()
                .map_err(|_| CommandParseError {
                    message: "Year must be a valid number (e.g., 2024)".to_string(),
                })?;

            // Remaining args for optional flags
            let export_csv = parts.any(|p| p.eq_ignore_ascii_case("--export"));

            match action.as_str() {
                "report" => Ok(Command::TaxReport { year, export_csv }),
                "summary" => Ok(Command::TaxSummary { year }),
                _ => Err(CommandParseError {
                    message: format!("Unknown tax action: {}. Use: report or summary", action),
                }),
            }
        }
        "prices" | "price" => {
            let action = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message: "prices requires action. Usage: prices <import-b3|clear-cache> [args]"
                        .to_string(),
                })?
                .to_lowercase();

            match action.as_str() {
                "import-b3" => {
                    let year = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "import-b3 requires a year. Usage: prices import-b3 <year> [--nocache]".to_string(),
                        })?
                        .parse::<i32>()
                        .map_err(|_| CommandParseError {
                            message: "Year must be a valid number (e.g., 2023)".to_string(),
                        })?;

                    let no_cache = parts.any(|p| p == "--nocache");

                    Ok(Command::Prices {
                        action: PricesAction::ImportB3 { year, no_cache },
                    })
                }
                "import-b3-file" => {
                    let path = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "import-b3-file requires a ZIP path. Usage: prices import-b3-file <path>".to_string(),
                        })?
                        .to_string();

                    Ok(Command::Prices {
                        action: PricesAction::ImportB3File { path },
                    })
                }
                "clear-cache" => {
                    let year = parts.next().and_then(|y| y.parse::<i32>().ok());

                    Ok(Command::Prices {
                        action: PricesAction::ClearCache { year },
                    })
                }
                _ => Err(CommandParseError {
                    message: format!(
                        "Unknown prices action: {}. Use: import-b3 or clear-cache",
                        action
                    ),
                }),
            }
        }
        "help" | "?" => Ok(Command::Help),
        "exit" | "quit" | "q" => Ok(Command::Exit),
        _ => Err(CommandParseError {
            message: format!(
                "Unknown command: '{}'. Type 'help' for available commands.",
                cmd
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_import_command() {
        let cmd = parse_command("import file.xlsx").unwrap();
        assert_eq!(
            cmd,
            Command::Import {
                path: "file.xlsx".to_string(),
                dry_run: false
            }
        );
    }

    #[test]
    fn test_parse_import_with_slash() {
        let cmd = parse_command("/import file.xlsx").unwrap();
        assert_eq!(
            cmd,
            Command::Import {
                path: "file.xlsx".to_string(),
                dry_run: false
            }
        );
    }

    #[test]
    fn test_parse_import_dry_run() {
        let cmd = parse_command("import file.xlsx --dry-run").unwrap();
        assert_eq!(
            cmd,
            Command::Import {
                path: "file.xlsx".to_string(),
                dry_run: true
            }
        );
    }

    #[test]
    fn test_parse_portfolio_show() {
        let cmd = parse_command("portfolio show").unwrap();
        assert_eq!(cmd, Command::PortfolioShow { filter: None });
    }

    #[test]
    fn test_parse_portfolio_show_with_filter() {
        let cmd = parse_command("portfolio show --asset-type stock").unwrap();
        assert_eq!(
            cmd,
            Command::PortfolioShow {
                filter: Some("stock".to_string())
            }
        );
    }

    #[test]
    fn test_parse_portfolio_show_short_filter() {
        let cmd = parse_command("portfolio show -a fii").unwrap();
        assert_eq!(
            cmd,
            Command::PortfolioShow {
                filter: Some("fii".to_string())
            }
        );
    }

    #[test]
    fn test_parse_tax_report() {
        let cmd = parse_command("tax report 2024").unwrap();
        assert_eq!(
            cmd,
            Command::TaxReport {
                year: 2024,
                export_csv: false
            }
        );
    }

    #[test]
    fn test_parse_tax_report_with_export() {
        let cmd = parse_command("tax report 2024 --export").unwrap();
        assert_eq!(
            cmd,
            Command::TaxReport {
                year: 2024,
                export_csv: true
            }
        );
    }

    #[test]
    fn test_parse_tax_summary() {
        let cmd = parse_command("tax summary 2023").unwrap();
        assert_eq!(cmd, Command::TaxSummary { year: 2023 });
    }

    #[test]
    fn test_parse_snapshot_commands_removed() {
        let save = parse_command("snapshot save checkpoint");
        assert!(save.is_err());
        let list = parse_command("snapshot list");
        assert!(list.is_err());
    }

    #[test]
    fn test_parse_performance_show_mtd() {
        let cmd = parse_command("performance show MTD").unwrap();
        assert_eq!(
            cmd,
            Command::PerformanceShow {
                period: "MTD".to_string()
            }
        );
    }

    #[test]
    fn test_parse_performance_show_ytd() {
        let cmd = parse_command("performance show YTD").unwrap();
        assert_eq!(
            cmd,
            Command::PerformanceShow {
                period: "YTD".to_string()
            }
        );
    }

    #[test]
    fn test_parse_performance_without_period() {
        let result = parse_command("performance show");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("requires a period"));
    }

    #[test]
    fn test_parse_help() {
        let cmd = parse_command("help").unwrap();
        assert_eq!(cmd, Command::Help);

        let cmd = parse_command("?").unwrap();
        assert_eq!(cmd, Command::Help);
    }

    #[test]
    fn test_parse_exit() {
        let cmd = parse_command("exit").unwrap();
        assert_eq!(cmd, Command::Exit);

        let cmd = parse_command("quit").unwrap();
        assert_eq!(cmd, Command::Exit);

        let cmd = parse_command("q").unwrap();
        assert_eq!(cmd, Command::Exit);
    }

    #[test]
    fn test_parse_unknown_command() {
        let result = parse_command("invalid");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Unknown command"));
    }

    #[test]
    fn test_parse_import_without_path() {
        let result = parse_command("import");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("requires a file path"));
    }

    #[test]
    fn test_parse_tax_without_year() {
        let result = parse_command("tax report");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("requires a year"));
    }

    #[test]
    fn test_parse_tax_invalid_year() {
        let result = parse_command("tax report abc");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .message
            .contains("must be a valid number"));
    }

    #[test]
    fn test_parse_empty_input() {
        let result = parse_command("");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Empty command"));
    }

    #[test]
    fn test_parse_whitespace_only() {
        let result = parse_command("   ");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_case_insensitive() {
        let cmd1 = parse_command("IMPORT file.txt").unwrap();
        let cmd2 = parse_command("import file.txt").unwrap();
        assert_eq!(cmd1, cmd2);

        let cmd1 = parse_command("PORTFOLIO SHOW").unwrap();
        let cmd2 = parse_command("portfolio show").unwrap();
        assert_eq!(cmd1, cmd2);
    }
}
