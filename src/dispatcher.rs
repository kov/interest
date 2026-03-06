//! Command dispatcher that routes clap Commands to the appropriate handlers.
//!
//! This module provides a unified interface for command routing, with clap
//! as the single source of truth for command definitions.

pub mod performance;
use performance::dispatch_performance;
mod actions;
mod assets;
mod cashflow;
pub mod imports;
pub mod imports_helpers;
pub mod income;
mod inconsistencies;
mod inspect;
mod irpf;
mod portfolio;
mod prices;
mod prices_ui;
mod terms;
mod tickers;
mod transactions;
use crate::utils::format_currency;
use crate::{db, formatters, options, tax};
use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;
use tracing::info;

/// Route a parsed command to its handler
pub async fn dispatch_command(
    command: &crate::cli::Commands,
    options: options::OutputOptions,
) -> Result<()> {
    use crate::cli::Commands;

    match command {
        Commands::Import {
            file,
            dry_run,
            force_reimport,
        } => imports::dispatch_import(file, *dry_run, *force_reimport, options).await,
        Commands::ImportIrpf {
            file,
            year,
            dry_run,
        } => irpf::dispatch_irpf_import(file, *year, *dry_run, options).await,
        Commands::Portfolio { action } => portfolio::dispatch_portfolio(action, options).await,
        Commands::Performance { action } => dispatch_performance(action, options).await,
        Commands::CashFlow { action } => cashflow::dispatch_cashflow(action, options).await,
        Commands::Tax { action } => dispatch_tax(action, options).await,
        Commands::Income { action } => income::dispatch_income(action, options).await,
        Commands::Actions { action } => actions::dispatch_actions(action, options).await,
        Commands::Prices { action } => prices::dispatch_prices(action, options).await,
        Commands::Transactions { action } => {
            transactions::dispatch_transactions(action, options).await
        }
        Commands::Inspect { file, full, column } => {
            inspect::dispatch_inspect(file, *full, *column).await
        }
        Commands::ProcessTerms => terms::dispatch_process_terms().await,
        Commands::Inconsistencies { action } => {
            inconsistencies::dispatch_inconsistencies(action, options).await
        }
        Commands::Tickers { action } => tickers::dispatch_tickers(action, options).await,
        Commands::Assets { action } => assets::dispatch_assets(action, options).await,
        Commands::Interactive => {
            // This should never be reached since main.rs handles Interactive separately
            Err(anyhow::anyhow!(
                "Interactive mode should be handled by main.rs"
            ))
        }
        Commands::Chat => {
            // This should never be reached since main.rs handles Chat separately
            Err(anyhow::anyhow!("Chat mode should be handled by main.rs"))
        }
        Commands::Completions { shell, no_install } => {
            dispatch_completions(*shell, *no_install, options)
        }
        Commands::Complete { args } => dispatch_dynamic_complete(args),
        Commands::Privacy { .. } => Err(anyhow::anyhow!(
            "Privacy mode is only supported in interactive mode. Use --privacy for CLI commands."
        )),
    }
}

async fn dispatch_tax(
    action: &crate::cli::TaxCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::TaxCommands::Report { year, export } => {
            dispatch_tax_report(*year, *export, options).await
        }
        crate::cli::TaxCommands::Summary { year } => dispatch_tax_summary(*year, options).await,
        crate::cli::TaxCommands::Calculate { month } => {
            dispatch_tax_calculate(month, options).await
        }
    }
}

async fn dispatch_tax_report(
    year: i32,
    export_csv: bool,
    options: options::OutputOptions,
) -> Result<()> {
    info!("Generating IRPF annual report for {}", year);

    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let report = if options.is_json() {
        tax::generate_annual_report_with_progress(&conn, year, |_ev| {})?
    } else {
        let mut printer = TaxProgressPrinter::new(&options);
        tax::generate_annual_report_with_progress(&conn, year, |ev| printer.on_event(ev))?
    };

    let income_summary = formatters::tax::build_income_summary(&conn, year)?;

    let output =
        formatters::tax::format_tax_report(&report, &income_summary, year, options.clone())?;
    options.writer().writeln(&output)?;

    if export_csv {
        let csv_content = tax::irpf::export_to_csv(&report);
        let csv_path = format!("irpf_report_{}.csv", year);
        std::fs::write(&csv_path, csv_content)?;

        options.writer().writeln(&format!(
            "{} Report exported to: {}\n",
            "✓".green().bold(),
            csv_path
        ))?;
    }

    Ok(())
}

