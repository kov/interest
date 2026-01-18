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

        Commands::CashFlow { action } => match action {
            crate::cli::CashFlowCommands::Show { period } => Ok(Some(Command::CashFlow {
                action: cmd::CashFlowAction::Show {
                    period: period.clone().unwrap_or_else(|| "ALL".to_string()),
                },
            })),
            crate::cli::CashFlowCommands::Stats { period } => Ok(Some(Command::CashFlow {
                action: cmd::CashFlowAction::Stats {
                    period: period.clone().unwrap_or_else(|| "ALL".to_string()),
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

        Commands::Actions { action } => match action {
            crate::cli::ActionCommands::Rename { action } => {
                let mapped = match action {
                    crate::cli::RenameCommands::Add {
                        from,
                        to,
                        date,
                        notes,
                    } => cmd::ActionsAction::Rename {
                        action: cmd::RenameAction::Add {
                            from: from.clone(),
                            to: to.clone(),
                            date: date.clone(),
                            notes: notes.clone(),
                        },
                    },
                    crate::cli::RenameCommands::List { ticker } => cmd::ActionsAction::Rename {
                        action: cmd::RenameAction::List {
                            ticker: ticker.clone(),
                        },
                    },
                    crate::cli::RenameCommands::Remove { id } => cmd::ActionsAction::Rename {
                        action: cmd::RenameAction::Remove { id: *id },
                    },
                };
                Ok(Some(Command::Actions { action: mapped }))
            }
            crate::cli::ActionCommands::Split { action } => {
                let mapped = match action {
                    crate::cli::SplitCommands::Add {
                        ticker,
                        quantity_adjustment,
                        date,
                        notes,
                    } => cmd::ActionsAction::Split {
                        action: cmd::SplitAction::Add {
                            ticker: ticker.clone(),
                            quantity_adjustment: quantity_adjustment.clone(),
                            date: date.clone(),
                            notes: notes.clone(),
                        },
                    },
                    crate::cli::SplitCommands::List { ticker } => cmd::ActionsAction::Split {
                        action: cmd::SplitAction::List {
                            ticker: ticker.clone(),
                        },
                    },
                    crate::cli::SplitCommands::Remove { id } => cmd::ActionsAction::Split {
                        action: cmd::SplitAction::Remove { id: *id },
                    },
                };
                Ok(Some(Command::Actions { action: mapped }))
            }
            crate::cli::ActionCommands::Bonus { action } => {
                let mapped = match action {
                    crate::cli::BonusCommands::Add {
                        ticker,
                        quantity_adjustment,
                        date,
                        notes,
                    } => cmd::ActionsAction::Bonus {
                        action: cmd::BonusAction::Add {
                            ticker: ticker.clone(),
                            quantity_adjustment: quantity_adjustment.clone(),
                            date: date.clone(),
                            notes: notes.clone(),
                        },
                    },
                    crate::cli::BonusCommands::List { ticker } => cmd::ActionsAction::Bonus {
                        action: cmd::BonusAction::List {
                            ticker: ticker.clone(),
                        },
                    },
                    crate::cli::BonusCommands::Remove { id } => cmd::ActionsAction::Bonus {
                        action: cmd::BonusAction::Remove { id: *id },
                    },
                };
                Ok(Some(Command::Actions { action: mapped }))
            }
            crate::cli::ActionCommands::Spinoff { action } => {
                let mapped = match action {
                    crate::cli::ExchangeCommands::Add {
                        from,
                        to,
                        date,
                        quantity,
                        allocated_cost,
                        cash,
                        notes,
                    } => cmd::ActionsAction::Spinoff {
                        action: cmd::ExchangeAction::Add {
                            from: from.clone(),
                            to: to.clone(),
                            date: date.clone(),
                            quantity: quantity.clone(),
                            allocated_cost: allocated_cost.clone(),
                            cash: cash.clone(),
                            notes: notes.clone(),
                        },
                    },
                    crate::cli::ExchangeCommands::List { ticker } => cmd::ActionsAction::Spinoff {
                        action: cmd::ExchangeAction::List {
                            ticker: ticker.clone(),
                        },
                    },
                    crate::cli::ExchangeCommands::Remove { id } => cmd::ActionsAction::Spinoff {
                        action: cmd::ExchangeAction::Remove { id: *id },
                    },
                };
                Ok(Some(Command::Actions { action: mapped }))
            }
            crate::cli::ActionCommands::Merger { action } => {
                let mapped = match action {
                    crate::cli::ExchangeCommands::Add {
                        from,
                        to,
                        date,
                        quantity,
                        allocated_cost,
                        cash,
                        notes,
                    } => cmd::ActionsAction::Merger {
                        action: cmd::ExchangeAction::Add {
                            from: from.clone(),
                            to: to.clone(),
                            date: date.clone(),
                            quantity: quantity.clone(),
                            allocated_cost: allocated_cost.clone(),
                            cash: cash.clone(),
                            notes: notes.clone(),
                        },
                    },
                    crate::cli::ExchangeCommands::List { ticker } => cmd::ActionsAction::Merger {
                        action: cmd::ExchangeAction::List {
                            ticker: ticker.clone(),
                        },
                    },
                    crate::cli::ExchangeCommands::Remove { id } => cmd::ActionsAction::Merger {
                        action: cmd::ExchangeAction::Remove { id: *id },
                    },
                };
                Ok(Some(Command::Actions { action: mapped }))
            }
            crate::cli::ActionCommands::Apply { ticker } => Ok(Some(Command::Actions {
                action: cmd::ActionsAction::Apply {
                    ticker: ticker.clone(),
                },
            })),
        },
        Commands::Inconsistencies { action } => match action {
            crate::cli::InconsistenciesCommands::List {
                open: _,
                all,
                status,
                issue_type,
                asset,
            } => {
                let status = if let Some(status) = status.clone() {
                    Some(status)
                } else if *all {
                    Some("ALL".to_string())
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
        Commands::Tickers { action } => match action {
            crate::cli::TickersCommands::Refresh { force } => Ok(Some(Command::Tickers {
                action: cmd::TickersAction::Refresh { force: *force },
            })),
            crate::cli::TickersCommands::Status => Ok(Some(Command::Tickers {
                action: cmd::TickersAction::Status,
            })),
            crate::cli::TickersCommands::ListUnknown => Ok(Some(Command::Tickers {
                action: cmd::TickersAction::ListUnknown,
            })),
            crate::cli::TickersCommands::Resolve { ticker, asset_type } => {
                if ticker.is_some() && asset_type.is_none() {
                    anyhow::bail!("tickers resolve requires --type when a ticker is provided");
                }
                Ok(Some(Command::Tickers {
                    action: cmd::TickersAction::Resolve {
                        ticker: ticker.clone(),
                        asset_type: asset_type.clone(),
                    },
                }))
            }
        },
        Commands::Assets { action } => match action {
            crate::cli::AssetsCommands::List { asset_type } => Ok(Some(Command::Assets {
                action: cmd::AssetsAction::List {
                    asset_type: asset_type.clone(),
                },
            })),
            crate::cli::AssetsCommands::Show { ticker } => Ok(Some(Command::Assets {
                action: cmd::AssetsAction::Show {
                    ticker: ticker.clone(),
                },
            })),
            crate::cli::AssetsCommands::Add {
                ticker,
                asset_type,
                name,
            } => Ok(Some(Command::Assets {
                action: cmd::AssetsAction::Add {
                    ticker: ticker.clone(),
                    asset_type: asset_type.clone(),
                    name: name.clone(),
                },
            })),
            crate::cli::AssetsCommands::SetType { ticker, asset_type } => {
                Ok(Some(Command::Assets {
                    action: cmd::AssetsAction::SetType {
                        ticker: ticker.clone(),
                        asset_type: asset_type.clone(),
                    },
                }))
            }
            crate::cli::AssetsCommands::SetName { ticker, name } => Ok(Some(Command::Assets {
                action: cmd::AssetsAction::SetName {
                    ticker: ticker.clone(),
                    name: name.clone(),
                },
            })),
            crate::cli::AssetsCommands::Rename {
                old_ticker,
                new_ticker,
            } => Ok(Some(Command::Assets {
                action: cmd::AssetsAction::Rename {
                    old_ticker: old_ticker.clone(),
                    new_ticker: new_ticker.clone(),
                },
            })),
            crate::cli::AssetsCommands::Remove { ticker } => Ok(Some(Command::Assets {
                action: cmd::AssetsAction::Remove {
                    ticker: ticker.clone(),
                },
            })),
            crate::cli::AssetsCommands::SyncMaisRetorno {
                asset_type,
                dry_run,
            } => Ok(Some(Command::Assets {
                action: cmd::AssetsAction::SyncMaisRetorno {
                    asset_type: asset_type.clone(),
                    dry_run: *dry_run,
                },
            })),
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
