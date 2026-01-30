//! JSON schema lock tests
//!
//! These tests ensure backward compatibility of JSON output across refactorings.
//! They verify:
//! 1. Required fields are present
//! 2. Field types are correct (especially Decimals as strings, not numbers)
//! 3. Date formatting is consistent
//!
//! Run these BEFORE and AFTER refactoring to ensure no breaking changes.

use serde_json::Value;
use tempfile::TempDir;

mod cli_helpers;
use cli_helpers::{
    add_asset, add_transaction, base_cmd, extract_table_rows, find_key_value_block, kv_value,
};

fn collect_value_nodes<'a>(blocks: &'a [Value], values: &mut Vec<&'a Value>) {
    for block in blocks {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("table") => {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(cells) = row.get("cells").and_then(|c| c.as_array()) {
                            for cell in cells {
                                values.push(cell);
                            }
                        }
                    }
                }
            }
            Some("key_value") => {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(value) = row.get("value") {
                            values.push(value);
                        }
                    }
                }
            }
            Some("section") => {
                if let Some(nested) = block.get("blocks").and_then(|b| b.as_array()) {
                    collect_value_nodes(nested, values);
                }
            }
            _ => {}
        }
    }
}

fn setup_test_portfolio(home: &TempDir) -> anyhow::Result<()> {
    // Add a test asset
    add_asset(home, "TEST4", "STOCK")?;

    // Add some transactions
    add_transaction(home, "TEST4", "BUY", "100", "10.50", "2024-01-15", false)?;
    add_transaction(home, "TEST4", "BUY", "50", "11.00", "2024-02-20", false)?;

    Ok(())
}