async fn dispatch_tax_summary(year: i32, options: options::OutputOptions) -> Result<()> {
    info!("Generating tax summary for {}", year);

    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let report = if options.is_json() {
        tax::generate_annual_report_with_progress(&conn, year, |_ev| {})?
    } else {
        let mut printer = TaxProgressPrinter::new(&options);
        tax::generate_annual_report_with_progress(&conn, year, |ev| printer.on_event(ev))?
    };

    let output = formatters::tax::format_tax_summary(&report, year, options.clone())?;
    options.writer().writeln(&output)?;

    Ok(())
}

async fn dispatch_tax_calculate(month_str: &str, options: options::OutputOptions) -> Result<()> {
    use anyhow::Context;
    use colored::Colorize;

    tracing::info!("Calculating swing trade tax for {}", month_str);

    // Parse month string (MM/YYYY)
    let parts: Vec<&str> = month_str.split('/').collect();
    if parts.len() != 2 {
        return Err(anyhow::anyhow!(
            "Invalid month format. Use MM/YYYY (e.g., 01/2025)"
        ));
    }

    let month: u32 = parts[0].parse().context("Invalid month number")?;
    let year: i32 = parts[1].parse().context("Invalid year")?;

    if !(1..=12).contains(&month) {
        return Err(anyhow::anyhow!("Month must be between 01 and 12"));
    }

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Calculate monthly tax; carryforward map stays empty for one-off calculation
    let mut carryforward = std::collections::HashMap::new();
    let calculations = tax::calculate_monthly_tax(&conn, year, month, &mut carryforward)?;

    if calculations.is_empty() {
        options.writer().writeln(&format!(
            "\n{} No sales found for {}/{}\n",
            "ℹ".blue().bold(),
            month,
            year
        ))?;
        return Ok(());
    }

    options.writer().writeln(&format!(
        "\n{} Swing Trade Tax Calculation - {}/{}\n",
        "💰".cyan().bold(),
        month,
        year
    ))?;

    // Display results by tax category
    for calc in &calculations {
        options.writer().writeln(&format!(
            "{} {}",
            "Tax Category:".bold(),
            calc.category.display_name()
        ))?;
        options.writer().writeln(&format!(
            "  Total Sales:      {}",
            format_currency(calc.total_sales, &options).cyan()
        ))?;
        options.writer().writeln(&format!(
            "  Total Cost Basis: {}",
            format_currency(calc.total_cost_basis, &options).cyan()
        ))?;
        options.writer().writeln(&format!(
            "  Gross Profit:     {}",
            format_currency(calc.total_profit, &options).green()
        ))?;
        options.writer().writeln(&format!(
            "  Gross Loss:       {}",
            format_currency(calc.total_loss, &options).red()
        ))?;

        let net_str = if calc.net_profit >= rust_decimal::Decimal::ZERO {
            format_currency(calc.net_profit, &options).green()
        } else {
            format_currency(calc.net_profit, &options).red()
        };
        options
            .writer()
            .writeln(&format!("  Net P&L:          {}", net_str))?;

        // Show loss offset if applied
        if calc.loss_offset_applied > rust_decimal::Decimal::ZERO {
            options.writer().writeln(&format!(
                "  Loss Offset:      {} (from previous months)",
                format_currency(calc.loss_offset_applied, &options).cyan()
            ))?;
            options.writer().writeln(&format!(
                "  After Loss Offset: {}",
                format_currency(calc.profit_after_loss_offset, &options).green()
            ))?;
        }

        if calc.exemption_applied > rust_decimal::Decimal::ZERO {
            options.writer().writeln(&format!(
                "  Exemption:        {} (sales under R$20.000)",
                format_currency(calc.exemption_applied, &options)
                    .yellow()
                    .bold()
            ))?;
        }

        if calc.taxable_amount > rust_decimal::Decimal::ZERO {
            options.writer().writeln(&format!(
                "  Taxable Amount:   {}",
                format_currency(calc.taxable_amount, &options).yellow()
            ))?;
            let tax_rate_pct = calc.tax_rate * rust_decimal::Decimal::from(100);
            options.writer().writeln(&format!(
                "  Tax Rate:         {}",
                format!("{:.0}%", tax_rate_pct).yellow()
            ))?;
            options.writer().writeln(&format!(
                "  {} {}",
                "Tax Due:".bold(),
                format_currency(calc.tax_due, &options).red().bold()
            ))?;
        } else if calc.profit_after_loss_offset < rust_decimal::Decimal::ZERO {
            options.writer().writeln(&format!(
                "  {} Loss to carry forward",
                format_currency(calc.net_profit.abs(), &options)
                    .yellow()
                    .bold()
            ))?;
        } else {
            options.writer().writeln(&format!(
                "  {} No tax due (exempt)",
                "Tax Due:".bold().green()
            ))?;
        }

        options.writer().writeln("")?;
    }

    // Summary
    let total_tax: rust_decimal::Decimal = calculations.iter().map(|c| c.tax_due).sum();

    if total_tax > rust_decimal::Decimal::ZERO {
        options.writer().writeln(&format!(
            "{} Total Tax Due for {}/{}: {}\n",
            "📋".cyan().bold(),
            month,
            year,
            format_currency(total_tax, &options).red().bold()
        ))?;

        // Generate DARF payments
        let darf_payments = tax::generate_darf_payments(calculations, year, month)?;

        if !darf_payments.is_empty() {
            options
                .writer()
                .writeln(&format!("{} DARF Payments:\n", "💳".cyan().bold()))?;

            for payment in &darf_payments {
                options.writer().writeln(&format!(
                    "  {} Code {}: {}",
                    "DARF".yellow().bold(),
                    payment.darf_code,
                    payment.description
                ))?;
                options.writer().writeln(&format!(
                    "    Amount:   {}",
                    format_currency(payment.tax_due, &options).red()
                ))?;
                options.writer().writeln(&format!(
                    "    Due Date: {}",
                    payment.due_date.format("%d/%m/%Y").to_string().yellow()
                ))?;
                options.writer().writeln("")?;
            }

            options.writer().writeln(&format!(
                "{} Payment due by {}\n",
                "⏰".yellow(),
                darf_payments[0].due_date.format("%d/%m/%Y")
            ))?;
        }
    }

    Ok(())
}

