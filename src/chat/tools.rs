//! Tool registry for Interest functionality.
//!
//! Maps Interest commands to OpenAI-style tool definitions.

use anyhow::Result;
use serde_json::json;

use crate::chat::approvals::PolicyGroup;
use crate::chat::llm::{FunctionDefinition, ToolDefinition};
use crate::options::{OutputMode, OutputOptions, OutputTarget, PrivacyMode};

/// Tool metadata
pub struct Tool {
    pub name: String,
    pub description: String,
    pub policy_group: PolicyGroup,
    pub parameters_schema: serde_json::Value,
}

impl Tool {
    /// Convert to OpenAI tool definition
    pub fn to_definition(&self) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: self.name.clone(),
                description: self.description.clone(),
                parameters: self.parameters_schema.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOutputMode {
    Capture,
    Present,
    CaptureAndPresent,
}

impl ToolOutputMode {
    pub fn from_arguments(arguments: &serde_json::Value) -> Self {
        let mode = arguments
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("capture");
        match mode {
            "present" => ToolOutputMode::Present,
            "capture_and_present" | "capture+present" | "both" => ToolOutputMode::CaptureAndPresent,
            _ => ToolOutputMode::Capture,
        }
    }

    pub fn capture(self) -> bool {
        matches!(
            self,
            ToolOutputMode::Capture | ToolOutputMode::CaptureAndPresent
        )
    }