#[test]
fn test_portfolio_json_schema_stable() {
    // Set up test environment with sample data
    let home = TempDir::new().expect("Failed to create temp dir");
    setup_test_portfolio(&home).expect("Failed to set up test portfolio");

    // Run portfolio show --json
    let mut cmd = base_cmd(&home);
    cmd.args(["--json", "portfolio", "show"]);
    let output = cmd.output().expect("Failed to run command");

    // Command should succeed
    assert!(
        output.status.success(),
        "Command failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

    // Verify top-level structure
    assert!(json.is_object(), "JSON should be an object");
    let blocks = json
        .get("blocks")
        .and_then(|b| b.as_array())
        .expect("Should have blocks array");
    assert!(!blocks.is_empty(), "Should have at least one block");

    let positions =
        extract_table_rows(&json, None, Some("avg_cost")).expect("missing positions table");
    assert!(!positions.is_empty(), "expected positions table rows");
}

#[test]
fn test_portfolio_json_decimals_are_strings() {
    // Critical: Verify all Decimal fields are serialized as strings, not numbers

    let home = TempDir::new().expect("Failed to create temp dir");
    setup_test_portfolio(&home).expect("Failed to set up test portfolio");

    let mut cmd = base_cmd(&home);
    cmd.args(["--json", "portfolio", "show"]);
    let output = cmd.output().expect("Failed to run command");

    assert!(
        output.status.success(),
        "Command failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

    let blocks = json["blocks"].as_array().expect("blocks array missing");
    let mut has_currency = false;
    let mut has_quantity = false;

    let mut values = Vec::new();
    collect_value_nodes(blocks, &mut values);

    for value in values {
        match value.get("kind").and_then(|k| k.as_str()) {
            Some("currency") | Some("currency_delta") => {
                if let Some(val) = value.get("value") {
                    assert!(val.is_string());
                    has_currency = true;
                }
            }
            Some("quantity") => {
                if let Some(val) = value.get("value") {
                    assert!(val.is_string());
                    has_quantity = true;
                }
            }
            _ => {}
        }
    }

    assert!(has_currency, "expected currency fields in JSON");
    assert!(has_quantity, "expected quantity fields in JSON");
}

// =============================================================================
// Cashflow JSON Schema Tests
// =============================================================================

#[test]
fn test_cashflow_show_json_schema_stable() {
    let home = TempDir::new().expect("Failed to create temp dir");
    setup_test_portfolio(&home).expect("Failed to set up test portfolio");

    let mut cmd = base_cmd(&home);
    cmd.args(["--json", "cash-flow", "show", "2024"]);
    let output = cmd.output().expect("Failed to run command");

    assert!(
        output.status.success(),
        "Command failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

    // Verify top-level structure
    assert!(json.is_object(), "JSON should be an object");
    let yearly = extract_table_rows(&json, Some("Yearly Breakdown"), None)
        .expect("missing yearly breakdown table");
    assert!(!yearly.is_empty(), "expected yearly breakdown rows");

    let totals = find_key_value_block(&json, "Totals").expect("missing totals block");
    for label in ["Total In", "Total Out", "Net Flow"] {
        let value = kv_value(Some(totals), label);
        assert!(
            value.is_string() || value.is_number(),
            "missing totals field: {}",
            label
        );
    }
}

#[test]
fn test_cashflow_show_json_decimals_are_strings() {
    let home = TempDir::new().expect("Failed to create temp dir");
    setup_test_portfolio(&home).expect("Failed to set up test portfolio");

    let mut cmd = base_cmd(&home);
    cmd.args(["--json", "cash-flow", "show", "2024"]);
    let output = cmd.output().expect("Failed to run command");

    assert!(
        output.status.success(),
        "Command failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

    let blocks = json["blocks"].as_array().expect("blocks array missing");
    let mut values = Vec::new();
    collect_value_nodes(blocks, &mut values);

    let mut has_currency = false;
    for value in values {
        match value.get("kind").and_then(|k| k.as_str()) {
            Some("currency") | Some("currency_delta") | Some("percent") => {
                if let Some(val) = value.get("value") {
                    assert!(val.is_string());
                    has_currency = true;
                }
            }
            _ => {}
        }
    }

    assert!(has_currency, "expected currency fields in JSON");
}

#[test]
fn test_cashflow_stats_json_decimals_are_strings() {
    let home = TempDir::new().expect("Failed to create temp dir");
    setup_test_portfolio(&home).expect("Failed to set up test portfolio");

    let mut cmd = base_cmd(&home);
    cmd.args(["--json", "cash-flow", "stats", "2024"]);
    let output = cmd.output().expect("Failed to run command");

    assert!(
        output.status.success(),
        "Command failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

    let blocks = json["blocks"].as_array().expect("blocks array missing");
    let mut values = Vec::new();
    collect_value_nodes(blocks, &mut values);

    let mut has_currency = false;
    for value in values {
        match value.get("kind").and_then(|k| k.as_str()) {
            Some("currency") | Some("currency_delta") | Some("percent") => {
                if let Some(val) = value.get("value") {
                    assert!(val.is_string());
                    has_currency = true;
                }
            }
            _ => {}
        }
    }

    assert!(has_currency, "expected currency fields in JSON");
}

#[test]
fn test_portfolio_json_output_is_valid() {
    // Simple smoke test: verify portfolio JSON output is valid JSON

    let home = TempDir::new().expect("Failed to create temp dir");
    setup_test_portfolio(&home).expect("Failed to set up test portfolio");

    let mut cmd = base_cmd(&home);
    cmd.args(["--json", "portfolio", "show"]);
    let output = cmd.output().expect("Failed to run command");

    assert!(
        output.status.success(),
        "Command failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<Value>(&stdout).expect("Portfolio JSON output should be valid JSON");
}

// =============================================================================
// Performance JSON Schema Tests
// =============================================================================

#[test]
fn test_performance_json_schema_stable() {
    let home = TempDir::new().expect("Failed to create temp dir");
    setup_test_portfolio(&home).expect("Failed to set up test portfolio");

    let mut cmd = base_cmd(&home);
    cmd.args(["--json", "performance", "show", "YTD"]);
    let output = cmd.output().expect("Failed to run command");

    assert!(
        output.status.success(),
        "Command failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

    // Verify top-level structure
    assert!(json.is_object(), "JSON should be an object");
    let summary = find_key_value_block(&json, "Summary").expect("missing summary block");
    for label in [
        "Start Value",
        "End Value",
        "Return",
        "Realized Gains",
        "Unrealized Gains",
    ] {
        let value = kv_value(Some(summary), label);
        assert!(
            value.is_string() || value.is_number(),
            "missing summary field: {}",
            label
        );
    }

    let breakdown = extract_table_rows(&json, Some("By Asset Type"), None)
        .expect("missing asset breakdown table");
    assert!(!breakdown.is_empty(), "expected asset breakdown rows");
}

#[test]
fn test_performance_json_decimals_are_strings() {
    let home = TempDir::new().expect("Failed to create temp dir");
    setup_test_portfolio(&home).expect("Failed to set up test portfolio");

    let mut cmd = base_cmd(&home);
    cmd.args(["--json", "performance", "show", "YTD"]);
    let output = cmd.output().expect("Failed to run command");

    assert!(
        output.status.success(),
        "Command failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

    let blocks = json["blocks"].as_array().expect("blocks array missing");
    let mut values = Vec::new();
    collect_value_nodes(blocks, &mut values);

    let mut has_currency = false;
    for value in values {
        match value.get("kind").and_then(|k| k.as_str()) {
            Some("currency") | Some("currency_delta") | Some("percent") => {
                if let Some(val) = value.get("value") {
                    assert!(val.is_string());
                    has_currency = true;
                }
            }
            _ => {}
        }
    }

    assert!(has_currency, "expected currency fields in JSON");
}
