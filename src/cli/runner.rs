use anyhow::Result;

use crate::cli::Commands;
use crate::commands::{self as cmd, Command};

/// Convert clap `Commands` into the internal `commands::Command` when possible.
/// Returns Ok(Some(Command)) when conversion succeeds, Ok(None) when the CLI
/// command requires special handling (e.g., `prices update`), and Err on parse
/// errors (e.g., invalid date formats).
pub fn to_internal_command(c: &Commands) -> Result<Option<Command>> {
    match c {
        // Import is handled specially by `main::handle_import` (keeps existing behavior)
        Commands::Import { .. } => Ok(None),

        Commands::Portfolio { action } => match action {
            crate::cli::PortfolioCommands::Show { asset_type, at } => {
                let as_of_date = match at.as_ref() {
                    Some(d) => Some(cmd::parse_flexible_date(d)?),
                    None => None,
                };
                Ok(Some(Command::Portfolio {
                    action: cmd::PortfolioAction::Show {
                        filter: asset_type.clone(),
                        as_of_date,
                    },
                }))
            }
        },

        Commands::Performance { action } => match action {
            crate::cli::PerformanceCommands::Show { period } => Ok(Some(Command::Performance {
                action: cmd::PerformanceAction::Show {
                    period: period.clone(),
                },
            })),
        },

        Commands::Tax { action } => match action {
            crate::cli::TaxCommands::Report { year, export } => Ok(Some(Command::Tax {
                action: cmd::TaxAction::Report {
                    year: *year,
                    export_csv: *export,
                },
            })),
            crate::cli::TaxCommands::Summary { year } => Ok(Some(Command::Tax {
                action: cmd::TaxAction::Summary { year: *year },
            })),
            crate::cli::TaxCommands::Calculate { .. } => Ok(None),
        },

        Commands::Income { action } => match action {
            crate::cli::IncomeCommands::Show { year } => Ok(Some(Command::Income {
                action: cmd::IncomeAction::Show { year: *year },
            })),
            crate::cli::IncomeCommands::Detail { year, asset } => Ok(Some(Command::Income {
                action: cmd::IncomeAction::Detail {
                    year: *year,
                    asset: asset.clone(),
                },
            })),
            crate::cli::IncomeCommands::Summary { year } => Ok(Some(Command::Income {
                action: cmd::IncomeAction::Summary { year: *year },
            })),
        },

        Commands::Actions { .. } => Ok(None),
        Commands::Inconsistencies { action } => match action {
            crate::cli::InconsistenciesCommands::List {
                open,
                all,
                status,
                issue_type,
                asset,
            } => {
                let status = if let Some(status) = status.clone() {
                    Some(status)
                } else if *all {
                    Some("ALL".to_string())
                } else if *open {
                    Some("OPEN".to_string())
                } else {
                    Some("OPEN".to_string())
                };

                // Map to internal InconsistenciesAction::List
                Ok(Some(Command::Inconsistencies {
                    action: cmd::InconsistenciesAction::List {
                        status,
                        issue_type: issue_type.clone(),
                        asset: asset.clone(),
                    },
                }))
            }
            crate::cli::InconsistenciesCommands::Show { id } => {
                Ok(Some(Command::Inconsistencies {
                    action: cmd::InconsistenciesAction::Show { id: *id },
                }))
            }
            crate::cli::InconsistenciesCommands::Resolve {
                id,
                set,
                json_payload,
            } => {
                let parsed_set = set
                    .iter()
                    .filter_map(|s| {
                        let mut parts = s.splitn(2, '=');
                        if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                            Some((k.to_string(), v.to_string()))
                        } else {
                            None
                        }
                    })
                    .collect();

                Ok(Some(Command::Inconsistencies {
                    action: cmd::InconsistenciesAction::Resolve {
                        id: *id,
                        set: parsed_set,
                        json: json_payload.clone(),
                    },
                }))
            }
            crate::cli::InconsistenciesCommands::Ignore { id, reason } => {
                Ok(Some(Command::Inconsistencies {
                    action: cmd::InconsistenciesAction::Ignore {
                        id: *id,
                        reason: reason.clone(),
                    },
                }))
            }
        },

        Commands::ProcessTerms => Ok(None),
        Commands::Transactions { .. } => Ok(None),
        Commands::Inspect { .. } => Ok(None),
        Commands::ImportIrpf { .. } => Ok(None),
        Commands::Interactive => Ok(None),
        Commands::Prices { action } => match action {
            crate::cli::PriceCommands::History {
                ticker: _,
                from: _,
                to: _,
            } => Ok(None),
            crate::cli::PriceCommands::Update => Ok(None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_portfolio_show_with_date() {
        let cmd = Commands::Portfolio {
            action: crate::cli::PortfolioCommands::Show {
                asset_type: Some("STOCK".to_string()),
                at: Some("2025-05".to_string()),
            },
        };

        let converted = to_internal_command(&cmd).expect("conversion failed");
        match converted {
            Some(Command::Portfolio { action }) => match action {
                crate::commands::PortfolioAction::Show { filter, as_of_date } => {
                    assert_eq!(filter, Some("STOCK".to_string()));
                    assert_eq!(as_of_date, Some("2025-05-31".to_string()));
                }
            },
            other => panic!("unexpected converted result: {:?}", other),
        }
    }
}