    pub fn present(self) -> bool {
        matches!(
            self,
            ToolOutputMode::Present | ToolOutputMode::CaptureAndPresent
        )
    }
}

pub struct ToolExecution {
    pub content: String,
    pub presented: bool,
}

fn with_output_mode(schema: serde_json::Value) -> serde_json::Value {
    let mut schema = schema;
    if let Some(obj) = schema.as_object_mut() {
        let props = obj
            .entry("properties")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let Some(props) = props.as_object_mut() {
            props.insert(
                "output_mode".to_string(),
                json!({
                    "type": "string",
                    "description": "How to use tool output: capture (LLM-only), present (show user), or capture_and_present",
                    "enum": ["capture", "present", "capture_and_present"],
                    "default": "capture"
                }),
            );
        }
    }
    schema
}

/// Get all available tools
pub fn get_all_tools() -> Vec<Tool> {
    vec![
        // Portfolio tools
        Tool {
            name: "portfolio_show".to_string(),
            description: "Show current portfolio positions with profit/loss. Can filter by asset type, single asset, or show historical snapshots.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "asset_type": {
                        "type": "string",
                        "description": "Filter by asset type (STOCK, FII, FIAGRO, FI_INFRA, BDR, ETF)",
                        "enum": ["STOCK", "FII", "FIAGRO", "FI_INFRA", "BDR", "ETF"]
                    },
                    "ticker": {
                        "type": "string",
                        "description": "Show a single asset by ticker (e.g., PETR4)"
                    },
                    "period": {
                        "type": "string",
                        "description": "Period or date: YTD, 2025, 2024-06, 2024-06-15, etc."
                    }
                }
            })),
        },
        Tool {
            name: "performance_show".to_string(),
            description: "Show performance report for a period with time-weighted return (TWR) calculation. Can scope to asset type or single asset.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "period": {
                        "type": "string",
                        "description": "Period: MTD, QTD, YTD, 1Y, ALL, YYYY (e.g., 2025), or from:to (YYYY-MM-DD:YYYY-MM-DD)",
                        "default": "YTD"
                    },
                    "asset_type": {
                        "type": "string",
                        "description": "Filter by asset type (STOCK, FII, etc.)",
                        "enum": ["STOCK", "FII", "FIAGRO", "FI_INFRA", "BDR", "ETF"]
                    },
                    "ticker": {
                        "type": "string",
                        "description": "Show performance for a single asset (e.g., PETR4)"
                    }
                }
            })),
        },
        // Tax tools
        Tool {
            name: "tax_report".to_string(),
            description: "Generate annual IRPF tax report for a specific year.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "year": {
                        "type": "integer",
                        "description": "Year (e.g., 2025)"
                    }
                },
                "required": ["year"]
            })),
        },
        Tool {
            name: "tax_calculate".to_string(),
            description: "Calculate swing trade tax for a specific month (MM/YYYY format).".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "month": {
                        "type": "string",
                        "description": "Month in MM/YYYY format (e.g., 12/2025)"
                    }
                },
                "required": ["month"]
            })),
        },
        Tool {
            name: "tax_summary".to_string(),
            description: "Show monthly tax summary for a year.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "year": {
                        "type": "integer",
                        "description": "Year (e.g., 2025)"
                    }
                },
                "required": ["year"]
            })),
        },
        // Income tools
        Tool {
            name: "income_show".to_string(),
            description: "Show income summary by asset, grouped by asset type (dividends, JCP, amortization). Can scope to asset type or single asset.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "period": {
                        "type": "string",
                        "description": "Period: YTD, 2025, MTD, from:to, etc. (default: YTD)"
                    },
                    "asset_type": {
                        "type": "string",
                        "description": "Filter by asset type (STOCK, FII, etc.)",
                        "enum": ["STOCK", "FII", "FIAGRO", "FI_INFRA", "BDR", "ETF"]
                    },
                    "ticker": {
                        "type": "string",
                        "description": "Show income detail for a single asset (e.g., PETR4)"
                    }
                }
            })),
        },
        Tool {
            name: "income_detail".to_string(),
            description: "Show detailed income events with dates and amounts.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "year": {
                        "type": "integer",
                        "description": "Year to filter (optional, defaults to current year)"
                    },
                    "asset": {
                        "type": "string",
                        "description": "Filter by asset ticker"
                    }
                }
            })),
        },
        Tool {
            name: "income_summary".to_string(),
            description: "Show monthly breakdown (if year given) or yearly totals (if no year).".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "year": {
                        "type": "integer",
                        "description": "Year for monthly breakdown (optional - omit for yearly totals)"
                    }
                }
            })),
        },
        // Income analytics tools
        Tool {
            name: "income_yield".to_string(),
            description: "Show yield analysis: LTM portfolio yield and per-asset yields. Can filter by ticker or asset type.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "description": "Filter by ticker symbol"
                    },
                    "asset_type": {
                        "type": "string",
                        "description": "Filter by asset type",
                        "enum": ["STOCK", "FII", "FIAGRO", "FI_INFRA", "BDR", "ETF"]
                    }
                }
            })),
        },
        Tool {
            name: "income_trends".to_string(),
            description: "Analyze income trends over time: direction (growing/declining/stable), YoY growth, volatility, and monthly series.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "description": "Filter by ticker symbol"
                    },
                    "months": {
                        "type": "integer",
                        "description": "Number of months to analyze (default: 36)",
                        "default": 36
                    }
                }
            })),
        },
        Tool {
            name: "income_forecast".to_string(),
            description: "Forecast future income based on historical patterns. Shows expected annual income, confidence bounds, and per-asset forecasts.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "year": {
                        "type": "integer",
                        "description": "Forecast year (default: next year)"
                    },
                    "conservative": {
                        "type": "boolean",
                        "description": "Use conservative estimates",
                        "default": false
                    }
                }
            })),
        },
        Tool {
            name: "income_calendar".to_string(),
            description: "Predict upcoming payment dates and amounts based on historical payment patterns.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {}
            })),
        },
        Tool {
            name: "income_alerts".to_string(),
            description: "Detect income anomalies: missed payments, unusual amounts, income drops.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {}
            })),
        },
        // Cash flow tools
        Tool {
            name: "cashflow_show".to_string(),
            description: "Show cash flow summary (money in/out). Can scope to asset type or single asset.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "period": {
                        "type": "string",
                        "description": "Period: ALL, YTD, 2025, MTD, from:to (default: ALL)"
                    },
                    "asset_type": {
                        "type": "string",
                        "description": "Filter by asset type",
                        "enum": ["STOCK", "FII", "FIAGRO", "FI_INFRA", "BDR", "ETF"]
                    },
                    "ticker": {
                        "type": "string",
                        "description": "Show cash flow for a single asset (e.g., PETR4)"
                    }
                }
            })),
        },
        // Assets tools
        Tool {
            name: "assets_list".to_string(),
            description: "List all assets, optionally filtered by type.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "asset_type": {
                        "type": "string",
                        "description": "Asset type to filter (STOCK, FII, FIAGRO, FI_INFRA, etc.)"
                    }
                }
            })),
        },
        Tool {
            name: "assets_show".to_string(),
            description: "Show details for a single asset by ticker.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "description": "Ticker symbol"
                    }
                },
                "required": ["ticker"]
            })),
        },
        // Transaction tools
        Tool {
            name: "transactions_list".to_string(),
            description: "List transactions, optionally filtered by ticker.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "description": "Ticker symbol to filter"
                    }
                }
            })),
        },
        // Database tools
        Tool {
            name: "db_schema".to_string(),
            description: "Get database schema and table information. Use this to understand the data model before querying.".to_string(),
            policy_group: PolicyGroup::Read,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {}
            })),
        },
        Tool {
            name: "db_query".to_string(),
            description: "Execute a read-only SQL query. Only SELECT and PRAGMA statements are allowed.".to_string(),
            policy_group: PolicyGroup::Sql,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "SQL query to execute (SELECT or PRAGMA only)"
                    }
                },
                "required": ["query"]
            })),
        },
        // Mutating operations
        Tool {
            name: "import_file".to_string(),
            description: "Import transactions from B3/CEI or Movimentação files. Auto-detects format.".to_string(),
            policy_group: PolicyGroup::Mutate,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the Excel or CSV file"
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Preview only, don't save to database",
                        "default": false
                    }
                },
                "required": ["file_path"]
            })),
        },
        Tool {
            name: "transactions_add".to_string(),
            description: "Manually add a buy or sell transaction.".to_string(),
            policy_group: PolicyGroup::Mutate,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "description": "Ticker symbol (e.g., PETR4, MXRF11)"
                    },
                    "transaction_type": {
                        "type": "string",
                        "description": "Transaction type: buy or sell",
                        "enum": ["buy", "sell"]
                    },
                    "quantity": {
                        "type": "string",
                        "description": "Quantity of shares/quotas"
                    },
                    "price": {
                        "type": "string",
                        "description": "Price per unit"
                    },
                    "date": {
                        "type": "string",
                        "description": "Trade date (YYYY-MM-DD)"
                    },
                    "fees": {
                        "type": "string",
                        "description": "Optional fees/brokerage",
                        "default": "0"
                    },
                    "day_trade": {
                        "type": "boolean",
                        "description": "Mark as day trade",
                        "default": false
                    },
                    "notes": {
                        "type": "string",
                        "description": "Optional notes"
                    }
                },
                "required": ["ticker", "transaction_type", "quantity", "price", "date"]
            })),
        },
        Tool {
            name: "income_add".to_string(),
            description: "Manually add an income event (dividend, JCP, or amortization).".to_string(),
            policy_group: PolicyGroup::Mutate,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {
                    "ticker": {
                        "type": "string",
                        "description": "Ticker symbol"
                    },
                    "event_type": {
                        "type": "string",
                        "description": "Event type",
                        "enum": ["DIVIDEND", "JCP", "AMORTIZATION"]
                    },
                    "total_amount": {
                        "type": "string",
                        "description": "Total amount received"
                    },
                    "date": {
                        "type": "string",
                        "description": "Event date (YYYY-MM-DD)"
                    },
                    "ex_date": {
                        "type": "string",
                        "description": "Optional ex-date (YYYY-MM-DD)"
                    },
                    "withholding": {
                        "type": "string",
                        "description": "Optional withholding tax",
                        "default": "0"
                    },
                    "amount_per_quota": {
                        "type": "string",
                        "description": "Optional amount per quota",
                        "default": "0"
                    },
                    "notes": {
                        "type": "string",
                        "description": "Optional notes"
                    }
                },
                "required": ["ticker", "event_type", "total_amount", "date"]
            })),
        },
        Tool {
            name: "prices_update".to_string(),
            description: "Update all asset prices from Yahoo Finance.".to_string(),
            policy_group: PolicyGroup::Mutate,
            parameters_schema: with_output_mode(json!({
                "type": "object",
                "properties": {}
            })),
        },
    ]
}

