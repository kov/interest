// Import module - B3/CEI Excel and CSV parsers

pub mod cei_excel;
pub mod cei_csv;

use anyhow::{anyhow, Result};
use std::path::Path;
use tracing::info;

pub use cei_excel::RawTransaction;

/// Import transactions from a file (auto-detects Excel vs CSV)
pub fn import_file<P: AsRef<Path>>(file_path: P) -> Result<Vec<RawTransaction>> {
    let path = file_path.as_ref();
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| anyhow!("File has no extension"))?
        .to_lowercase();

    info!("Importing file: {:?} (type: {})", path, extension);

    match extension.as_str() {
        "xlsx" | "xls" => cei_excel::parse_cei_excel(path),
        "csv" | "txt" => cei_csv::parse_cei_csv(path),
        _ => Err(anyhow!(
            "Unsupported file format: {}. Supported formats: .xlsx, .xls, .csv",
            extension
        )),
    }
}