// Snapshot commands are intentionally internal-only; no public dispatcher.

struct TaxProgressPrinter {
    printer: crate::ui::progress::ProgressPrinter,
    in_progress: bool,
    from_year: Option<i32>,
    target_year: Option<i32>,
    total_years: usize,
    completed_years: usize,
}

impl TaxProgressPrinter {
    fn new(options: &options::OutputOptions) -> Self {
        Self {
            printer: crate::ui::progress::ProgressPrinter::new(options),
            in_progress: false,
            from_year: None,
            target_year: None,
            total_years: 0,
            completed_years: 0,
        }
    }

    fn on_event(&mut self, event: tax::ReportProgress) {
        match event {
            tax::ReportProgress::Start { target_year, .. } => {
                self.target_year = Some(target_year);
            }
            tax::ReportProgress::RecomputeStart { from_year } => {
                self.from_year = Some(from_year);
                self.in_progress = true;
                self.completed_years = 0;
                self.total_years = self
                    .target_year
                    .map(|t| (t - from_year + 1).max(1) as usize)
                    .unwrap_or(1);
                self.printer
                    .handle_event(&crate::ui::progress::ProgressEvent::Recomputing {
                        what: format!("snapshots (starting {})", from_year),
                        progress: Some(crate::ui::progress::ProgressData {
                            current: self.completed_years,
                            total: Some(self.total_years),
                        }),
                    });
            }
            tax::ReportProgress::RecomputedYear { year } => {
                if self.in_progress {
                    self.completed_years = (self.completed_years + 1).min(self.total_years);
                    let from = self.from_year.unwrap_or(year);
                    if Some(year) == self.target_year {
                        self.printer
                            .handle_event(&crate::ui::progress::ProgressEvent::Success {
                                message: format!("Snapshots updated {}→{}", from, year),
                            });
                        self.in_progress = false;
                    } else {
                        self.printer.handle_event(
                            &crate::ui::progress::ProgressEvent::Recomputing {
                                what: format!("snapshots (year {})", year),
                                progress: Some(crate::ui::progress::ProgressData {
                                    current: self.completed_years,
                                    total: Some(self.total_years),
                                }),
                            },
                        );
                    }
                }
            }
            tax::ReportProgress::TargetCacheHit { year } => {
                self.printer
                    .handle_event(&crate::ui::progress::ProgressEvent::Success {
                        message: format!("Cache hit for {}; using cached carry", year),
                    });
            }
            _ => {}
        }
    }
}

