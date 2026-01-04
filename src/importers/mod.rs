// Import module - B3/CEI Excel and CSV parsers

pub mod cei_excel;
pub mod cei_csv;
pub mod movimentacao_excel;
pub mod irpf_pdf;
mod file_detector;

use anyhow::{anyhow, Result};
use std::path::Path;
use tracing::info;

pub use cei_excel::RawTransaction;
pub use movimentacao_excel::MovimentacaoEntry;
pub use irpf_pdf::IrpfPosition;
pub use file_detector::FileType;

/// Result of importing a file with auto-detection
#[derive(Debug)]
pub enum ImportResult {
    Cei(Vec<RawTransaction>),
    Movimentacao(Vec<MovimentacaoEntry>),
    IrpfPositions(Vec<IrpfPosition>),
}

/// Import file with automatic format detection
///
/// Detects whether the file is CEI or Movimentacao format based on content,
/// then parses accordingly. Returns an ImportResult indicating which format
/// was detected and the parsed data.
pub fn import_file_auto<P: AsRef<Path>>(path: P) -> Result<ImportResult> {
    let path_ref = path.as_ref();

    // Detect file type from contents
    let file_type = file_detector::detect_file_type(path_ref)?;

    match file_type {
        FileType::Cei => {
            let transactions = import_file(path_ref)?;
            Ok(ImportResult::Cei(transactions))
        }
        FileType::Movimentacao => {
            let entries = movimentacao_excel::parse_movimentacao_excel(path_ref)?;
            Ok(ImportResult::Movimentacao(entries))
        }
    }
}

/// Import transactions from a CEI file (auto-detects Excel vs CSV)
pub fn import_file<P: AsRef<Path>>(file_path: P) -> Result<Vec<RawTransaction>> {
    let path = file_path.as_ref();
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| anyhow!("File has no extension"))?
        .to_lowercase();

    info!("Importing CEI file: {:?} (type: {})", path, extension);

    match extension.as_str() {
        "xlsx" | "xls" => cei_excel::parse_cei_excel(path),
        "csv" | "txt" => cei_csv::parse_cei_csv(path),
        _ => Err(anyhow!(
            "Unsupported file format: {}. Supported formats: .xlsx, .xls, .csv",
            extension
        )),
    }
}
