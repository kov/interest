use anyhow::Result;

pub async fn dispatch_inspect(file_path: &str, full: bool, column: Option<usize>) -> Result<()> {
    use anyhow::Context;
    use calamine::{open_workbook, Data, Reader, Xlsx};
    use colored::Colorize;
    use std::collections::HashMap;

    println!(
        "{} Inspecting file: {}\n",
        "📊".cyan().bold(),
        file_path.green()
    );

    let mut workbook: Xlsx<_> = open_workbook(file_path).context("Failed to open Excel file")?;

    let sheet_names = workbook.sheet_names().to_vec();
    println!(
        "{} Found {} sheet(s):",
        "📄".cyan().bold(),
        sheet_names.len()
    );
    for name in &sheet_names {
        println!("  • {}", name.yellow());
    }
    println!();

    let mut sheet_stats: HashMap<String, (usize, usize)> = HashMap::new();

    for sheet_name in sheet_names {
        let range = match workbook.worksheet_range(&sheet_name) {
            Ok(range) => range,
            Err(err) => {
                eprintln!(
                    "{} Failed to read sheet {}: {}",
                    "⚠️".yellow(),
                    sheet_name.yellow(),
                    err
                );
                continue;
            }
        };

        {
            let rows = range.height();
            let cols = range.width();
            sheet_stats.insert(sheet_name.clone(), (rows, cols));

            println!(
                "{} Sheet: {}",
                "📌".cyan().bold(),
                sheet_name.yellow().bold()
            );
            println!("  Rows: {}", rows);
            println!("  Columns: {}\n", cols);

            if full || column.is_some() {
                for (row_idx, row) in range.rows().enumerate() {
                    if let Some(col_idx) = column {
                        if let Some(cell) = row.get(col_idx) {
                            if cell != &Data::Empty {
                                println!("  Row {}: {:?}", row_idx + 1, cell);
                            }
                        }
                    } else {
                        println!("  Row {}:", row_idx + 1);
                        for (col_idx, cell) in row.iter().enumerate() {
                            if cell != &Data::Empty {
                                println!("    Col {}: {:?}", col_idx + 1, cell);
                            }
                        }
                    }
                }
                println!();
            }
        }
    }

    if !full && column.is_none() {
        println!("{}", "Tip: Use --full to see all data".blue());
        println!(
            "{}",
            "Tip: Use --column <n> to inspect a specific column".blue()
        );
    }

    Ok(())
}
