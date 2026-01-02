use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "interest")]
#[command(version, about = "Brazilian B3 investment tracker with tax calculations")]
#[command(long_about = "Track your Brazilian stock exchange investments (stocks, FII, FIAGRO, FI-INFRA) with automatic price updates, performance analysis, and tax calculations.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Import transactions from B3/CEI exports (Excel or CSV)
    Import {
        /// Path to the Excel or CSV file
        file: String,

        /// Preview only, don't save to database
        #[arg(short, long)]
        dry_run: bool,
    },

    /// Portfolio management and viewing
    Portfolio {
        #[command(subcommand)]
        action: PortfolioCommands,
    },

    /// Price data management
    Prices {
        #[command(subcommand)]
        action: PriceCommands,
    },

    /// Tax calculations and reports
    Tax {
        #[command(subcommand)]
        action: TaxCommands,
    },

    /// Corporate actions (splits, bonuses, amortization)
    Actions {
        #[command(subcommand)]
        action: ActionCommands,
    },

    /// Manual transaction management
    Transactions {
        #[command(subcommand)]
        action: TransactionCommands,
    },
}

#[derive(Subcommand)]
pub enum PortfolioCommands {
    /// Show current portfolio with P&L
    Show {
        /// Filter by asset type (STOCK, FII, FIAGRO, FI_INFRA)
        #[arg(short, long)]
        asset_type: Option<String>,
    },

    /// Show performance metrics over time
    Performance {
        /// Time period: 1m, 3m, 6m, 1y, all
        #[arg(short, long, default_value = "all")]
        period: String,
    },
}

#[derive(Subcommand)]
pub enum PriceCommands {
    /// Update all asset prices
    Update,

    /// Fetch historical prices for a specific ticker
    History {
        /// Ticker symbol (e.g., PETR4)
        ticker: String,

        /// Start date (YYYY-MM-DD)
        #[arg(short, long)]
        from: String,

        /// End date (YYYY-MM-DD)
        #[arg(short, long)]
        to: String,
    },
}

#[derive(Subcommand)]
pub enum TaxCommands {
    /// Calculate tax for a specific month
    Calculate {
        /// Month in MM/YYYY format (e.g., 12/2025)
        month: String,
    },

    /// Generate annual IRPF tax report
    Report {
        /// Year (e.g., 2025)
        year: i32,
    },

    /// Show monthly tax summary for a year
    Summary {
        /// Year (e.g., 2025)
        year: i32,
    },
}

#[derive(Subcommand)]
pub enum ActionCommands {
    /// Manually add a corporate action (split, reverse split, bonus)
    Add {
        /// Ticker symbol (e.g., PETR4, A1MD34)
        ticker: String,

        /// Action type: split, reverse-split, or bonus
        #[arg(value_parser = ["split", "reverse-split", "bonus", "SPLIT", "REVERSE-SPLIT", "BONUS"])]
        action_type: String,

        /// Ratio in format "from:to" (e.g., "1:2" for 1:2 split, "10:1" for 10:1 reverse split)
        ratio: String,

        /// Ex-date when the action becomes effective (YYYY-MM-DD)
        date: String,

        /// Optional notes
        #[arg(short, long)]
        notes: Option<String>,
    },

    /// Update corporate actions from API
    Update,

    /// List corporate actions for a ticker
    List {
        /// Ticker symbol (optional, shows all if not specified)
        ticker: Option<String>,
    },

    /// Apply unapplied corporate actions to transactions
    Apply {
        /// Ticker symbol (optional, applies all if not specified)
        ticker: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum TransactionCommands {
    /// Manually add a buy or sell transaction
    Add {
        /// Ticker symbol (e.g., PETR4, MXRF11)
        ticker: String,

        /// Transaction type: buy or sell
        #[arg(value_parser = ["buy", "sell", "BUY", "SELL"])]
        transaction_type: String,

        /// Quantity of shares/quotas
        quantity: String,

        /// Price per unit
        price: String,

        /// Trade date (YYYY-MM-DD)
        date: String,

        /// Optional fees/brokerage
        #[arg(short, long, default_value = "0")]
        fees: String,

        /// Optional notes
        #[arg(short, long)]
        notes: Option<String>,
    },
}
