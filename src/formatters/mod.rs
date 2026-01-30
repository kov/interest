//! Output formatting layer for all commands
//!
//! This module provides a unified approach to formatting output across CLI and TUI modes:
//!
//! ## Architecture
//!
//! - **Separation of concerns**: Dispatchers handle business logic, formatters handle presentation
//! - **Standard interface**: Formatters build an `OutputDocument` and render via OutputMode
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
//! pub async fn dispatch_command(json_output: bool) -> Result<()> {
//!     let report = calculate_report(...)?;
//!
//!     let mode = formatters::OutputMode::from_json_flag(json_output);
//!     println!("{}", formatters::command::format(&report, mode));
//!     Ok(())
//! }
//! ```
//!
//! For formatters that need extra parameters:
//!
//! ```rust,ignore
//! let mode = formatters::OutputMode::from_json_flag(json_output);
//! println!("{}", formatters::portfolio::format(&report, asset_type_filter, mode));
//! ```
//!
//! ## Checklist for Adding New Formatters
//!
//! 1. Create `src/formatters/command_name.rs` with:
//!    - `pub fn format(&Report, ..., mode: OutputMode) -> String`
//!    - `fn build_*_document(&Report, ...) -> OutputDocument`
//!
//! 2. Add JSON schema lock test in `tests/json_schema_tests.rs`:
//!    - Verify field names/types before refactoring
//!    - Ensure decimals are strings, not numbers
//!
//! 3. Update dispatcher to use new API:
//!    - Remove inline `serde_json::json!` constructs
//!    - Remove inline `#[derive(Tabled)]` structs
//!    - Add: `let mode = formatters::OutputMode::from_json_flag(json_output);`
//!    - Call: `println!("{}", formatters::command::format(&data, mode));`
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

/// Output format for command results
///
/// This enum provides a type-safe, extensible alternative to boolean flags.
/// Future formats (CSV, HTML, YAML) can be added without breaking existing code.
///
/// # Example
///
/// ```ignore
/// let mode = OutputMode::from_json_flag(json_output);
/// println!("{}", formatters::portfolio::format(&report, asset_type, mode));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Human-readable terminal table (default)
    Table,
    /// JSON (machine-readable, preserves all precision)
    Json,
}

impl OutputMode {
    /// Create from legacy boolean flag
    ///
    /// This helper enables gradual migration from `bool` to `OutputMode`.
    ///
    /// # Example
    ///
    /// ```
    /// use interest::formatters::OutputMode;
    ///
    /// assert_eq!(OutputMode::from_json_flag(false), OutputMode::Table);
    /// assert_eq!(OutputMode::from_json_flag(true), OutputMode::Json);
    /// ```
    pub fn from_json_flag(json_output: bool) -> Self {
        if json_output {
            Self::Json
        } else {
            Self::Table
        }
    }
}

/// Render an OutputDocument using the selected output mode.
pub fn render_document(doc: &crate::output::OutputDocument, mode: OutputMode) -> String {
    match mode {
        OutputMode::Table => crate::output::terminal::TerminalRenderer::render(doc),
        OutputMode::Json => crate::output::json::JsonRenderer::render(doc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_mode_from_json_flag() {
        assert_eq!(OutputMode::from_json_flag(false), OutputMode::Table);
        assert_eq!(OutputMode::from_json_flag(true), OutputMode::Json);
    }

    #[test]
    fn test_output_mode_equality() {
        assert_eq!(OutputMode::Table, OutputMode::Table);
        assert_eq!(OutputMode::Json, OutputMode::Json);
        assert_ne!(OutputMode::Table, OutputMode::Json);
    }

    #[test]
    fn test_output_mode_debug() {
        // Verify Debug implementation works
        assert_eq!(format!("{:?}", OutputMode::Table), "Table");
        assert_eq!(format!("{:?}", OutputMode::Json), "Json");
    }
}