/// Generate shell completion scripts
fn dispatch_completions(
    shell: Option<crate::cli::Shell>,
    no_install: bool,
    options: options::OutputOptions,
) -> Result<()> {
    use clap::CommandFactory;
    use clap_complete::{generate, shells};

    // Reject --json flag for completions since the output is a shell script
    if options.output_mode == options::OutputMode::Json {
        return Err(anyhow::anyhow!(
            "JSON output is not supported for shell completions. \
             The completions command generates shell scripts that must be written to stdout directly."
        ));
    }

    // Auto-detect shell if not specified
    let shell = match shell {
        Some(s) => s,
        None => detect_shell()?,
    };

    let mut cmd = crate::cli::Cli::command();
    let bin_name = cmd
        .get_bin_name()
        .unwrap_or_else(|| cmd.get_name())
        .to_string();

    // Determine if we should use interactive installation
    let is_tty = std::io::stdout().is_terminal();
    let should_install = is_tty && !no_install;

    if should_install {
        // Interactive installation mode
        install_completion_interactively(shell, &mut cmd, &bin_name)
    } else {
        // Print to stdout (for piping or when --no-install is specified)
        match shell {
            crate::cli::Shell::Bash => {
                // Generate base Bash completions
                let mut buffer = Vec::new();
                generate(shells::Bash, &mut cmd, &bin_name, &mut buffer);

                // Add dynamic completion support
                let completion_script = String::from_utf8(buffer)?;
                let enhanced_script = add_bash_dynamic_completions(&completion_script, &bin_name);
                print!("{}", enhanced_script);
            }
            crate::cli::Shell::Fish => {
                // Generate base Fish completions
                let mut buffer = Vec::new();
                generate(shells::Fish, &mut cmd, &bin_name, &mut buffer);

                // Add dynamic completion support
                let completion_script = String::from_utf8(buffer)?;
                let enhanced_script = add_fish_dynamic_completions(&completion_script, &bin_name);
                print!("{}", enhanced_script);
            }
            crate::cli::Shell::Zsh => {
                // Generate base Zsh completions
                let mut buffer = Vec::new();
                generate(shells::Zsh, &mut cmd, &bin_name, &mut buffer);

                // Add dynamic completion support
                let completion_script = String::from_utf8(buffer)?;
                let enhanced_script = add_zsh_dynamic_completions(&completion_script, &bin_name);
                print!("{}", enhanced_script);
            }
        }
        Ok(())
    }
}

/// Detect the current shell from the environment
fn detect_shell() -> Result<crate::cli::Shell> {
    use std::env;

    let shell = env::var("SHELL")
        .map_err(|_| anyhow::anyhow!("Could not detect shell from $SHELL environment variable. Please specify the shell explicitly."))?;

    // Extract the shell name from the path (e.g., /bin/bash -> bash)
    let shell_name = shell.rsplit('/').next().unwrap_or(&shell);

    match shell_name {
        "bash" => Ok(crate::cli::Shell::Bash),
        "fish" => Ok(crate::cli::Shell::Fish),
        "zsh" => Ok(crate::cli::Shell::Zsh),
        _ => Err(anyhow::anyhow!(
            "Unsupported shell '{}'. Supported shells: bash, fish, zsh. Please specify the shell explicitly.",
            shell_name
        )),
    }
}

