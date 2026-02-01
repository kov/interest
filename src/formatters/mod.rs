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
//!     println!("{}", formatters::command::format(&report, options));
//!     Ok(())
//! }
//! ```
//!
//! For formatters that need extra parameters:
//!
//! ```rust,ignore
//! let options = OutputOptions::from_flags(json_output, false);
//! println!("{}", formatters::portfolio::format(&report, asset_type_filter, options));
//! ```
//!
//! ## Checklist for Adding New Formatters
//!
//! 1. Create `src/formatters/command_name.rs` with:
//!    - `pub fn format(&Report, ..., options: OutputOptions) -> String`
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
//!    - Call: `println!("{}", formatters::command::format(&data, options));`
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

/// Render an OutputDocument using the selected output mode.
pub fn render_document(doc: &crate::output::OutputDocument, options: OutputOptions) -> String {
    match options.output_mode {
        OutputMode::Table => crate::output::terminal::TerminalRenderer::render(doc, options),
        OutputMode::Json => crate::output::json::JsonRenderer::render(doc, options),
    }
}
