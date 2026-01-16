use clap::{Parser, Subcommand};

pub mod formatters;
pub mod runner;

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

    /// Cash flow reporting
    CashFlow {
        #[command(subcommand)]
        action: CashFlowCommands,
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

    /// Inconsistencies management
    Inconsistencies {
        #[command(subcommand)]
        action: InconsistenciesCommands,
    },

    /// Ticker metadata management
    Tickers {
        #[command(subcommand)]
        action: TickersCommands,
    },

    /// Asset management
    Assets {
        #[command(subcommand)]
        action: AssetsCommands,
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
pub enum CashFlowCommands {
    /// Show cash flow summary
    Show {
        /// Period: MTD, QTD, YTD, 1Y, ALL, YYYY (e.g., 2025), or from:to (YYYY-MM-DD:YYYY-MM-DD)
        period: Option<String>,
    },
    /// Show cash flow statistics
    Stats {
        /// Period: MTD, QTD, YTD, 1Y, ALL, YYYY (e.g., 2025), or from:to (YYYY-MM-DD:YYYY-MM-DD)
        period: Option<String>,
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
    /// Manage asset renames (symbol-only changes)
    Rename {
        #[command(subcommand)]
        action: RenameCommands,
    },

    /// Manage splits and reverse splits (quantity adjustments)
    Split {
        #[command(subcommand)]
        action: SplitCommands,
    },

    /// Manage bonus actions (synthetic share grants)
    Bonus {
        #[command(subcommand)]
        action: BonusCommands,
    },

    /// Manage spin-offs (parent continues)
    Spinoff {
        #[command(subcommand)]
        action: ExchangeCommands,
    },

    /// Manage mergers/exchanges (source disappears)
    Merger {
        #[command(subcommand)]
        action: ExchangeCommands,
    },

    /// Apply unapplied corporate actions to transactions
    Apply {
        /// Ticker symbol (optional, applies all if not specified)
        ticker: Option<String>,
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
}

#[derive(Subcommand)]
pub enum RenameCommands {
    /// Add a rename
    Add {
        /// Old ticker symbol
        from: String,
        /// New ticker symbol
        to: String,
        /// Effective date (YYYY-MM-DD)
        date: String,
        /// Optional notes
        #[arg(short, long)]
        notes: Option<String>,
    },
    /// List renames (optional ticker filter)
    List {
        /// Ticker symbol (optional)
        ticker: Option<String>,
    },
    /// Remove a rename by ID
    Remove {
        /// Rename ID
        id: i64,
    },
}

#[derive(Subcommand)]
pub enum SplitCommands {
    /// Add a split or reverse split (negative adjustment for reverse)
    Add {
        /// Ticker symbol
        ticker: String,
        /// Quantity adjustment (signed)
        quantity_adjustment: String,
        /// Ex-date (YYYY-MM-DD)
        date: String,
        /// Optional notes
        #[arg(short, long)]
        notes: Option<String>,
    },
    /// List splits (optional ticker filter)
    List {
        /// Ticker symbol (optional)
        ticker: Option<String>,
    },
    /// Remove a split by ID
    Remove {
        /// Corporate action ID
        id: i64,
    },
}

#[derive(Subcommand)]
pub enum BonusCommands {
    /// Add a bonus action
    Add {
        /// Ticker symbol
        ticker: String,
        /// Quantity adjustment
        quantity_adjustment: String,
        /// Ex-date (YYYY-MM-DD)
        date: String,
        /// Optional notes
        #[arg(short, long)]
        notes: Option<String>,
    },
    /// List bonus actions (optional ticker filter)
    List {
        /// Ticker symbol (optional)
        ticker: Option<String>,
    },
    /// Remove a bonus action by ID
    Remove {
        /// Corporate action ID
        id: i64,
    },
}

#[derive(Subcommand)]
pub enum ExchangeCommands {
    /// Add a spin-off or merger exchange
    Add {
        /// Source ticker
        from: String,
        /// Target ticker
        to: String,
        /// Effective date (YYYY-MM-DD)
        date: String,
        /// Quantity received
        quantity: String,
        /// Cost basis allocated to new ticker
        allocated_cost: String,
        /// Cash amortization amount
        #[arg(long)]
        cash: Option<String>,
        /// Optional notes
        #[arg(short, long)]
        notes: Option<String>,
    },
    /// List exchanges (optional ticker filter)
    List {
        /// Ticker symbol (optional)
        ticker: Option<String>,
    },
    /// Remove an exchange by ID
    Remove {
        /// Exchange ID
        id: i64,
    },
}

#[derive(Subcommand)]
pub enum InconsistenciesCommands {
    /// List inconsistencies
    List {
        /// Show only open issues (default)
        #[arg(long, conflicts_with = "all")]
        open: bool,

        /// Show all issues
        #[arg(long)]
        all: bool,

        /// Filter by status (OPEN, RESOLVED, IGNORED)
        #[arg(long)]
        status: Option<String>,

        /// Filter by issue type (e.g., MISSING_COST_BASIS)
        #[arg(long = "type")]
        issue_type: Option<String>,

        /// Filter by asset ticker
        #[arg(long)]
        asset: Option<String>,
    },

    /// Show details for a single inconsistency
    Show {
        /// Inconsistency id
        id: i64,
    },

    /// Resolve an inconsistency (interactive if no fields provided)
    /// If no ID is provided, iterates through all open inconsistencies
    Resolve {
        /// Inconsistency id (optional - if not provided, resolves all open issues one by one)
        id: Option<i64>,

        /// Inline fields (repeatable) in key=value format
        #[arg(long = "set")]
        set: Vec<String>,

        /// JSON resolution payload (object as string)
        #[arg(long = "payload")]
        json_payload: Option<String>,
    },

    /// Ignore an inconsistency
    Ignore {
        /// Inconsistency id
        id: i64,

        /// Optional ignore reason
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum TickersCommands {
    /// Refresh B3 tickers cache
    Refresh {
        /// Force refresh even if cache is fresh
        #[arg(long)]
        force: bool,
    },

    /// Show cache status
    Status,

    /// List tickers with UNKNOWN asset type
    ListUnknown,

    /// Resolve a ticker's asset type (interactive if no ticker provided)
    Resolve {
        /// Ticker symbol (optional - if not provided, resolves all unknown tickers one by one)
        ticker: Option<String>,

        /// Asset type to set (required when a ticker is provided)
        #[arg(long = "type")]
        asset_type: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum AssetsCommands {
    /// List all assets (optional filter by type)
    List {
        /// Asset type to filter (STOCK, FII, FIAGRO, FI_INFRA, etc.)
        #[arg(long = "type")]
        asset_type: Option<String>,
    },

    /// Show details for a single asset
    Show {
        /// Ticker symbol
        ticker: String,
    },

    /// Add a new asset
    Add {
        /// Ticker symbol
        ticker: String,

        /// Asset type (overrides auto-detect)
        #[arg(long = "type")]
        asset_type: Option<String>,

        /// Optional name
        #[arg(long)]
        name: Option<String>,
    },

    /// Update asset type
    SetType {
        /// Ticker symbol
        ticker: String,

        /// Asset type
        asset_type: String,
    },

    /// Update asset name
    SetName {
        /// Ticker symbol
        ticker: String,

        /// Asset name
        name: String,
    },

    /// Rename ticker symbol (correction-only)
    Rename {
        /// Old ticker
        old_ticker: String,

        /// New ticker
        new_ticker: String,
    },

    /// Remove asset and all related data
    Remove {
        /// Ticker symbol
        ticker: String,
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