/// Get tool by name
pub fn get_tool(name: &str) -> Option<Tool> {
    get_all_tools().into_iter().find(|t| t.name == name)
}

pub fn strip_output_mode(arguments: &serde_json::Value) -> serde_json::Value {
    let mut cleaned = arguments.clone();
    if let Some(obj) = cleaned.as_object_mut() {
        obj.remove("output_mode");
    }
    cleaned
}

/// Execute a tool by routing to the appropriate dispatcher
pub async fn execute_tool(
    name: &str,
    arguments: serde_json::Value,
    output_mode: ToolOutputMode,
) -> Result<ToolExecution> {
    use crate::cli::Commands;

    let policy_group = get_tool(name)
        .as_ref()
        .map(|tool| tool.policy_group)
        .unwrap_or(PolicyGroup::Read);

    match name {
        "db_schema" => execute_text_tool(execute_db_schema().await?, output_mode),
        "db_query" => {
            let query = arguments
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?;
            execute_text_tool(execute_db_query(query).await?, output_mode)
        }
        "portfolio_show" => {
            let asset_type = arguments
                .get("asset_type")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<crate::db::AssetType>().ok());
            let ticker = arguments
                .get("ticker")
                .and_then(|v| v.as_str())
                .map(String::from);
            let period = arguments
                .get("period")
                .and_then(|v| v.as_str())
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(String::from);

            let action = if let Some(ticker) = ticker {
                crate::cli::PortfolioCommands::Asset { ticker, period }
            } else if let Some(asset_type) = asset_type {
                crate::cli::PortfolioCommands::Type { asset_type, period }
            } else {
                crate::cli::PortfolioCommands::Show {
                    period: period.clone(),
                    asset_type: None,
                    at: None,
                }
            };

            let command = Commands::Portfolio { action };
            execute_command(command, output_mode, policy_group).await
        }
        "performance_show" => {
            let period = arguments
                .get("period")
                .and_then(|v| v.as_str())
                .map(String::from);
            let asset_type = arguments
                .get("asset_type")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<crate::db::AssetType>().ok());
            let ticker = arguments
                .get("ticker")
                .and_then(|v| v.as_str())
                .map(String::from);

            let action = if let Some(ticker) = ticker {
                crate::cli::PerformanceCommands::Asset { ticker, period }
            } else if let Some(asset_type) = asset_type {
                crate::cli::PerformanceCommands::Type { asset_type, period }
            } else {
                crate::cli::PerformanceCommands::Show {
                    period: period.or(Some("YTD".to_string())),
                }
            };

            let command = Commands::Performance { action };
            execute_command(command, output_mode, policy_group).await
        }
        "tax_report" => {
            let year = arguments
                .get("year")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("Missing 'year' parameter"))?
                as i32;

            let command = Commands::Tax {
                action: crate::cli::TaxCommands::Report {
                    year,
                    export: false,
                },
            };

            execute_command(command, output_mode, policy_group).await
        }
        "tax_calculate" => {
            let month = arguments
                .get("month")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'month' parameter"))?
                .to_string();

            let command = Commands::Tax {
                action: crate::cli::TaxCommands::Calculate { month },
            };

            execute_command(command, output_mode, policy_group).await
        }
        "tax_summary" => {
            let year = arguments
                .get("year")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("Missing 'year' parameter"))?
                as i32;

            let command = Commands::Tax {
                action: crate::cli::TaxCommands::Summary { year },
            };

            execute_command(command, output_mode, policy_group).await
        }
        "income_show" => {
            let period = arguments
                .get("period")
                .and_then(|v| v.as_str())
                .map(String::from);
            let asset_type = arguments
                .get("asset_type")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<crate::db::AssetType>().ok());
            let ticker = arguments
                .get("ticker")
                .and_then(|v| v.as_str())
                .map(String::from);

            let action = if let Some(ticker) = ticker {
                crate::cli::IncomeCommands::Asset { ticker, period }
            } else if let Some(asset_type) = asset_type {
                crate::cli::IncomeCommands::Type { asset_type, period }
            } else {
                crate::cli::IncomeCommands::Show { period }
            };

            let command = Commands::Income { action };
            execute_command(command, output_mode, policy_group).await
        }
        "income_detail" => {
            let year = arguments
                .get("year")
                .and_then(|v| v.as_i64())
                .map(|y| y as i32);
            let asset = arguments
                .get("asset")
                .and_then(|v| v.as_str())
                .map(String::from);

            let command = Commands::Income {
                action: crate::cli::IncomeCommands::Detail { year, asset },
            };

            execute_command(command, output_mode, policy_group).await
        }
        "income_summary" => {
            let year = arguments
                .get("year")
                .and_then(|v| v.as_i64())
                .map(|y| y as i32);

            let command = Commands::Income {
                action: crate::cli::IncomeCommands::Summary {
                    year,
                    categorize: false,
                    tax_aware: false,
                },
            };

            execute_command(command, output_mode, policy_group).await
        }
        "income_yield" => {
            let ticker = arguments
                .get("ticker")
                .and_then(|v| v.as_str())
                .map(String::from);
            let asset_type = arguments
                .get("asset_type")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<crate::db::AssetType>().ok());

            let command = Commands::Income {
                action: crate::cli::IncomeCommands::Yield {
                    ticker,
                    asset_type,
                    period: "LTM".to_string(),
                },
            };
            execute_command(command, output_mode, policy_group).await
        }
        "income_trends" => {
            let ticker = arguments
                .get("ticker")
                .and_then(|v| v.as_str())
                .map(String::from);
            let months = arguments
                .get("months")
                .and_then(|v| v.as_i64())
                .unwrap_or(36) as i32;

            let command = Commands::Income {
                action: crate::cli::IncomeCommands::Trends { ticker, months },
            };
            execute_command(command, output_mode, policy_group).await
        }
        "income_forecast" => {
            let year = arguments
                .get("year")
                .and_then(|v| v.as_i64())
                .map(|y| y as i32);
            let conservative = arguments
                .get("conservative")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let command = Commands::Income {
                action: crate::cli::IncomeCommands::Forecast { year, conservative },
            };
            execute_command(command, output_mode, policy_group).await
        }
        "income_calendar" => {
            let command = Commands::Income {
                action: crate::cli::IncomeCommands::Calendar { month: None },
            };
            execute_command(command, output_mode, policy_group).await
        }
        "income_alerts" => {
            let command = Commands::Income {
                action: crate::cli::IncomeCommands::Alerts,
            };
            execute_command(command, output_mode, policy_group).await
        }
        "cashflow_show" => {
            let period = arguments
                .get("period")
                .and_then(|v| v.as_str())
                .map(String::from);
            let asset_type = arguments
                .get("asset_type")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<crate::db::AssetType>().ok());
            let ticker = arguments
                .get("ticker")
                .and_then(|v| v.as_str())
                .map(String::from);

            let action = if let Some(ticker) = ticker {
                crate::cli::CashFlowCommands::Asset { ticker, period }
            } else if let Some(asset_type) = asset_type {
                crate::cli::CashFlowCommands::Type { asset_type, period }
            } else {
                crate::cli::CashFlowCommands::Show { period }
            };

            let command = Commands::CashFlow { action };
            execute_command(command, output_mode, policy_group).await
        }
        "assets_list" => {
            let asset_type = arguments
                .get("asset_type")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<crate::db::AssetType>().ok());

            let command = Commands::Assets {
                action: crate::cli::AssetsCommands::List { asset_type },
            };

            execute_command(command, output_mode, policy_group).await
        }
        "assets_show" => {
            let ticker = arguments
                .get("ticker")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'ticker' parameter"))?
                .to_string();

            let command = Commands::Assets {
                action: crate::cli::AssetsCommands::Show { ticker },
            };

            execute_command(command, output_mode, policy_group).await
        }
        "transactions_list" => {
            let ticker = arguments
                .get("ticker")
                .and_then(|v| v.as_str())
                .map(String::from);

            let command = Commands::Transactions {
                action: crate::cli::TransactionCommands::List { ticker },
            };

            execute_command(command, output_mode, policy_group).await
        }
        "transactions_add" => {
            let ticker = arguments
                .get("ticker")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'ticker' parameter"))?
                .to_string();
            let transaction_type = arguments
                .get("transaction_type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'transaction_type' parameter"))?
                .to_string();
            let quantity = arguments
                .get("quantity")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'quantity' parameter"))?
                .to_string();
            let price = arguments
                .get("price")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'price' parameter"))?
                .to_string();
            let date = arguments
                .get("date")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'date' parameter"))?
                .to_string();
            let fees = arguments
                .get("fees")
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string();
            let day_trade = arguments
                .get("day_trade")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let notes = arguments
                .get("notes")
                .and_then(|v| v.as_str())
                .map(String::from);

            let command = Commands::Transactions {
                action: crate::cli::TransactionCommands::Add {
                    ticker,
                    transaction_type,
                    quantity,
                    price,
                    date,
                    fees,
                    day_trade,
                    notes,
                },
            };

            execute_command(command, output_mode, policy_group).await
        }
        "income_add" => {
            let ticker = arguments
                .get("ticker")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'ticker' parameter"))?
                .to_string();
            let event_type = arguments
                .get("event_type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'event_type' parameter"))?
                .to_string();
            let total_amount = arguments
                .get("total_amount")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'total_amount' parameter"))?
                .to_string();
            let date = arguments
                .get("date")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'date' parameter"))?
                .to_string();
            let ex_date = arguments
                .get("ex_date")
                .and_then(|v| v.as_str())
                .map(String::from);
            let withholding = arguments
                .get("withholding")
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string();
            let amount_per_quota = arguments
                .get("amount_per_quota")
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string();
            let notes = arguments
                .get("notes")
                .and_then(|v| v.as_str())
                .map(String::from);

            let command = Commands::Income {
                action: crate::cli::IncomeCommands::Add {
                    ticker,
                    event_type,
                    total_amount,
                    date,
                    ex_date,
                    withholding,
                    amount_per_quota,
                    notes,
                },
            };

            execute_command(command, output_mode, policy_group).await
        }
        "import_file" => {
            let file_path = arguments
                .get("file_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'file_path' parameter"))?
                .to_string();
            let dry_run = arguments
                .get("dry_run")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let command = Commands::Import {
                file: file_path,
                dry_run,
                force_reimport: false,
            };

            execute_command(command, output_mode, policy_group).await
        }
        "prices_update" => {
            let command = Commands::Prices {
                action: crate::cli::PriceCommands::Update,
            };

            execute_command(command, output_mode, policy_group).await
        }
        _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
    }
}

