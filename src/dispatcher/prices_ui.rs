use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::Connection;

use crate::db::Asset;
use crate::options::OutputOptions;
use crate::ui::progress::{ProgressEvent, ProgressPrinter};

/// Fetch prices for a set of assets, showing progress in interactive mode
/// and running silently in JSON mode. Errors are logged and swallowed so
/// that reports can still be generated with whatever prices are available.
pub async fn ensure_prices_with_ui(
    conn: &mut Connection,
    assets: &[Asset],
    price_range: (NaiveDate, NaiveDate),
    options: &OutputOptions,
) -> Result<()> {
    let skip = std::env::var("INTEREST_SKIP_PRICE_FETCH")
        .map(|v| v != "0")
        .unwrap_or(false);

    if skip || assets.is_empty() {
        return Ok(());
    }

    let priceable = crate::pricing::resolver::filter_priceable_assets(assets);

    if priceable.is_empty() {
        return Ok(());
    }

    if !options.is_json() {
        let total = priceable.len();
        let printer = ProgressPrinter::new(options);

        printer.handle_event(&ProgressEvent::Spinner {
            message: format!("Fetching prices 0/{}...", total),
        });

        let is_private = options.is_private();
        crate::pricing::resolver::ensure_prices_available_with_progress(
            conn,
            &priceable,
            price_range,
            options.clone(),
            |event| match event {
                ProgressEvent::TickerResult {
                    ticker,
                    price,
                    current,
                    total,
                } => {
                    let masked = if is_private {
                        ProgressEvent::TickerResult {
                            ticker: ticker.clone(),
                            price: price
                                .as_ref()
                                .map(|_| "R$ ***".to_string())
                                .map_err(|err| err.clone()),
                            current: *current,
                            total: *total,
                        }
                    } else {
                        ProgressEvent::TickerResult {
                            ticker: ticker.clone(),
                            price: price.clone(),
                            current: *current,
                            total: *total,
                        }
                    };
                    printer.handle_event(&masked);
                    printer.handle_event(&ProgressEvent::Spinner {
                        message: format!("Fetching prices {}/{}...", current, total),
                    });
                }
                _ => printer.handle_event(event),
            },
        )
        .await
        .or_else(|e: anyhow::Error| {
            tracing::warn!("Price resolution failed: {}", e);
            Ok::<(), anyhow::Error>(())
        })?;

        printer.handle_event(&ProgressEvent::Success {
            message: "Price resolution complete".to_string(),
        });
    } else {
        crate::pricing::resolver::ensure_prices_available(
            conn,
            &priceable,
            price_range,
            options.clone(),
        )
        .await
        .or_else(|e: anyhow::Error| {
            tracing::warn!("Price resolution failed: {}", e);
            Ok::<(), anyhow::Error>(())
        })?;
    }

    Ok(())
}
