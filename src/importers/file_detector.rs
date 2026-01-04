use anyhow::{anyhow, Context, Result};
use calamine::{open_workbook, Reader, Xlsx, DataType};
use std::path::Path;
use tracing::info;

/// Type of import file detected
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    Cei,
    Movimentacao,
    OfertasPublicas,
}

/// Detect the type of import file based on its contents
///
/// Detection strategy:
/// - CSV/TXT files → Always CEI format (Movimentacao only supports Excel)
/// - Excel files → Check sheet names:
///   - "Movimentação" → Movimentacao format
///   - "negociação", "ativos", "trading", etc → CEI format
///   - Unknown → Error with helpful message
pub fn detect_file_type<P: AsRef<Path>>(path: P) -> Result<FileType> {
    let path = path.as_ref();
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| anyhow!("File has no extension"))?
        .to_lowercase();

    // CSV/TXT files are always CEI format
    if matches!(extension.as_str(), "csv" | "txt") {
        info!("Detected CEI format (CSV/TXT file)");
        return Ok(FileType::Cei);
    }

    // For Excel files, check sheet names
    if matches!(extension.as_str(), "xlsx" | "xls") {
        let workbook: Xlsx<_> = open_workbook(path)
            .context("Failed to open Excel file for type detection")?;
        let sheet_names = workbook.sheet_names();

        info!("Examining Excel sheets: {:?}", sheet_names);

        // Check for Movimentacao sheet (may also be Ofertas Públicas)
        if sheet_names.iter().any(|name| name == "Movimentação") {
            let mut workbook: Xlsx<_> = open_workbook(path)
                .context("Failed to open Excel file for header detection")?;
            if let Ok(range) = workbook.worksheet_range("Movimentação") {
                if let Some(header_row) = range.rows().next() {
                    let headers: Vec<String> = header_row
                        .iter()
                        .filter_map(|cell| cell.get_string())
                        .map(|s| s.trim().to_string())
                        .collect();
                    let has_ofertas_header = headers.iter().any(|h| {
                        matches!(
                            h.as_str(),
                            "Oferta"
                                | "Modalidade de Reserva"
                                | "Preço Máximo"
                                | "Data de liquidação"
                                | "Quantidade Reservada"
                        )
                    });
                    if has_ofertas_header {
                        info!("Detected Ofertas Públicas format (Movimentação sheet with ofertas headers)");
                        return Ok(FileType::OfertasPublicas);
                    }
                }
            }

            info!("Detected Movimentacao format (found 'Movimentação' sheet)");
            return Ok(FileType::Movimentacao);
        }

        // Check for CEI trading sheets (case-insensitive pattern matching)
        let cei_patterns = ["negociação", "negociacao", "ativos", "trading", "trades"];
        for sheet_name in &sheet_names {
            let lower = sheet_name.to_lowercase();
            for pattern in &cei_patterns {
                if lower.contains(pattern) {
                    info!("Detected CEI format (found trading sheet: '{}')", sheet_name);
                    return Ok(FileType::Cei);
                }
            }
        }

        // Unknown format - provide helpful error
        return Err(anyhow!(
            "Could not determine file type.\n\
             Found sheets: {:?}\n\
             Expected either:\n  \
             - CEI format with sheets matching: negociação, ativos, trading\n  \
             - Movimentacao format with sheet: Movimentação\n  \
             - Ofertas Públicas format with sheet: Movimentação + oferta headers",
            sheet_names
        ));
    }

    Err(anyhow!("Unsupported file extension: {}", extension))
}