fn execute_text_tool(result: String, output_mode: ToolOutputMode) -> Result<ToolExecution> {
    if output_mode.present() {
        println!("\n{}", result);
        Ok(ToolExecution {
            content: if output_mode.capture() {
                result
            } else {
                "Output presented to user.".to_string()
            },
            presented: true,
        })
    } else {
        Ok(ToolExecution {
            content: result,
            presented: false,
        })
    }
}

/// Helper to execute a command and capture/present output as configured
async fn execute_command(
    command: crate::cli::Commands,
    output_mode: ToolOutputMode,
    policy_group: PolicyGroup,
) -> Result<ToolExecution> {
    let command = std::sync::Arc::new(command);
    execute_command_with(command, output_mode, policy_group, |cmd, options| {
        let cmd = cmd.clone();
        async move { crate::dispatcher::dispatch_command(cmd.as_ref(), options).await }
    })
    .await
}

async fn execute_command_with<F, Fut>(
    command: std::sync::Arc<crate::cli::Commands>,
    output_mode: ToolOutputMode,
    _policy_group: PolicyGroup,
    dispatch: F,
) -> Result<ToolExecution>
where
    F: Fn(std::sync::Arc<crate::cli::Commands>, OutputOptions) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let mut captured = String::new();
    let mut presented = false;

    match output_mode {
        ToolOutputMode::Capture => {
            let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let options = OutputOptions::new(OutputMode::Json, PrivacyMode::Full)
                .with_capture(buffer.clone());
            dispatch(command.clone(), options).await?;
            let output = buffer
                .lock()
                .map_err(|_| anyhow::anyhow!("Output buffer lock poisoned"))?
                .clone();
            captured = String::from_utf8_lossy(&output).to_string();
        }
        ToolOutputMode::Present => {
            let options = OutputOptions::new(OutputMode::Table, PrivacyMode::Full);
            dispatch(command.clone(), options).await?;
            presented = true;
        }
        ToolOutputMode::CaptureAndPresent => {
            let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let options = OutputOptions::new(OutputMode::Table, PrivacyMode::Full)
                .with_secondary_output(OutputMode::Json, OutputTarget::Capture(buffer.clone()));
            dispatch(command.clone(), options).await?;
            let output = buffer
                .lock()
                .map_err(|_| anyhow::anyhow!("Output buffer lock poisoned"))?
                .clone();
            captured = String::from_utf8_lossy(&output).to_string();
            presented = true;
        }
    }

    let content = if output_mode.capture() {
        captured
    } else {
        "Output presented to user.".to_string()
    };

    Ok(ToolExecution { content, presented })
}

