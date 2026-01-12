// Import module - B3/CEI Excel and CSV parsers

pub mod b3_cotahist;
pub mod cei_csv;
pub mod cei_excel;
mod file_detector;
pub mod irpf_pdf;
pub mod movimentacao_excel;
pub mod movimentacao_import;
pub mod ofertas_publicas_excel;
pub mod validation;

use anyhow::{anyhow, Result};
use std::path::Path;
use tracing::info;

pub use cei_excel::RawTransaction;
pub use file_detector::FileType;
pub use movimentacao_excel::MovimentacaoEntry;
pub use movimentacao_import::import_movimentacao_entries;
pub use ofertas_publicas_excel::OfertaPublicaEntry;

use chrono::NaiveDate;
use serde::Serialize;

/// Unified import statistics shared across import formats
#[derive(Serialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportStats {
    // CEI / Ofertas 'generic' imported count
    pub imported: usize,
    pub skipped_old: usize,

    // Movimentacao-specific counts
    pub imported_trades: usize,
    pub skipped_trades: usize,
    pub skipped_trades_old: usize,
    pub imported_actions: usize,
    pub skipped_actions: usize,
    pub skipped_actions_old: usize,
    pub auto_applied_actions: usize,
    pub imported_income: usize,
    pub skipped_income: usize,
    pub skipped_income_old: usize,

    pub errors: usize,

    pub earliest: Option<NaiveDate>,
    pub latest: Option<NaiveDate>,
}

/// Result of importing a file with auto-detection
#[derive(Debug)]
pub enum ImportResult {
    Cei(Vec<RawTransaction>),
    Movimentacao(Vec<MovimentacaoEntry>),
    OfertasPublicas(Vec<OfertaPublicaEntry>),
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
        FileType::OfertasPublicas => {
            let entries = ofertas_publicas_excel::parse_ofertas_publicas_excel(path_ref)?;
            Ok(ImportResult::OfertasPublicas(entries))
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