/// Install completion script interactively
fn install_completion_interactively(
    shell: crate::cli::Shell,
    cmd: &mut clap::Command,
    bin_name: &str,
) -> Result<()> {
    use clap_complete::{generate, shells};
    use std::fs;
    use std::io::{self, Write};

    // Determine the installation path based on shell
    let install_path = get_completion_install_path(shell)?;

    // Ask user for confirmation
    eprint!(
        "Install {} completion to {}? [Y/n] ",
        shell_name(shell),
        install_path.display()
    );
    io::stderr().flush()?;

    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    let response = response.trim().to_lowercase();

    if !response.is_empty() && response != "y" && response != "yes" {
        eprintln!("Installation cancelled.");
        return Ok(());
    }

    // Create parent directory if it doesn't exist
    if let Some(parent) = install_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Generate completion script to a buffer
    let mut buffer = Vec::new();
    match shell {
        crate::cli::Shell::Bash => {
            // Generate base Bash completions
            generate(shells::Bash, cmd, bin_name, &mut buffer);

            // Add dynamic completion support
            let completion_script = String::from_utf8(buffer)?;
            let enhanced_script = add_bash_dynamic_completions(&completion_script, bin_name);
            buffer = enhanced_script.into_bytes();
        }
        crate::cli::Shell::Fish => {
            // Generate base Fish completions
            generate(shells::Fish, cmd, bin_name, &mut buffer);

            // Add dynamic completion support
            let completion_script = String::from_utf8(buffer)?;
            let enhanced_script = add_fish_dynamic_completions(&completion_script, bin_name);
            buffer = enhanced_script.into_bytes();
        }
        crate::cli::Shell::Zsh => {
            // Generate base Zsh completions
            generate(shells::Zsh, cmd, bin_name, &mut buffer);

            // Add dynamic completion support
            let completion_script = String::from_utf8(buffer)?;
            let enhanced_script = add_zsh_dynamic_completions(&completion_script, bin_name);
            buffer = enhanced_script.into_bytes();
        }
    }

    // Write to file
    fs::write(&install_path, buffer)?;

    eprintln!("✓ Completion installed to {}", install_path.display());
    eprintln!("\nTo activate completions:");
    match shell {
        crate::cli::Shell::Bash => {
            eprintln!("  source {}", install_path.display());
            eprintln!("Or restart your shell.");
        }
        crate::cli::Shell::Fish => {
            eprintln!("  Restart your shell or run: source ~/.config/fish/config.fish");
        }
        crate::cli::Shell::Zsh => {
            eprintln!("  Restart your shell or run: source ~/.zshrc");
        }
    }

    Ok(())
}

/// Add dynamic completion support to Fish completion script
fn add_fish_dynamic_completions(script: &str, bin_name: &str) -> String {
    let mut result = script.to_string();

    // Add dynamic completions for specific options
    let dynamic_completions = format!(
        r#"
# Dynamic completions for --asset-type
complete -c {bin} -n "__fish_interest_using_subcommand portfolio; and __fish_seen_subcommand_from show" -l asset-type -a "({bin} complete (commandline -opc))" -d 'Asset type'

# Dynamic completions for --at
complete -c {bin} -n "__fish_interest_using_subcommand portfolio; and __fish_seen_subcommand_from show" -l at -a "({bin} complete (commandline -opc))" -d 'Date or year'
"#,
        bin = bin_name
    );

    result.push_str(&dynamic_completions);
    result
}

/// Add dynamic completion support to Bash completion script
fn add_bash_dynamic_completions(script: &str, bin_name: &str) -> String {
    let mut result = script.to_string();

    // Add helper function for dynamic completions
    let dynamic_completions = format!(
        r#"

# Dynamic completion helper function
__{bin}_dynamic_complete() {{
    local cur prev_word cmd_line
    cur="${{COMP_WORDS[$COMP_CWORD]}}"
    
    # Get the full command line building from COMP_WORDS
    cmd_line=("${{COMP_WORDS[@]}}")
    
    # Call interest complete with the command line
    COMPREPLY=($(compgen -W "$({bin} complete "${{cmd_line[@]}}")" -- "$cur"))
}}

# Dynamic completions for portfolio show --asset-type
__{bin}_asset_type_complete() {{
    local cur="${{COMP_WORDS[$COMP_CWORD]}}"
    if [[ "${{#COMP_WORDS[@]}}" -gt 2 ]] && [[ "${{COMP_WORDS[1]}}" == "portfolio" ]] && [[ "${{COMP_WORDS[2]}}" == "show" ]]; then
        COMPREPLY=($(compgen -W "$({bin} complete "${{COMP_WORDS[@]}}")" -- "$cur"))
    fi
}}

# Dynamic completions for portfolio show --at
__{bin}_at_complete() {{
    local cur="${{COMP_WORDS[$COMP_CWORD]}}"
    if [[ "${{#COMP_WORDS[@]}}" -gt 2 ]] && [[ "${{COMP_WORDS[1]}}" == "portfolio" ]] && [[ "${{COMP_WORDS[2]}}" == "show" ]]; then
        COMPREPLY=($(compgen -W "$({bin} complete "${{COMP_WORDS[@]}}")" -- "$cur"))
    fi
}}
"#,
        bin = bin_name
    );

    result.push_str(&dynamic_completions);
    result
}