/// Execute db_schema tool
async fn execute_db_schema() -> Result<String> {
    use crate::db;

    let conn = db::open_db_read_only(None)?;

    // Query schema information
    let mut schema_info = String::new();

    // Get table list
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    schema_info.push_str("# Database Schema\n\n");
    schema_info.push_str(&format!("Tables: {}\n\n", tables.join(", ")));

    // Get schema for each table
    for table in &tables {
        let create_sql: String = conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |row| row.get(0),
        )?;
        schema_info.push_str(&format!(
            "## Table: {}\n\n```sql\n{}\n```\n\n",
            table, create_sql
        ));
    }

    Ok(schema_info)
}

/// Execute db_query tool (read-only)
async fn execute_db_query(query: &str) -> Result<String> {
    use crate::db;

    // Validate query is read-only
    let query_upper = query.trim().to_uppercase();
    if !query_upper.starts_with("SELECT") && !query_upper.starts_with("PRAGMA") {
        anyhow::bail!("Only SELECT and PRAGMA queries are allowed");
    }

    // Check for dangerous keywords
    let dangerous_keywords = ["INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE"];
    for keyword in &dangerous_keywords {
        if query_upper.contains(keyword) {
            anyhow::bail!("Query contains forbidden keyword: {}", keyword);
        }
    }

    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Execute query
    let mut stmt = conn.prepare(query)?;
    let column_count = stmt.column_count();
    let column_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

    let rows = stmt.query_map([], |row| {
        let mut values = Vec::new();
        for i in 0..column_count {
            let value: String = match row.get_ref(i)? {
                rusqlite::types::ValueRef::Null => "NULL".to_string(),
                rusqlite::types::ValueRef::Integer(i) => i.to_string(),
                rusqlite::types::ValueRef::Real(f) => f.to_string(),
                rusqlite::types::ValueRef::Text(s) => String::from_utf8_lossy(s).to_string(),
                rusqlite::types::ValueRef::Blob(b) => format!("<blob {} bytes>", b.len()),
            };
            values.push(value);
        }
        Ok(values)
    })?;

    // Format as table
    let mut result = String::new();
    result.push_str("# Query Results\n\n");
    result.push_str(&format!("Columns: {}\n\n", column_names.join(" | ")));

    let mut row_count = 0;
    for row in rows {
        let row = row?;
        result.push_str(&format!("{}\n", row.join(" | ")));
        row_count += 1;
        if row_count >= 100 {
            result.push_str("\n... (limited to 100 rows)\n");
            break;
        }
    }

    if row_count == 0 {
        result.push_str("(no rows)\n");
    } else {
        result.push_str(&format!("\nTotal: {} rows\n", row_count));
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn tool_output_mode_defaults_to_capture() {
        let args = json!({"asset_type": "STOCK"});
        assert_eq!(
            ToolOutputMode::from_arguments(&args),
            ToolOutputMode::Capture
        );
    }

    #[test]
    fn tool_output_mode_respects_present() {
        let args = json!({"output_mode": "present"});
        assert_eq!(
            ToolOutputMode::from_arguments(&args),
            ToolOutputMode::Present
        );
    }

    #[test]
    fn tool_output_mode_respects_capture_and_present_variants() {
        let args = json!({"output_mode": "capture_and_present"});
        assert_eq!(
            ToolOutputMode::from_arguments(&args),
            ToolOutputMode::CaptureAndPresent
        );

        let args = json!({"output_mode": "both"});
        assert_eq!(
            ToolOutputMode::from_arguments(&args),
            ToolOutputMode::CaptureAndPresent
        );
    }

    #[test]
    fn strip_output_mode_removes_only_output_mode() {
        let args = json!({"output_mode": "present", "ticker": "ANIM3"});
        let stripped = strip_output_mode(&args);
        assert_eq!(stripped, json!({"ticker": "ANIM3"}));
    }

    #[tokio::test]
    async fn capture_and_present_executes_once() {
        let command = crate::cli::Commands::Portfolio {
            action: crate::cli::PortfolioCommands::Show {
                period: None,
                asset_type: None,
                at: None,
            },
        };
        let command = std::sync::Arc::new(command);
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let result = execute_command_with(
            command,
            ToolOutputMode::CaptureAndPresent,
            PolicyGroup::Mutate,
            move |_cmd, options| {
                let counter = counter_clone.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    if let Some(target) = options.secondary_output.clone() {
                        target.write_all(b"ok")?;
                        target.write_all(b"\n")?;
                    }
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(result.presented);
        assert!(result.content.contains("ok"));
    }
}
