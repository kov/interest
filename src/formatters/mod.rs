//! Output formatting layer for all commands
//!
//! This module provides a unified approach to formatting output across CLI and TUI modes:
//!
//! ## Architecture
//!
//! - **Separation of concerns**: Dispatchers handle business logic, formatters handle presentation
//! - **Standard interface**: Formatters build an `OutputDocument` and render via OutputOptions
//! - **Consistent policies**: Renderer enforces decimal/date formatting rules
//!
//! ## Output Policies
//!
//! - Use `OutputDocument` to model output intent (tables, key/value blocks).
//! - Renderer decides JSON vs terminal formatting and alignment.
//!
//! ## Usage Pattern
//!
//! ```rust,ignore
//! // In dispatcher module:
//! pub async fn dispatch_command(options: OutputOptions) -> Result<()> {
//!     let report = calculate_report(...)?;
//!
//!     println!("{}", formatters::command::format(&report, options)?);
//!     Ok(())
//! }
//! ```
//!
//! For formatters that need extra parameters:
//!
//! ```rust,ignore
//! let options = OutputOptions::from_flags(json_output, false);
//! println!("{}", formatters::portfolio::format(&report, asset_type_filter, options)?);
//! ```
//!
//! ## Checklist for Adding New Formatters
//!
//! 1. Create `src/formatters/command_name.rs` with:
//!    - `pub fn format(&Report, ..., options: OutputOptions) -> Result<String>`
//!    - `fn build_*_document(&Report, ...) -> OutputDocument`
//!
//! 2. Add JSON schema lock test in `tests/json_schema_tests.rs`:
//!    - Verify field names/types before refactoring
//!    - Ensure decimals are strings, not numbers
//!
//! 3. Update dispatcher to use new API:
//!    - Remove inline `serde_json::json!` constructs
//!    - Remove inline `#[derive(Tabled)]` structs
//!    - Add: `let options = OutputOptions::from_flags(json_output, false);`
//!    - Call: `println!("{}", formatters::command::format(&data, options)?);`
//!
//! 4. Run before/after JSON diff to ensure backward compatibility
//!
//! 5. Export new formatter module below

pub mod actions;
pub mod assets;
pub mod cashflow;
pub mod imports;
pub mod income;
pub mod inconsistencies;
pub mod performance;
pub mod portfolio;
pub mod prices;
pub mod tax;
pub mod tickers;
pub mod transactions;

pub use crate::options::{OutputMode, OutputOptions};
use anyhow::Result;

/// Render an OutputDocument using the selected output mode.
pub fn render_document(
    doc: &crate::output::OutputDocument,
    options: OutputOptions,
) -> Result<String> {
    let primary_options = options.clone();
    let primary = match primary_options.output_mode {
        OutputMode::Table => {
            crate::output::terminal::TerminalRenderer::render(doc, primary_options)
        }
        OutputMode::Json => crate::output::json::JsonRenderer::render(doc, primary_options),
    };

    if let (Some(secondary_mode), Some(secondary_target)) = (
        options.secondary_output_mode,
        options.secondary_output.clone(),
    ) {
        let mut secondary_options = options.clone();
        secondary_options.output_mode = secondary_mode;
        secondary_options.output = secondary_target.clone();
        secondary_options.error_output = secondary_target.clone();
        secondary_options.secondary_output_mode = None;
        secondary_options.secondary_output = None;
        let secondary = match secondary_options.output_mode {
            OutputMode::Table => {
                crate::output::terminal::TerminalRenderer::render(doc, secondary_options)
            }
            OutputMode::Json => crate::output::json::JsonRenderer::render(doc, secondary_options),
        };
        secondary_target
            .write_all(secondary.as_bytes())
            .map_err(anyhow::Error::from)?;
        secondary_target
            .write_all(b"\n")
            .map_err(anyhow::Error::from)?;
    }

    Ok(primary)
}
