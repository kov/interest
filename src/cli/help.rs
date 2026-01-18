use colored::Colorize;
use std::io::{self, Write};

pub struct RenderOpts {
    pub show_examples: bool,
}

impl Default for RenderOpts {
    fn default() -> Self {
        RenderOpts {
            show_examples: true,
        }
    }
}

pub fn render_help<W: Write>(mut out: W, _opts: &RenderOpts) -> io::Result<()> {
    // Simple, borrow-safe sequential writes
    writeln!(out, "{}", "Interest - Help".bold())?;
    writeln!(out)?;

    writeln!(out, "{}  interest <command> [options]", "Usage:".bold())?;
    writeln!(out)?;

    writeln!(out, "{}", "Common commands:".bold())?;
    writeln!(
        out,
        "  {:24} - Show portfolio snapshot",
        "portfolio show [--at DATE]"
    )?;
    writeln!(
        out,
        "  {:24} - Show performance (MTD/QTD/YTD/1Y/ALL)",
        "performance show <period>"
    )?;
    writeln!(out, "  {:24} - Show income by asset", "income show [year]")?;
    writeln!(
        out,
        "  {:24} - Filter by asset type (fii, stock, fiagro)",
        "portfolio show --asset-type <type>"
    )?;
    writeln!(
        out,
        "  {:24} - Import trades or movimentacao (preview with --dry-run)",
        "import <file> [--dry-run]"
    )?;

    writeln!(out)?;
    writeln!(out, "{}", "Import & sync:".bold())?;
    writeln!(
        out,
        "  {:24} - Import opening balances from IRPF PDF",
        "import-irpf <file> <year>"
    )?;
    writeln!(
        out,
        "  {:24} - Import COTAHIST yearly prices",
        "prices import-b3 <year>"
    )?;
    writeln!(
        out,
        "  {:24} - Sync asset metadata registry",
        "assets sync-maisretorno"
    )?;

    writeln!(out)?;
    writeln!(out, "{}", "Resolve & reconcile:".bold())?;
    writeln!(
        out,
        "  {:24} - Find and resolve import issues",
        "inconsistencies list/resolve"
    )?;
    writeln!(
        out,
        "  {:24} - Resolve unknown tickers",
        "tickers list-unknown/resolve"
    )?;
    writeln!(
        out,
        "  {:24} - Apply unapplied corporate actions",
        "actions apply [ticker]"
    )?;

    writeln!(out)?;
    writeln!(out, "{}", "Manage & maintain:".bold())?;
    writeln!(
        out,
        "  {:24} - Manage asset registry",
        "assets add/set-type/set-name"
    )?;
    writeln!(
        out,
        "  {:24} - Manage corporate actions",
        "actions split/bonus/spinoff/merger"
    )?;
    writeln!(
        out,
        "  {:24} - Add manual buy/sell entries",
        "transactions add"
    )?;

    writeln!(out)?;
    writeln!(out, "{}", "Reports & tax:".bold())?;
    writeln!(
        out,
        "  {:24} - Generate IRPF report (CSV export available)",
        "tax report <year>"
    )?;
    writeln!(out, "  {:24} - Condensed tax summary", "tax summary <year>")?;

    writeln!(out)?;
    writeln!(out, "{}", "Utilities & session:".bold())?;
    writeln!(out, "  {:24} - Launch the TUI (default)", "interactive")?;
    writeln!(out, "  {:24} - Show this help", "help")?;
    writeln!(out, "  {:24} - Exit the application", "exit")?;

    writeln!(out)?;
    if _opts.show_examples {
        writeln!(out, "{}", "Examples:".bold())?;
        writeln!(out, "  interest import negociacao.xlsx --dry-run")?;
        writeln!(out, "  interest import movimentacao.xlsx")?;
        writeln!(out, "  interest portfolio show --at 2024-12-31")?;
        writeln!(out, "  interest tax report 2024 --export")?;
        writeln!(out)?;
    }

    writeln!(
        out,
        "For full details, consult README.md or use the interactive mode."
    )?;
    Ok(())
}
