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
    /// Import commands and sub-actions (e.g. `import <path> [--dry-run]`)
    Import { action: ImportAction },
    /// Portfolio commands and sub-actions (e.g. `portfolio show`)
    Portfolio { action: PortfolioAction },
    /// Performance commands and sub-actions (e.g. `performance show`)
    Performance { action: PerformanceAction },
    /// Cash flow commands and sub-actions (e.g. `cash-flow show`)
    CashFlow { action: CashFlowAction },
    /// Tax commands and sub-actions (e.g. `tax report`)
    Tax { action: TaxAction },
    /// Income commands and sub-actions (e.g. `income show`)
    Income { action: IncomeAction },
    /// Corporate actions (renames, splits, bonuses, exchanges)
    Actions { action: ActionsAction },
    /// Price management: `prices import-b3 <year> [--nocache]` or `prices clear-cache [year]`
    Prices { action: PricesAction },
    /// Inconsistencies management
    Inconsistencies { action: InconsistenciesAction },
    /// Ticker metadata management
    Tickers { action: TickersAction },
    /// Asset management
    Assets { action: AssetsAction },
    /// Show help
    Help,
    /// Exit/quit
    Exit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Variants constructed via parse_command() runtime string matching
pub enum PortfolioAction {
    /// Show portfolio: `portfolio show [--at <date>] [--filter stock|fii|...]`
    Show {
        filter: Option<String>,
        as_of_date: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Variants constructed via parse_command() runtime string matching
pub enum PerformanceAction {
    /// Show performance: `performance show <MTD|QTD|YTD|1Y|ALL|from:to>`
    Show { period: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Variants constructed via parse_command() runtime string matching
pub enum CashFlowAction {
    /// Show cash flow summary: `cash-flow show [period]`
    /// period: MTD|QTD|YTD|1Y|ALL|<year>|<from:to>
    Show { period: String },
    /// Show cash flow statistics: `cash-flow stats [period]`
    Stats { period: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Variants constructed via parse_command() runtime string matching
pub enum TaxAction {
    /// Generate annual report: `tax report <year> [--export]`
    Report { year: i32, export_csv: bool },
    /// Show tax summary: `tax summary <year>`
    Summary { year: i32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Variants constructed via parse_command() runtime string matching
pub enum IncomeAction {
    /// Show income summary by asset: `income show [year]`
    Show { year: Option<i32> },
    /// Show income detail: `income detail [year] [--asset <ticker>]`
    Detail {
        year: Option<i32>,
        asset: Option<String>,
    },
    /// Show income summary: `income summary [year]`
    Summary { year: Option<i32> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AssetsAction {
    List {
        asset_type: Option<String>,
    },
    Show {
        ticker: String,
    },
    Add {
        ticker: String,
        asset_type: Option<String>,
        name: Option<String>,
    },
    SetType {
        ticker: String,
        asset_type: String,
    },
    SetName {
        ticker: String,
        name: String,
    },
    Rename {
        old_ticker: String,
        new_ticker: String,
    },
    Remove {
        ticker: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Variants constructed via parse_command() runtime string matching
pub enum ActionsAction {
    Rename {
        action: RenameAction,
    },
    Split {
        action: SplitAction,
    },
    Bonus {
        action: BonusAction,
    },
    Spinoff {
        action: ExchangeAction,
    },
    Merger {
        action: ExchangeAction,
    },
    /// Apply corporate actions (bonus synthetic transactions)
    Apply {
        ticker: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum RenameAction {
    Add {
        from: String,
        to: String,
        date: String,
        notes: Option<String>,
    },
    List {
        ticker: Option<String>,
    },
    Remove {
        id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SplitAction {
    Add {
        ticker: String,
        quantity_adjustment: String,
        date: String,
        notes: Option<String>,
    },
    List {
        ticker: Option<String>,
    },
    Remove {
        id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum BonusAction {
    Add {
        ticker: String,
        quantity_adjustment: String,
        date: String,
        notes: Option<String>,
    },
    List {
        ticker: Option<String>,
    },
    Remove {
        id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ExchangeAction {
    Add {
        from: String,
        to: String,
        date: String,
        quantity: String,
        allocated_cost: String,
        cash: Option<String>,
        notes: Option<String>,
    },
    List {
        ticker: Option<String>,
    },
    Remove {
        id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Variants constructed via parse_command() runtime string matching
pub enum ImportAction {
    /// Import a file with auto-detection: `import <path> [--dry-run]`
    File { path: String, dry_run: bool },
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Variants constructed via parse_command() runtime string matching
pub enum InconsistenciesAction {
    /// List inconsistencies
    List {
        status: Option<String>,
        issue_type: Option<String>,
        asset: Option<String>,
    },
    /// Show a single inconsistency
    Show { id: i64 },
    /// Resolve an inconsistency (interactive if no fields provided)
    /// If id is None, iterate through all open inconsistencies
    Resolve {
        id: Option<i64>,
        set: Vec<(String, String)>,
        json: Option<String>,
    },
    /// Ignore an inconsistency
    Ignore { id: i64, reason: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Variants constructed via parse_command() runtime string matching
pub enum TickersAction {
    /// Refresh B3 tickers cache
    Refresh { force: bool },
    /// Show cache status
    Status,
    /// List unknown tickers
    ListUnknown,
    /// Resolve one ticker or all unknowns (interactive if no ticker provided)
    Resolve {
        ticker: Option<String>,
        asset_type: Option<String>,
    },
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

/// Parse flexible date formats: YYYY-MM-DD, YYYY-MM, or YYYY
/// Returns the date as a string for later parsing with full error context
pub fn parse_flexible_date(s: &str) -> Result<String, CommandParseError> {
    use chrono::{Datelike, NaiveDate};

    // YYYY-MM-DD (exact date)
    if NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() {
        return Ok(s.to_string());
    }

    // YYYY-MM (last day of month)
    if let Ok(ym) = NaiveDate::parse_from_str(&format!("{}-01", s), "%Y-%m-%d") {
        // Calculate last day of month
        let next_month = if ym.month() == 12 {
            NaiveDate::from_ymd_opt(ym.year() + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(ym.year(), ym.month() + 1, 1)
        };
        if let Some(nm) = next_month {
            let last_day = nm.pred_opt().unwrap_or(ym);
            return Ok(last_day.format("%Y-%m-%d").to_string());
        }
    }

    // YYYY (December 31)
    if let Ok(year) = s.parse::<i32>() {
        if (1900..=2100).contains(&year) {
            return Ok(format!("{}-12-31", year));
        }
    }

    Err(CommandParseError {
        message: format!("Invalid date '{}'. Use YYYY-MM-DD, YYYY-MM, or YYYY", s),
    })
}

/// Parse a command string into a Command enum
///
/// Supports both "long form" (with full keywords) and "short form" (slash commands).
/// Examples:
/// - `import file.xlsx` or `/import file.xlsx`
/// - `portfolio show stock` or `/portfolio show --filter stock`
/// - `tax report 2024` or `/tax report 2024`
#[allow(dead_code)] // Kept for Phase 3+ TUI implementation
fn tokenize_command(input: &str) -> Result<Vec<String>, CommandParseError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(ch) = chars.next() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else if ch == '\\' {
                    if let Some(next) = chars.peek().copied() {
                        if next == q || next == '\\' {
                            current.push(next);
                            chars.next();
                        } else {
                            current.push(ch);
                        }
                    } else {
                        current.push(ch);
                    }
                } else {
                    current.push(ch);
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                } else if ch.is_whitespace() {
                    if !current.is_empty() {
                        tokens.push(current.clone());
                        current.clear();
                    }
                } else if ch == '\\' {
                    if let Some(next) = chars.peek().copied() {
                        current.push(next);
                        chars.next();
                    } else {
                        current.push(ch);
                    }
                } else {
                    current.push(ch);
                }
            }
        }
    }

    if quote.is_some() {
        return Err(CommandParseError {
            message: "Unterminated quote in command input".to_string(),
        });
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    Ok(tokens)
}

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

    let tokens = tokenize_command(input)?;
    let mut parts = tokens.into_iter();
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

            Ok(Command::Import {
                action: ImportAction::File { path, dry_run },
            })
        }
        "portfolio" => {
            let action = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message: "portfolio requires action (show). Usage: portfolio show [-a|--asset-type <type>]".to_string(),
                })?
                .to_lowercase();

            match action.as_str() {
                "show" => {
                    // Look for --asset-type/-a and --at options
                    let mut filter = None;
                    let mut as_of_date = None;
                    let collected: Vec<_> = parts.collect();

                    let mut i = 0;
                    while i < collected.len() {
                        if (collected[i] == "--asset-type" || collected[i] == "-a")
                            && i + 1 < collected.len()
                        {
                            filter = Some(collected[i + 1].to_string());
                            i += 2;
                        } else if collected[i] == "--at" && i + 1 < collected.len() {
                            // Parse and validate the date, converting to canonical form
                            as_of_date = Some(parse_flexible_date(&collected[i + 1])?);
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }

                    Ok(Command::Portfolio {
                        action: PortfolioAction::Show { filter, as_of_date },
                    })
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

                    Ok(Command::Performance {
                        action: PerformanceAction::Show { period },
                    })
                }
                _ => Err(CommandParseError {
                    message: format!(
                        "Unknown performance action: {}. Use: performance show",
                        action
                    ),
                }),
            }
        }
        "cashflow" | "cash-flow" => {
            let action = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message: "cash-flow requires action (show, stats). Usage: cash-flow show [period] or cash-flow stats [period]"
                        .to_string(),
                })?
                .to_lowercase();

            let period = parts.next().unwrap_or_else(|| "ALL".to_string());

            match action.as_str() {
                "show" => Ok(Command::CashFlow {
                    action: CashFlowAction::Show { period },
                }),
                "stats" => Ok(Command::CashFlow {
                    action: CashFlowAction::Stats { period },
                }),
                _ => Err(CommandParseError {
                    message: format!("Unknown cash-flow action: {}. Use: show or stats", action),
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
                "report" => Ok(Command::Tax {
                    action: TaxAction::Report { year, export_csv },
                }),
                "summary" => Ok(Command::Tax {
                    action: TaxAction::Summary { year },
                }),
                _ => Err(CommandParseError {
                    message: format!("Unknown tax action: {}. Use: report or summary", action),
                }),
            }
        }
        "income" => {
            let action = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message: "income requires action (show, detail, summary). Usage: income show [year], income detail [year] [--asset <ticker>], or income summary <year>"
                        .to_string(),
                })?
                .to_lowercase();

            match action.as_str() {
                "show" => {
                    // income show [year] - summary by asset
                    let year = parts.next().and_then(|y| y.parse::<i32>().ok());
                    Ok(Command::Income {
                        action: IncomeAction::Show { year },
                    })
                }
                "detail" => {
                    // income detail [year] [--asset <ticker>] - detailed view
                    let collected: Vec<_> = parts.collect();
                    let mut year: Option<i32> = None;
                    let mut asset: Option<String> = None;
                    let mut i = 0;

                    while i < collected.len() {
                        if (collected[i] == "--asset" || collected[i] == "-a")
                            && i + 1 < collected.len()
                        {
                            asset = Some(collected[i + 1].to_uppercase());
                            i += 2;
                            continue;
                        }
                        // Try to parse as year if no year set yet
                        if year.is_none() {
                            if let Ok(y) = collected[i].parse::<i32>() {
                                year = Some(y);
                            }
                        }
                        i += 1;
                    }

                    Ok(Command::Income {
                        action: IncomeAction::Detail { year, asset },
                    })
                }
                "summary" => {
                    // income summary [year] - monthly breakdown if year, yearly summary otherwise
                    let year = parts.next().and_then(|y| y.parse::<i32>().ok());
                    Ok(Command::Income {
                        action: IncomeAction::Summary { year },
                    })
                }
                _ => Err(CommandParseError {
                    message: format!(
                        "Unknown income action: {}. Use: show, detail, or summary",
                        action
                    ),
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
        "actions" | "action" => {
            let action_type = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message: "actions requires a type (rename, split, bonus, spinoff, merger)."
                        .to_string(),
                })?
                .to_lowercase();

            if action_type == "apply" {
                let ticker = parts.next().map(|t| t.to_string());
                return Ok(Command::Actions {
                    action: ActionsAction::Apply { ticker },
                });
            }

            let verb = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message: "actions requires a verb (add, list, remove).".to_string(),
                })?
                .to_lowercase();

            let collected: Vec<_> = parts.collect();
            let mut i = 0;
            let mut notes: Option<String> = None;
            let mut cash: Option<String> = None;
            let mut args: Vec<String> = Vec::new();

            while i < collected.len() {
                match collected[i].as_str() {
                    "--notes" if i + 1 < collected.len() => {
                        notes = Some(collected[i + 1].to_string());
                        i += 2;
                    }
                    "--cash" if i + 1 < collected.len() => {
                        cash = Some(collected[i + 1].to_string());
                        i += 2;
                    }
                    _ => {
                        args.push(collected[i].to_string());
                        i += 1;
                    }
                }
            }

            let action = match (action_type.as_str(), verb.as_str()) {
                ("rename", "add") => {
                    if args.len() < 3 {
                        return Err(CommandParseError {
                            message: "actions rename add requires: <from> <to> <date>".to_string(),
                        });
                    }
                    ActionsAction::Rename {
                        action: RenameAction::Add {
                            from: args[0].to_string(),
                            to: args[1].to_string(),
                            date: args[2].to_string(),
                            notes,
                        },
                    }
                }
                ("rename", "list") => ActionsAction::Rename {
                    action: RenameAction::List {
                        ticker: args.first().map(|s| s.to_string()),
                    },
                },
                ("rename", "remove") => {
                    let id = args
                        .first()
                        .ok_or_else(|| CommandParseError {
                            message: "actions rename remove requires an id".to_string(),
                        })?
                        .parse::<i64>()
                        .map_err(|_| CommandParseError {
                            message: "actions rename remove requires a numeric id".to_string(),
                        })?;
                    ActionsAction::Rename {
                        action: RenameAction::Remove { id },
                    }
                }
                ("split", "add") => {
                    if args.len() < 3 {
                        return Err(CommandParseError {
                            message: "actions split add requires: <ticker> <qty_adjustment> <date>"
                                .to_string(),
                        });
                    }
                    ActionsAction::Split {
                        action: SplitAction::Add {
                            ticker: args[0].to_string(),
                            quantity_adjustment: args[1].to_string(),
                            date: args[2].to_string(),
                            notes,
                        },
                    }
                }
                ("split", "list") => ActionsAction::Split {
                    action: SplitAction::List {
                        ticker: args.first().map(|s| s.to_string()),
                    },
                },
                ("split", "remove") => {
                    let id = args
                        .first()
                        .ok_or_else(|| CommandParseError {
                            message: "actions split remove requires an id".to_string(),
                        })?
                        .parse::<i64>()
                        .map_err(|_| CommandParseError {
                            message: "actions split remove requires a numeric id".to_string(),
                        })?;
                    ActionsAction::Split {
                        action: SplitAction::Remove { id },
                    }
                }
                ("bonus", "add") => {
                    if args.len() < 3 {
                        return Err(CommandParseError {
                            message: "actions bonus add requires: <ticker> <qty_adjustment> <date>"
                                .to_string(),
                        });
                    }
                    ActionsAction::Bonus {
                        action: BonusAction::Add {
                            ticker: args[0].to_string(),
                            quantity_adjustment: args[1].to_string(),
                            date: args[2].to_string(),
                            notes,
                        },
                    }
                }
                ("bonus", "list") => ActionsAction::Bonus {
                    action: BonusAction::List {
                        ticker: args.first().map(|s| s.to_string()),
                    },
                },
                ("bonus", "remove") => {
                    let id = args
                        .first()
                        .ok_or_else(|| CommandParseError {
                            message: "actions bonus remove requires an id".to_string(),
                        })?
                        .parse::<i64>()
                        .map_err(|_| CommandParseError {
                            message: "actions bonus remove requires a numeric id".to_string(),
                        })?;
                    ActionsAction::Bonus {
                        action: BonusAction::Remove { id },
                    }
                }
                ("spinoff", "add") => {
                    if args.len() < 5 {
                        return Err(CommandParseError {
                            message: "actions spinoff add requires: <from> <to> <date> <quantity> <allocated_cost>".to_string(),
                        });
                    }
                    ActionsAction::Spinoff {
                        action: ExchangeAction::Add {
                            from: args[0].to_string(),
                            to: args[1].to_string(),
                            date: args[2].to_string(),
                            quantity: args[3].to_string(),
                            allocated_cost: args[4].to_string(),
                            cash,
                            notes,
                        },
                    }
                }
                ("spinoff", "list") => ActionsAction::Spinoff {
                    action: ExchangeAction::List {
                        ticker: args.first().map(|s| s.to_string()),
                    },
                },
                ("spinoff", "remove") => {
                    let id = args
                        .first()
                        .ok_or_else(|| CommandParseError {
                            message: "actions spinoff remove requires an id".to_string(),
                        })?
                        .parse::<i64>()
                        .map_err(|_| CommandParseError {
                            message: "actions spinoff remove requires a numeric id".to_string(),
                        })?;
                    ActionsAction::Spinoff {
                        action: ExchangeAction::Remove { id },
                    }
                }
                ("merger", "add") => {
                    if args.len() < 5 {
                        return Err(CommandParseError {
                            message: "actions merger add requires: <from> <to> <date> <quantity> <allocated_cost>".to_string(),
                        });
                    }
                    ActionsAction::Merger {
                        action: ExchangeAction::Add {
                            from: args[0].to_string(),
                            to: args[1].to_string(),
                            date: args[2].to_string(),
                            quantity: args[3].to_string(),
                            allocated_cost: args[4].to_string(),
                            cash,
                            notes,
                        },
                    }
                }
                ("merger", "list") => ActionsAction::Merger {
                    action: ExchangeAction::List {
                        ticker: args.first().map(|s| s.to_string()),
                    },
                },
                ("merger", "remove") => {
                    let id = args
                        .first()
                        .ok_or_else(|| CommandParseError {
                            message: "actions merger remove requires an id".to_string(),
                        })?
                        .parse::<i64>()
                        .map_err(|_| CommandParseError {
                            message: "actions merger remove requires a numeric id".to_string(),
                        })?;
                    ActionsAction::Merger {
                        action: ExchangeAction::Remove { id },
                    }
                }
                _ => {
                    return Err(CommandParseError {
                        message: format!("Unknown actions command: {} {}", action_type, verb),
                    })
                }
            };

            Ok(Command::Actions { action })
        }
        "inconsistencies" | "inconsistency" => {
            let action = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message: "inconsistencies requires action (list, show, resolve, ignore)"
                        .to_string(),
                })?
                .to_lowercase();

            match action.as_str() {
                "list" => {
                    let mut status = None;
                    let mut issue_type = None;
                    let mut asset = None;
                    let collected: Vec<_> = parts.collect();
                    let mut i = 0;
                    while i < collected.len() {
                        match collected[i].as_str() {
                            "--open" => {
                                status = Some("OPEN".to_string());
                                i += 1;
                            }
                            "--all" => {
                                status = Some("ALL".to_string());
                                i += 1;
                            }
                            "--status" if i + 1 < collected.len() => {
                                status = Some(collected[i + 1].to_string());
                                i += 2;
                            }
                            "--type" if i + 1 < collected.len() => {
                                issue_type = Some(collected[i + 1].to_string());
                                i += 2;
                            }
                            "--asset" if i + 1 < collected.len() => {
                                asset = Some(collected[i + 1].to_string());
                                i += 2;
                            }
                            _ => {
                                i += 1;
                            }
                        }
                    }

                    Ok(Command::Inconsistencies {
                        action: InconsistenciesAction::List {
                            status,
                            issue_type,
                            asset,
                        },
                    })
                }
                "show" => {
                    let id = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "inconsistencies show requires an id".to_string(),
                        })?
                        .parse::<i64>()
                        .map_err(|_| CommandParseError {
                            message: "inconsistencies show requires a numeric id".to_string(),
                        })?;

                    Ok(Command::Inconsistencies {
                        action: InconsistenciesAction::Show { id },
                    })
                }
                "resolve" => {
                    // ID is optional - if not provided, iterate through all open inconsistencies
                    let collected: Vec<_> = parts.collect();
                    let mut i = 0;

                    // Check if first arg is an ID (number) or a flag
                    let id = if !collected.is_empty()
                        && !collected[0].starts_with('-')
                        && collected[0].parse::<i64>().is_ok()
                    {
                        let parsed = collected[0].parse::<i64>().ok();
                        i = 1;
                        parsed
                    } else {
                        None
                    };

                    let mut set = Vec::new();
                    let mut json = None;
                    while i < collected.len() {
                        match collected[i].as_str() {
                            "--set" if i + 1 < collected.len() => {
                                if let Some((k, v)) = collected[i + 1].split_once('=') {
                                    set.push((k.to_string(), v.to_string()));
                                }
                                i += 2;
                            }
                            "--json" if i + 1 < collected.len() => {
                                json = Some(collected[i + 1].to_string());
                                i += 2;
                            }
                            _ => {
                                i += 1;
                            }
                        }
                    }

                    Ok(Command::Inconsistencies {
                        action: InconsistenciesAction::Resolve { id, set, json },
                    })
                }
                "ignore" => {
                    let id = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "inconsistencies ignore requires an id".to_string(),
                        })?
                        .parse::<i64>()
                        .map_err(|_| CommandParseError {
                            message: "inconsistencies ignore requires a numeric id".to_string(),
                        })?;

                    let mut reason = None;
                    let collected: Vec<_> = parts.collect();
                    let mut i = 0;
                    while i < collected.len() {
                        if collected[i] == "--reason" && i + 1 < collected.len() {
                            reason = Some(collected[i + 1].to_string());
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }

                    Ok(Command::Inconsistencies {
                        action: InconsistenciesAction::Ignore { id, reason },
                    })
                }
                _ => Err(CommandParseError {
                    message: format!(
                        "Unknown inconsistencies action: {}. Use: list, show, resolve, ignore",
                        action
                    ),
                }),
            }
        }
        "tickers" | "ticker" => {
            let action = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message: "tickers requires action (refresh, status, list-unknown, resolve)"
                        .to_string(),
                })?
                .to_lowercase();

            match action.as_str() {
                "refresh" => {
                    let force = parts.any(|p| p == "--force");
                    Ok(Command::Tickers {
                        action: TickersAction::Refresh { force },
                    })
                }
                "status" => Ok(Command::Tickers {
                    action: TickersAction::Status,
                }),
                "list-unknown" => Ok(Command::Tickers {
                    action: TickersAction::ListUnknown,
                }),
                "resolve" => {
                    let collected: Vec<_> = parts.collect();
                    let mut i = 0;
                    let ticker = if !collected.is_empty() && !collected[0].starts_with('-') {
                        let t = collected[0].to_string();
                        i = 1;
                        Some(t)
                    } else {
                        None
                    };

                    let mut asset_type = None;
                    while i < collected.len() {
                        if collected[i] == "--type" && i + 1 < collected.len() {
                            asset_type = Some(collected[i + 1].to_string());
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }

                    if ticker.is_some() && asset_type.is_none() {
                        return Err(CommandParseError {
                            message: "tickers resolve requires --type when a ticker is provided"
                                .to_string(),
                        });
                    }

                    Ok(Command::Tickers {
                        action: TickersAction::Resolve { ticker, asset_type },
                    })
                }
                _ => Err(CommandParseError {
                    message: format!(
                        "Unknown tickers action: {}. Use: refresh, status, list-unknown, resolve",
                        action
                    ),
                }),
            }
        }
        "assets" | "asset" => {
            let action = parts
                .next()
                .ok_or_else(|| CommandParseError {
                    message: "assets requires action (list, show, add, set-type, set-name, rename, remove)".to_string(),
                })?
                .to_lowercase();

            match action.as_str() {
                "list" => {
                    let collected: Vec<_> = parts.collect();
                    let mut asset_type = None;
                    let mut i = 0;
                    while i < collected.len() {
                        if collected[i] == "--type" && i + 1 < collected.len() {
                            asset_type = Some(collected[i + 1].to_string());
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }

                    Ok(Command::Assets {
                        action: AssetsAction::List { asset_type },
                    })
                }
                "show" => {
                    let ticker = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "assets show requires a ticker".to_string(),
                        })?
                        .to_string();
                    Ok(Command::Assets {
                        action: AssetsAction::Show { ticker },
                    })
                }
                "add" => {
                    let ticker = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "assets add requires a ticker".to_string(),
                        })?
                        .to_string();
                    let collected: Vec<_> = parts.collect();
                    let mut asset_type = None;
                    let mut name = None;
                    let mut i = 0;
                    while i < collected.len() {
                        if collected[i] == "--type" && i + 1 < collected.len() {
                            asset_type = Some(collected[i + 1].to_string());
                            i += 2;
                        } else if collected[i] == "--name" && i + 1 < collected.len() {
                            name = Some(collected[i + 1].to_string());
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                    Ok(Command::Assets {
                        action: AssetsAction::Add {
                            ticker,
                            asset_type,
                            name,
                        },
                    })
                }
                "set-type" => {
                    let ticker = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "assets set-type requires: <ticker> <type>".to_string(),
                        })?
                        .to_string();
                    let asset_type = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "assets set-type requires: <ticker> <type>".to_string(),
                        })?
                        .to_string();
                    Ok(Command::Assets {
                        action: AssetsAction::SetType { ticker, asset_type },
                    })
                }
                "set-name" => {
                    let ticker = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "assets set-name requires: <ticker> <name>".to_string(),
                        })?
                        .to_string();
                    let name = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "assets set-name requires: <ticker> <name>".to_string(),
                        })?
                        .to_string();
                    Ok(Command::Assets {
                        action: AssetsAction::SetName { ticker, name },
                    })
                }
                "rename" => {
                    let old_ticker = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "assets rename requires: <old> <new>".to_string(),
                        })?
                        .to_string();
                    let new_ticker = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "assets rename requires: <old> <new>".to_string(),
                        })?
                        .to_string();
                    Ok(Command::Assets {
                        action: AssetsAction::Rename {
                            old_ticker,
                            new_ticker,
                        },
                    })
                }
                "remove" => {
                    let ticker = parts
                        .next()
                        .ok_or_else(|| CommandParseError {
                            message: "assets remove requires a ticker".to_string(),
                        })?
                        .to_string();
                    Ok(Command::Assets {
                        action: AssetsAction::Remove { ticker },
                    })
                }
                _ => Err(CommandParseError {
                    message: format!(
                        "Unknown assets action: {}. Use: list, show, add, set-type, set-name, rename, remove",
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
    fn test_parse_flexible_date_full() {
        let result = parse_flexible_date("2024-06-15").unwrap();
        assert_eq!(result, "2024-06-15");
    }

    #[test]
    fn test_parse_flexible_date_month() {
        let result = parse_flexible_date("2024-06").unwrap();
        assert_eq!(result, "2024-06-30"); // Last day of June
    }

    #[test]
    fn test_parse_flexible_date_year() {
        let result = parse_flexible_date("2024").unwrap();
        assert_eq!(result, "2024-12-31"); // December 31
    }

    #[test]
    fn test_parse_flexible_date_december() {
        let result = parse_flexible_date("2024-12").unwrap();
        assert_eq!(result, "2024-12-31"); // Last day of December
    }

    #[test]
    fn test_parse_flexible_date_february_leap_year() {
        let result = parse_flexible_date("2024-02").unwrap();
        assert_eq!(result, "2024-02-29"); // Leap year
    }

    #[test]
    fn test_parse_flexible_date_february_non_leap_year() {
        let result = parse_flexible_date("2023-02").unwrap();
        assert_eq!(result, "2023-02-28"); // Non-leap year
    }

    #[test]
    fn test_parse_flexible_date_invalid() {
        let result = parse_flexible_date("not-a-date");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Invalid date"));
    }

    #[test]
    fn test_parse_import_command() {
        let cmd = parse_command("import file.xlsx").unwrap();
        assert_eq!(
            cmd,
            Command::Import {
                action: ImportAction::File {
                    path: "file.xlsx".to_string(),
                    dry_run: false
                }
            }
        );
    }

    #[test]
    fn test_parse_import_with_slash() {
        let cmd = parse_command("/import file.xlsx").unwrap();
        assert_eq!(
            cmd,
            Command::Import {
                action: ImportAction::File {
                    path: "file.xlsx".to_string(),
                    dry_run: false
                }
            }
        );
    }

    #[test]
    fn test_parse_import_dry_run() {
        let cmd = parse_command("import file.xlsx --dry-run").unwrap();
        assert_eq!(
            cmd,
            Command::Import {
                action: ImportAction::File {
                    path: "file.xlsx".to_string(),
                    dry_run: true
                }
            }
        );
    }

    #[test]
    fn test_parse_portfolio_show() {
        let cmd = parse_command("portfolio show").unwrap();
        assert_eq!(
            cmd,
            Command::Portfolio {
                action: PortfolioAction::Show {
                    filter: None,
                    as_of_date: None
                }
            }
        );
    }

    #[test]
    fn test_parse_portfolio_show_with_filter() {
        let cmd = parse_command("portfolio show --asset-type stock").unwrap();
        assert_eq!(
            cmd,
            Command::Portfolio {
                action: PortfolioAction::Show {
                    filter: Some("stock".to_string()),
                    as_of_date: None
                }
            }
        );
    }

    #[test]
    fn test_parse_portfolio_show_short_filter() {
        let cmd = parse_command("portfolio show -a fii").unwrap();
        assert_eq!(
            cmd,
            Command::Portfolio {
                action: PortfolioAction::Show {
                    filter: Some("fii".to_string()),
                    as_of_date: None
                }
            }
        );
    }

    #[test]
    fn test_parse_portfolio_show_with_date() {
        let cmd = parse_command("portfolio show --at 2024-06-15").unwrap();
        assert_eq!(
            cmd,
            Command::Portfolio {
                action: PortfolioAction::Show {
                    filter: None,
                    as_of_date: Some("2024-06-15".to_string())
                }
            }
        );
    }

    #[test]
    fn test_parse_portfolio_show_with_month_date() {
        let cmd = parse_command("portfolio show --at 2024-06").unwrap();
        assert_eq!(
            cmd,
            Command::Portfolio {
                action: PortfolioAction::Show {
                    filter: None,
                    as_of_date: Some("2024-06-30".to_string())
                }
            } // Last day of June
        );
    }

    #[test]
    fn test_parse_portfolio_show_with_year_date() {
        let cmd = parse_command("portfolio show --at 2024").unwrap();
        assert_eq!(
            cmd,
            Command::Portfolio {
                action: PortfolioAction::Show {
                    filter: None,
                    as_of_date: Some("2024-12-31".to_string())
                }
            } // December 31
        );
    }

    #[test]
    fn test_parse_portfolio_show_with_filter_and_date() {
        let cmd = parse_command("portfolio show -a fii --at 2024-06-15").unwrap();
        assert_eq!(
            cmd,
            Command::Portfolio {
                action: PortfolioAction::Show {
                    filter: Some("fii".to_string()),
                    as_of_date: Some("2024-06-15".to_string())
                }
            }
        );
    }

    #[test]
    fn test_parse_tax_report() {
        let cmd = parse_command("tax report 2024").unwrap();
        assert_eq!(
            cmd,
            Command::Tax {
                action: TaxAction::Report {
                    year: 2024,
                    export_csv: false
                }
            }
        );
    }

    #[test]
    fn test_parse_tax_report_with_export() {
        let cmd = parse_command("tax report 2024 --export").unwrap();
        assert_eq!(
            cmd,
            Command::Tax {
                action: TaxAction::Report {
                    year: 2024,
                    export_csv: true
                }
            }
        );
    }

    #[test]
    fn test_parse_tax_summary() {
        let cmd = parse_command("tax summary 2023").unwrap();
        assert_eq!(
            cmd,
            Command::Tax {
                action: TaxAction::Summary { year: 2023 }
            }
        );
    }

    #[test]
    fn test_parse_income_show() {
        let cmd = parse_command("income show").unwrap();
        assert_eq!(
            cmd,
            Command::Income {
                action: IncomeAction::Show { year: None }
            }
        );
    }

    #[test]
    fn test_parse_income_show_with_year() {
        let cmd = parse_command("income show 2024").unwrap();
        assert_eq!(
            cmd,
            Command::Income {
                action: IncomeAction::Show { year: Some(2024) }
            }
        );
    }

    #[test]
    fn test_parse_income_detail() {
        let cmd = parse_command("income detail").unwrap();
        assert_eq!(
            cmd,
            Command::Income {
                action: IncomeAction::Detail {
                    year: None,
                    asset: None
                }
            }
        );
    }

    #[test]
    fn test_parse_income_detail_with_asset() {
        let cmd = parse_command("income detail --asset XPLG11").unwrap();
        assert_eq!(
            cmd,
            Command::Income {
                action: IncomeAction::Detail {
                    year: None,
                    asset: Some("XPLG11".to_string())
                }
            }
        );
    }

    #[test]
    fn test_parse_income_detail_with_year_and_asset() {
        let cmd = parse_command("income detail 2024 --asset mxrf11").unwrap();
        assert_eq!(
            cmd,
            Command::Income {
                action: IncomeAction::Detail {
                    year: Some(2024),
                    asset: Some("MXRF11".to_string())
                }
            }
        );
    }

    #[test]
    fn test_parse_income_summary() {
        let cmd = parse_command("income summary 2025").unwrap();
        assert_eq!(
            cmd,
            Command::Income {
                action: IncomeAction::Summary { year: Some(2025) }
            }
        );
    }

    #[test]
    fn test_parse_income_summary_without_year() {
        let cmd = parse_command("income summary").unwrap();
        assert_eq!(
            cmd,
            Command::Income {
                action: IncomeAction::Summary { year: None }
            }
        );
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
            Command::Performance {
                action: PerformanceAction::Show {
                    period: "MTD".to_string()
                }
            }
        );
    }

    #[test]
    fn test_parse_performance_show_ytd() {
        let cmd = parse_command("performance show YTD").unwrap();
        assert_eq!(
            cmd,
            Command::Performance {
                action: PerformanceAction::Show {
                    period: "YTD".to_string()
                }
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

    #[test]
    fn test_tokenize_command_quotes() {
        let tokens = tokenize_command("assets add TEST --name 'My Fund' --type fii").unwrap();
        assert_eq!(
            tokens,
            vec!["assets", "add", "TEST", "--name", "My Fund", "--type", "fii"]
        );
    }

    #[test]
    fn test_tokenize_command_double_quotes() {
        let tokens = tokenize_command("assets add TEST --name \"My Fund\"").unwrap();
        assert_eq!(tokens, vec!["assets", "add", "TEST", "--name", "My Fund"]);
    }

    #[test]
    fn test_tokenize_command_unterminated_quote() {
        let err = tokenize_command("assets add TEST --name 'My Fund").unwrap_err();
        assert!(err.message.contains("Unterminated quote"));
    }
}
