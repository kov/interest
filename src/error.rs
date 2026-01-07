//! Error handling for Interest TUI
//!
//! Defines custom error types and establishes a unified Result type
//! using anyhow for context chaining and error propagation.

use thiserror::Error;

/// Core error types for portfolio operations
#[derive(Error, Debug)]
pub enum PortfolioError {
    #[error("database error: {0}")]
    DbError(String),

    #[error("parse error: {0}")]
    ParseError(String),

    #[error("validation error: {0}")]
    ValidationError(String),

    #[error("pricing error: {0}")]
    PricingError(String),

    #[error("io error")]
    Io(#[from] std::io::Error),
}

/// Result type alias for portfolio operations
pub type Result<T> = anyhow::Result<T>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_formatting_is_readable() {
        let err = PortfolioError::DbError("connection failed".to_string());
        assert_eq!(err.to_string(), "database error: connection failed");
    }

    #[test]
    fn test_anyhow_context_chains_errors() {
        use anyhow::Context;
        let result: Result<()> =
            Err(anyhow::anyhow!("original error")).context("failed to process transaction");
        match result {
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("failed to process transaction"));
                // The chain information may be in the debug representation
                let debug_msg = format!("{:?}", e);
                assert!(debug_msg.contains("original error") || msg.contains("original error"));
            }
            Ok(_) => panic!("expected error"),
        }
    }

    #[test]
    fn test_portfolio_error_variants() {
        let db_err = PortfolioError::DbError("test".to_string());
        assert!(db_err.to_string().starts_with("database error"));

        let parse_err = PortfolioError::ParseError("test".to_string());
        assert!(parse_err.to_string().starts_with("parse error"));

        let pricing_err = PortfolioError::PricingError("test".to_string());
        assert!(pricing_err.to_string().starts_with("pricing error"));
    }
}