/// Add dynamic completion support to Zsh completion script
fn add_zsh_dynamic_completions(script: &str, bin_name: &str) -> String {
    let mut result = script.to_string();

    // Add helper functions for dynamic completions
    let dynamic_completions = format!(
        r#"

# Dynamic completion helper function for zsh
(( $+functions[__{bin}_complete] )) || __{bin}_complete() {{
    local -a values
    values=($({bin} complete "${{words[@]}}"))
    compadd -a values
}}

# Dynamic completions for asset types
(( $+functions[__{bin}_asset_types] )) || __{bin}_asset_types() {{
    __{bin}_complete
}}

# Dynamic completions for years
(( $+functions[__{bin}_years] )) || __{bin}_years() {{
    __{bin}_complete
}}
"#,
        bin = bin_name
    );

    result.push_str(&dynamic_completions);
    result
}

/// Get the standard installation path for completion scripts
fn get_completion_install_path(shell: crate::cli::Shell) -> Result<std::path::PathBuf> {
    use std::env;
    use std::path::PathBuf;

    match shell {
        crate::cli::Shell::Bash => {
            // Try XDG_DATA_HOME first, then fall back to ~/.local/share
            let data_home = env::var("XDG_DATA_HOME")
                .ok()
                .map(PathBuf::from)
                .or_else(|| {
                    env::var("HOME")
                        .ok()
                        .map(|h| PathBuf::from(h).join(".local/share"))
                });

            if let Some(base) = data_home {
                Ok(base.join("bash-completion/completions/interest"))
            } else {
                Err(anyhow::anyhow!(
                    "Could not determine home directory for bash completion installation"
                ))
            }
        }
        crate::cli::Shell::Fish => {
            // Respect XDG_CONFIG_HOME if set, otherwise use ~/.config
            let config_home = env::var("XDG_CONFIG_HOME")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| env::var("HOME").ok().map(|h| format!("{}/.config", h)))
                .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
            Ok(PathBuf::from(config_home).join("fish/completions/interest.fish"))
        }
        crate::cli::Shell::Zsh => {
            // Use ~/.zsh/completions as recommended in README
            let home = env::var("HOME")
                .map_err(|_| anyhow::anyhow!("Could not determine home directory"))?;
            Ok(PathBuf::from(home).join(".zsh/completions/_interest"))
        }
    }
}

/// Get human-readable shell name
fn shell_name(shell: crate::cli::Shell) -> &'static str {
    match shell {
        crate::cli::Shell::Bash => "bash",
        crate::cli::Shell::Fish => "fish",
        crate::cli::Shell::Zsh => "zsh",
    }
}

/// Handle dynamic completion requests
fn dispatch_dynamic_complete(args: &[String]) -> Result<()> {
    // Parse the command line to understand what we're completing
    // args contains the partial command line being completed

    if args.is_empty() {
        return Ok(());
    }

    // Try to find what option we're completing for
    let completing_for = args.iter().rev().find(|arg| arg.starts_with("--"));

    match completing_for.map(|s| s.as_str()) {
        Some("--asset-type") | Some("-a") => {
            // Provide asset type completions
            println!("STOCK");
            println!("FII");
            println!("FIAGRO");
            println!("FI_INFRA");
        }
        Some("--at") => {
            // Provide year completions from database
            if let Ok(years) = get_available_years() {
                for year in years {
                    println!("{}", year);
                }
            }
        }
        Some("--ticker") | Some("ticker") if args.contains(&"assets".to_string()) => {
            // Provide ticker completions from database
            if let Ok(tickers) = get_available_tickers() {
                for ticker in tickers {
                    println!("{}", ticker);
                }
            }
        }
        _ => {
            // No specific completions for this context
        }
    }

    Ok(())
}

/// Get available years from transactions in the database
fn get_available_years() -> Result<Vec<i32>> {
    let conn = db::open_db_read_only(None)?;

    let mut stmt = conn.prepare(
        "SELECT DISTINCT strftime('%Y', trade_date) as year 
         FROM transactions 
         WHERE trade_date IS NOT NULL 
         ORDER BY year DESC",
    )?;

    let years = stmt
        .query_map([], |row| {
            let year_str: String = row.get(0)?;
            year_str
                .parse::<i32>()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        })?
        .collect::<Result<Vec<i32>, _>>()?;

    Ok(years)
}

/// Get available tickers from the database
fn get_available_tickers() -> Result<Vec<String>> {
    let conn = db::open_db_read_only(None)?;

    let mut stmt = conn.prepare("SELECT DISTINCT ticker FROM assets ORDER BY ticker")?;

    let tickers = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;

    Ok(tickers)
}

// Tests removed - dispatcher now works with clap Commands
// Integration tests in tests/ directory provide coverage
