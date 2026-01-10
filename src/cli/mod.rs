use clap::{Parser, Subcommand};

pub mod formatters;

#[derive(Parser)]
#[command(name = "interest")]
#[command(
    version,
    about = "Brazilian B3 investment tracker with tax calculations"
)]
#[command(
    long_about = "Track your Brazilian stock exchange investments (stocks, FII, FIAGRO, FI-INFRA) with automatic price updates, performance analysis, and tax calculations."
)]
pub struct Cli {
    /// Disable colorized/ANSI output
    #[arg(long = "no-color", global = true)]
    pub no_color: bool,

    /// Output results in JSON format
    #[arg(long = "json", global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Import transactions from B3/CEI or Movimentação files (auto-detects format)
    Import {
        /// Path to the Excel or CSV file
        file: String,

        /// Preview only, don't save to database
        #[arg(short, long)]
        dry_run: bool,
    },

    /// Import opening positions from IRPF tax declaration PDF
    ImportIrpf {
        /// Path to the IRPF PDF file
        file: String,

        /// IRPF year (e.g., 2018 for declaration filed in 2019 about 2018)
        year: i32,

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

    /// Performance analysis and reporting
    Performance {
        #[command(subcommand)]
        action: PerformanceCommands,
    },

    /// Income events (dividends, JCP, amortization)
    Income {
        #[command(subcommand)]
        action: IncomeCommands,
    },

    /// Corporate actions (splits, bonuses, amortization)
    Actions {
        #[command(subcommand)]
        action: ActionCommands,
    },

    /// Process term contract liquidations
    ProcessTerms,

    /// Manual transaction management
    Transactions {
        #[command(subcommand)]
        action: TransactionCommands,
    },

    /// Inspect Excel/CSV file structure
    Inspect {
        /// Path to the Excel or CSV file
        file: String,

        /// Show full data rows, not just headers
        #[arg(short, long)]
        full: bool,

        /// Analyze and show unique values in a column (e.g., --column 2 for movement types)
        #[arg(short, long)]
        column: Option<usize>,
    },

    /// Launch interactive TUI mode
    Interactive,
}

#[derive(Subcommand)]
pub enum PortfolioCommands {
    /// Show current portfolio with P&L
    Show {
        /// Filter by asset type (STOCK, FII, FIAGRO, FI_INFRA)
        #[arg(short, long)]
        asset_type: Option<String>,

        /// Show portfolio as of this date (YYYY-MM-DD, YYYY-MM, or YYYY)
        #[arg(long)]
        at: Option<String>,
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

        /// Export report to CSV (irpf_report_<year>.csv)
        #[arg(long)]
        export: bool,
    },

    /// Show monthly tax summary for a year
    Summary {
        /// Year (e.g., 2025)
        year: i32,
    },
}

#[derive(Subcommand)]
pub enum PerformanceCommands {
    /// Show performance report for a period
    Show {
        /// Period: MTD, QTD, YTD, 1Y, ALL, YYYY (e.g., 2025), or from:to (YYYY-MM-DD:YYYY-MM-DD)
        period: String,
    },
}

#[derive(Subcommand)]
pub enum IncomeCommands {
    /// Show income summary by asset, grouped by asset type
    Show {
        /// Year to filter (optional, defaults to current year)
        year: Option<i32>,
    },

    /// Show detailed income events
    Detail {
        /// Year to filter (optional, defaults to current year)
        year: Option<i32>,

        /// Filter by asset ticker
        #[arg(short, long)]
        asset: Option<String>,
    },

    /// Show monthly breakdown (if year given) or yearly totals (if no year)
    Summary {
        /// Year (optional - omit for yearly totals)
        year: Option<i32>,
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

    /// Scrape corporate actions from investing.com
    Scrape {
        /// Ticker symbol (e.g., A1MD34)
        ticker: String,

        /// investing.com URL (optional, will auto-build from asset name)
        #[arg(short, long)]
        url: Option<String>,

        /// Company name for URL building (optional, uses database if not provided)
        #[arg(short, long)]
        name: Option<String>,

        /// Save scraped actions to database
        #[arg(short, long)]
        save: bool,
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

    /// Delete a corporate action by ID
    Delete {
        /// Corporate action ID
        id: i64,
    },

    /// Edit a corporate action's details
    Edit {
        /// Corporate action ID
        id: i64,

        /// New action type: split, reverse-split, or bonus
        #[arg(long, value_parser = ["split", "reverse-split", "bonus", "SPLIT", "REVERSE-SPLIT", "BONUS"])]
        action_type: Option<String>,

        /// New ratio in format "from:to" (e.g., "1:8")
        #[arg(long)]
        ratio: Option<String>,

        /// New ex-date (YYYY-MM-DD)
        #[arg(long)]
        date: Option<String>,

        /// New notes
        #[arg(long)]
        notes: Option<String>,
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
