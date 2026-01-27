use anyhow::Result;

pub async fn dispatch_process_terms() -> Result<()> {
    use colored::Colorize;

    println!(
        "{} Processing term contract liquidations...\n",
        "🔄".cyan().bold()
    );

    // Initialize database
    crate::db::init_database(None)?;
    let conn = crate::db::open_db(None)?;

    // Process term liquidations
    let processed = crate::term_contracts::process_term_liquidations(&conn)?;

    if processed == 0 {
        println!("{} No term contract liquidations found", "ℹ".blue().bold());
        println!("\nTerm contracts are identified by transactions with notes containing");
        println!("'Term contract liquidation' and show the TICKERT → TICKER transition.");
    } else {
        println!(
            "\n{} Successfully processed {} term contract liquidation(s)!",
            "✓".green().bold(),
            processed
        );
        println!("\nCost basis from TICKERT purchases has been matched to TICKER liquidations.");
    }

    Ok(())
}
