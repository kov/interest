//! Interest - Brazilian B3 stock exchange investment tracker
//!
//! This library provides functionality for tracking investments, calculating
//! taxes, and managing portfolio data for Brazilian stock market investments.

pub mod cli;
pub mod commands;
pub mod corporate_actions;
pub mod db;
pub mod dispatcher;
pub mod error;
pub mod importers;
pub mod pricing;
pub mod reports;
pub mod scraping;
pub mod tax;
pub mod term_contracts;
pub mod tickers;
pub mod ui;
pub mod utils;

// Re-export common result type
pub use error::Result;
