//! Generate test XLS files for the interest tracker test suite.
//!
//! Run with: cargo test --test generate_test_files -- --ignored
//!
//! These files test various scenarios:
//! 1. Basic purchases and sales
//! 2. Term contracts (purchase with T suffix, liquidation)
//! 3. Splits and reverse splits
//! 4. Capital returns
//! 5. Complex scenarios with multiple events

use rust_xlsxwriter::Workbook;

fn create_workbook_with_header() -> Workbook {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Movimentação").unwrap();

    // Header row
    let headers = [
        "Entrada/Saída",
        "Data",
        "Movimentação",
        "Produto",
        "Instituição",
        "Quantidade",
        "Preço unitário",
        "Valor da Operação",
    ];

    for (col, header) in headers.iter().enumerate() {
        worksheet.write_string(0, col as u16, *header).unwrap();
    }

    workbook
}

#[test]
#[ignore]
fn generate_13_ofertas_publicas() {
    /*
    Test ofertas públicas import with L-suffix normalization.

    Scenario:
    - AMBP3L allocation 1064 @ 13.25 on 06/11/2023
    - Expect normalized ticker AMBP3 in import
    */
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Movimentação").unwrap();

    let headers = vec![
        "Data de liquidação",
        "Empresa",
        "Tipo",
        "Oferta",
        "Instituição",
        "Quantidade",
        "Preço",
        "Valor",
        "Código de Negociação",
        "Preço Máximo",
        "Modalidade de Reserva",
        "Quantidade Reservada",
        "Valor Reservado",
    ];

    for (col, header) in headers.iter().enumerate() {
        worksheet.write_string(0, col as u16, *header).unwrap();
    }

    worksheet.write_string(1, 0, "06/11/2023").unwrap();
    worksheet
        .write_string(1, 1, "AMBIPAR PARTICIPACOES E EMPREENDIMENTOS S/A")
        .unwrap();
    worksheet.write_string(1, 2, "OUTRO").unwrap();
    worksheet.write_string(1, 3, "Ambipar S.A. (P)").unwrap();
    worksheet
        .write_string(1, 4, "XP INVESTIMENTOS CCTVM S/A")
        .unwrap();
    worksheet.write_number(1, 5, 1064.0).unwrap();
    worksheet.write_number(1, 6, 13.25).unwrap();
    worksheet.write_number(1, 7, 14098.0).unwrap();
    worksheet.write_string(1, 8, "AMBP3L").unwrap();
    worksheet.write_number(1, 9, 15.0).unwrap();
    worksheet
        .write_string(1, 10, "Acionista compra até o LSP")
        .unwrap();
    worksheet.write_number(1, 11, 1064.0).unwrap();
    worksheet.write_number(1, 12, 0.0).unwrap();

    workbook
        .save("tests/data/13_ofertas_publicas.xlsx")
        .unwrap();
    println!("✓ Created: 13_ofertas_publicas.xlsx");
}

#[test]
#[ignore]
fn generate_12_desdobro_inference() {
    /*
    Test Desdobro ratio inference from credited quantity.

    Scenario:
    - Buy 80 A1MD34 @ R$10.00 on 2022-11-20
    - Desdobro on 2022-11-22 with credit of 560 shares
      - Expected ratio: 1:8 (80 -> 640)
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    // Buy 80 A1MD34 @ R$10.00
    worksheet.write_string(1, 0, "Credito").unwrap();
    worksheet.write_string(1, 1, "20/11/2022").unwrap();
    worksheet.write_string(1, 2, "Compra").unwrap();
    worksheet
        .write_string(1, 3, "A1MD34 - ADVANCED MICRO DEVICES INC")
        .unwrap();
    worksheet.write_string(1, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(1, 5, 80.0).unwrap();
    worksheet.write_number(1, 6, 10.0).unwrap();
    worksheet.write_number(1, 7, 800.0).unwrap();

    // Desdobro credit of 560 shares
    worksheet.write_string(2, 0, "Credito").unwrap();
    worksheet.write_string(2, 1, "22/11/2022").unwrap();
    worksheet.write_string(2, 2, "Desdobro").unwrap();
    worksheet
        .write_string(2, 3, "A1MD34 - ADVANCED MICRO DEVICES INC")
        .unwrap();
    worksheet.write_string(2, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(2, 5, 560.0).unwrap();
    worksheet.write_string(2, 6, "").unwrap();
    worksheet.write_string(2, 7, "").unwrap();

    workbook
        .save("tests/data/12_desdobro_inference.xlsx")
        .unwrap();
    println!("✓ Created: 12_desdobro_inference.xlsx");
}

#[test]
#[ignore]
fn generate_14_atualizacao_inference() {
    /*
    Test Atualização ratio inference from credited quantity.

    Scenario:
    - Buy 378 BRCR11 @ R$10.00 on 2020-09-10
    - Atualização credit of 22 shares on 2020-09-14
      - Expected ratio: 378:400 (bonus-style adjustment)
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    // Buy 378 BRCR11 @ R$10.00
    worksheet.write_string(1, 0, "Credito").unwrap();
    worksheet.write_string(1, 1, "10/09/2020").unwrap();
    worksheet.write_string(1, 2, "Compra").unwrap();
    worksheet
        .write_string(1, 3, "BRCR11 - BTG PACTUAL CORP. OFFICE FUND")
        .unwrap();
    worksheet.write_string(1, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(1, 5, 378.0).unwrap();
    worksheet.write_number(1, 6, 10.0).unwrap();
    worksheet.write_number(1, 7, 3780.0).unwrap();

    // Atualização credit of 22 shares
    worksheet.write_string(2, 0, "Credito").unwrap();
    worksheet.write_string(2, 1, "14/09/2020").unwrap();
    worksheet.write_string(2, 2, "Atualização").unwrap();
    worksheet
        .write_string(2, 3, "BRCR11 - BTG PACTUAL CORP. OFFICE FUND")
        .unwrap();
    worksheet.write_string(2, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(2, 5, 22.0).unwrap();
    worksheet.write_string(2, 6, "").unwrap();
    worksheet.write_string(2, 7, "").unwrap();

    workbook
        .save("tests/data/14_atualizacao_inference.xlsx")
        .unwrap();
    println!("✓ Created: 14_atualizacao_inference.xlsx");
}

#[test]
#[ignore]
fn generate_10_duplicate_trades() {
    /*
    Test duplicate trades (same date/qty/price) should both be imported.

    Scenario:
    - Buy 10 DUPL3 @ R$10.00 on 2025-01-10
    - Buy 10 DUPL3 @ R$10.00 on 2025-01-10 (duplicate row)
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    // Buy 10 DUPL3 @ R$10.00
    worksheet.write_string(1, 0, "Credito").unwrap();
    worksheet.write_string(1, 1, "10/01/2025").unwrap();
    worksheet.write_string(1, 2, "Compra").unwrap();
    worksheet.write_string(1, 3, "DUPL3 - DUPLICA SA").unwrap();
    worksheet.write_string(1, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(1, 5, 10.0).unwrap();
    worksheet.write_number(1, 6, 10.0).unwrap();
    worksheet.write_number(1, 7, 100.0).unwrap();

    // Duplicate buy row
    worksheet.write_string(2, 0, "Credito").unwrap();
    worksheet.write_string(2, 1, "10/01/2025").unwrap();
    worksheet.write_string(2, 2, "Compra").unwrap();
    worksheet.write_string(2, 3, "DUPL3 - DUPLICA SA").unwrap();
    worksheet.write_string(2, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(2, 5, 10.0).unwrap();
    worksheet.write_number(2, 6, 10.0).unwrap();
    worksheet.write_number(2, 7, 100.0).unwrap();

    workbook
        .save("tests/data/10_duplicate_trades.xlsx")
        .unwrap();
    println!("✓ Created: 10_duplicate_trades.xlsx");
}

#[test]
#[ignore]
fn generate_11_bonus_auto_apply() {
    /*
    Test bonus corporate action auto-apply.

    Scenario:
    - Buy 100 ITSA4 @ R$10.00 on 2021-01-10
    - Bonus 20% on 2021-12-22 (quantity field = 20)
      - Expected: 120 shares, total cost unchanged
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    // Buy 100 ITSA4 @ R$10.00
    worksheet.write_string(1, 0, "Credito").unwrap();
    worksheet.write_string(1, 1, "10/01/2021").unwrap();
    worksheet.write_string(1, 2, "Compra").unwrap();
    worksheet.write_string(1, 3, "ITSA4 - ITAUSA PN").unwrap();
    worksheet.write_string(1, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(1, 5, 100.0).unwrap();
    worksheet.write_number(1, 6, 10.0).unwrap();
    worksheet.write_number(1, 7, 1000.0).unwrap();

    // Bonus 20% (Bonificacao em Ativos)
    worksheet.write_string(2, 0, "Credito").unwrap();
    worksheet.write_string(2, 1, "22/12/2021").unwrap();
    worksheet
        .write_string(2, 2, "Bonificação em Ativos")
        .unwrap();
    worksheet.write_string(2, 3, "ITSA4 - ITAUSA PN").unwrap();
    worksheet.write_string(2, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(2, 5, 20.0).unwrap();
    worksheet.write_string(2, 6, "").unwrap();
    worksheet.write_string(2, 7, "").unwrap();

    workbook
        .save("tests/data/11_bonus_auto_apply.xlsx")
        .unwrap();
    println!("✓ Created: 11_bonus_auto_apply.xlsx");
}

#[test]
#[ignore]
fn generate_01_basic_purchase_and_sale() {
    /*
    Test basic purchase and sale with cost basis calculation.

    Scenario:
    - Buy 100 PETR4 @ R$25.00 = R$2,500.00 on 2025-01-10
    - Buy 50 PETR4 @ R$30.00 = R$1,500.00 on 2025-02-15
    - Sell 80 PETR4 @ R$35.00 = R$2,800.00 on 2025-03-20
      - Average cost basis applied to the sale
      - Profit: R$2,800.00 - R$2,000.00 = R$800.00
    - Remaining: 20 from first lot + 50 from second lot = 70 shares
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    // Buy 100 PETR4 @ R$25.00
    worksheet.write_string(1, 0, "Credito").unwrap();
    worksheet.write_string(1, 1, "10/01/2025").unwrap();
    worksheet.write_string(1, 2, "Compra").unwrap();
    worksheet
        .write_string(1, 3, "PETR4 - PETROBRAS PN")
        .unwrap();
    worksheet.write_string(1, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(1, 5, 100.0).unwrap();
    worksheet.write_number(1, 6, 25.0).unwrap();
    worksheet.write_number(1, 7, 2500.0).unwrap();

    // Buy 50 PETR4 @ R$30.00
    worksheet.write_string(2, 0, "Credito").unwrap();
    worksheet.write_string(2, 1, "15/02/2025").unwrap();
    worksheet.write_string(2, 2, "Compra").unwrap();
    worksheet
        .write_string(2, 3, "PETR4 - PETROBRAS PN")
        .unwrap();
    worksheet.write_string(2, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(2, 5, 50.0).unwrap();
    worksheet.write_number(2, 6, 30.0).unwrap();
    worksheet.write_number(2, 7, 1500.0).unwrap();

    // Sell 80 PETR4 @ R$35.00
    worksheet.write_string(3, 0, "Debito").unwrap();
    worksheet.write_string(3, 1, "20/03/2025").unwrap();
    worksheet.write_string(3, 2, "Venda").unwrap();
    worksheet
        .write_string(3, 3, "PETR4 - PETROBRAS PN")
        .unwrap();
    worksheet.write_string(3, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(3, 5, 80.0).unwrap();
    worksheet.write_number(3, 6, 35.0).unwrap();
    worksheet.write_number(3, 7, 2800.0).unwrap();

    workbook
        .save("tests/data/01_basic_purchase_sale.xlsx")
        .unwrap();
    println!("✓ Created: 01_basic_purchase_sale.xlsx");
}

#[test]
#[ignore]
fn generate_02_term_contract_lifecycle() {
    /*
    Test term contract purchase, expiry, and sale.

    Scenario:
    - Buy 200 ANIM3T (term) @ R$10.00 = R$2,000.00 on 2025-01-15
    - Term liquidation: 200 ANIM3T -> ANIM3 on 2025-02-28
      - Cost basis transfers from ANIM3T to ANIM3 @ R$10.00
    - Sell 100 ANIM3 @ R$12.00 = R$1,200.00 on 2025-03-15
      - Cost basis: 100 @ R$10.00 = R$1,000.00
      - Profit: R$200.00
    - Remaining: 100 ANIM3 @ R$10.00
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    // Buy term contract ANIM3T @ R$10.00
    worksheet.write_string(1, 0, "Credito").unwrap();
    worksheet.write_string(1, 1, "15/01/2025").unwrap();
    worksheet.write_string(1, 2, "Compra").unwrap();
    worksheet
        .write_string(1, 3, "Termo de Ação ANIM3 - ANIM3T - ANIM")
        .unwrap();
    worksheet.write_string(1, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(1, 5, 200.0).unwrap();
    worksheet.write_number(1, 6, 10.0).unwrap();
    worksheet.write_number(1, 7, 2000.0).unwrap();

    // Term liquidation - receives ANIM3 shares
    worksheet.write_string(2, 0, "Credito").unwrap();
    worksheet.write_string(2, 1, "28/02/2025").unwrap();
    worksheet.write_string(2, 2, "Liquidação Termo").unwrap();
    worksheet
        .write_string(2, 3, "ANIM3 - ANIMA HOLDING")
        .unwrap();
    worksheet.write_string(2, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(2, 5, 200.0).unwrap();
    worksheet.write_number(2, 6, 10.0).unwrap();
    worksheet.write_number(2, 7, 2000.0).unwrap();

    // Sell ANIM3 @ R$12.00
    worksheet.write_string(3, 0, "Debito").unwrap();
    worksheet.write_string(3, 1, "15/03/2025").unwrap();
    worksheet.write_string(3, 2, "Venda").unwrap();
    worksheet
        .write_string(3, 3, "ANIM3 - ANIMA HOLDING")
        .unwrap();
    worksheet.write_string(3, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(3, 5, 100.0).unwrap();
    worksheet.write_number(3, 6, 12.0).unwrap();
    worksheet.write_number(3, 7, 1200.0).unwrap();

    workbook
        .save("tests/data/02_term_contract_lifecycle.xlsx")
        .unwrap();
    println!("✓ Created: 02_term_contract_lifecycle.xlsx");
}

#[test]
#[ignore]
fn generate_03_term_contract_sold_before_expiry() {
    /*
    Test selling a term contract before it expires.

    Scenario:
    - Buy 150 SHUL4T (term) @ R$8.00 = R$1,200.00 on 2025-01-10
    - Sell 150 SHUL4T (term) @ R$9.00 = R$1,350.00 on 2025-02-05
      - Profit: R$150.00
    - No liquidation (sold before expiry)
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    // Buy term contract
    worksheet.write_string(1, 0, "Credito").unwrap();
    worksheet.write_string(1, 1, "10/01/2025").unwrap();
    worksheet.write_string(1, 2, "Compra").unwrap();
    worksheet
        .write_string(1, 3, "Termo de Ação SHUL4 - SHUL4T - SHUL")
        .unwrap();
    worksheet.write_string(1, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(1, 5, 150.0).unwrap();
    worksheet.write_number(1, 6, 8.0).unwrap();
    worksheet.write_number(1, 7, 1200.0).unwrap();

    // Sell term contract before expiry
    worksheet.write_string(2, 0, "Debito").unwrap();
    worksheet.write_string(2, 1, "05/02/2025").unwrap();
    worksheet.write_string(2, 2, "Venda").unwrap();
    worksheet
        .write_string(2, 3, "Termo de Ação SHUL4 - SHUL4T - SHUL")
        .unwrap();
    worksheet.write_string(2, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(2, 5, 150.0).unwrap();
    worksheet.write_number(2, 6, 9.0).unwrap();
    worksheet.write_number(2, 7, 1350.0).unwrap();

    workbook
        .save("tests/data/03_term_contract_sold.xlsx")
        .unwrap();
    println!("✓ Created: 03_term_contract_sold.xlsx");
}

#[test]
#[ignore]
fn generate_04_stock_split() {
    /*
    Test stock split (desdobro) adjustment.

    Scenario:
    - Buy 100 VALE3 @ R$80.00 = R$8,000.00 on 2025-01-10
    - Split 1:2 on 2025-02-15 (each share becomes 2)
      - Adjusted: 200 VALE3 @ R$40.00 = R$8,000.00 (cost unchanged)
    - Buy 50 VALE3 @ R$42.00 = R$2,100.00 on 2025-03-01
    - Sell 150 VALE3 @ R$45.00 = R$6,750.00 on 2025-04-10
      - Average cost basis applied after split adjustments
      - Profit: R$750.00
    - Remaining: 50 from first lot + 50 from second lot = 100 shares
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    // Buy 100 VALE3 @ R$80.00
    worksheet.write_string(1, 0, "Credito").unwrap();
    worksheet.write_string(1, 1, "10/01/2025").unwrap();
    worksheet.write_string(1, 2, "Compra").unwrap();
    worksheet.write_string(1, 3, "VALE3 - VALE SA").unwrap();
    worksheet.write_string(1, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(1, 5, 100.0).unwrap();
    worksheet.write_number(1, 6, 80.0).unwrap();
    worksheet.write_number(1, 7, 8000.0).unwrap();

    // Stock split 1:2 (receives 100 additional shares)
    worksheet.write_string(2, 0, "Credito").unwrap();
    worksheet.write_string(2, 1, "15/02/2025").unwrap();
    worksheet.write_string(2, 2, "Desdobro").unwrap();
    worksheet.write_string(2, 3, "VALE3 - VALE SA").unwrap();
    worksheet.write_string(2, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(2, 5, 100.0).unwrap();
    worksheet.write_string(2, 6, "").unwrap();
    worksheet.write_string(2, 7, "").unwrap();

    // Buy 50 more after split @ R$42.00
    worksheet.write_string(3, 0, "Credito").unwrap();
    worksheet.write_string(3, 1, "01/03/2025").unwrap();
    worksheet.write_string(3, 2, "Compra").unwrap();
    worksheet.write_string(3, 3, "VALE3 - VALE SA").unwrap();
    worksheet.write_string(3, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(3, 5, 50.0).unwrap();
    worksheet.write_number(3, 6, 42.0).unwrap();
    worksheet.write_number(3, 7, 2100.0).unwrap();

    // Sell 150 @ R$45.00
    worksheet.write_string(4, 0, "Debito").unwrap();
    worksheet.write_string(4, 1, "10/04/2025").unwrap();
    worksheet.write_string(4, 2, "Venda").unwrap();
    worksheet.write_string(4, 3, "VALE3 - VALE SA").unwrap();
    worksheet.write_string(4, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(4, 5, 150.0).unwrap();
    worksheet.write_number(4, 6, 45.0).unwrap();
    worksheet.write_number(4, 7, 6750.0).unwrap();

    workbook.save("tests/data/04_stock_split.xlsx").unwrap();
    println!("✓ Created: 04_stock_split.xlsx");
}

#[test]
#[ignore]
fn generate_05_reverse_split() {
    /*
    Test reverse split (grupamento) adjustment.

    Scenario:
    - Buy 1000 MGLU3 @ R$2.00 = R$2,000.00 on 2025-01-10
    - Reverse split 10:1 on 2025-02-20 (10 shares become 1)
      - Adjusted: 100 MGLU3 @ R$20.00 = R$2,000.00
    - Sell 50 MGLU3 @ R$22.00 = R$1,100.00 on 2025-03-15
      - Cost basis: 50 @ R$20.00 = R$1,000.00
      - Profit: R$100.00
    - Remaining: 50 shares
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    // Buy 1000 MGLU3 @ R$2.00
    worksheet.write_string(1, 0, "Credito").unwrap();
    worksheet.write_string(1, 1, "10/01/2025").unwrap();
    worksheet.write_string(1, 2, "Compra").unwrap();
    worksheet
        .write_string(1, 3, "MGLU3 - MAGAZINE LUIZA")
        .unwrap();
    worksheet.write_string(1, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(1, 5, 1000.0).unwrap();
    worksheet.write_number(1, 6, 2.0).unwrap();
    worksheet.write_number(1, 7, 2000.0).unwrap();

    // Reverse split 10:1 (900 shares removed, 1000 -> 100)
    worksheet.write_string(2, 0, "Debito").unwrap();
    worksheet.write_string(2, 1, "20/02/2025").unwrap();
    worksheet.write_string(2, 2, "Incorporação").unwrap();
    worksheet
        .write_string(2, 3, "MGLU3 - MAGAZINE LUIZA")
        .unwrap();
    worksheet.write_string(2, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(2, 5, 900.0).unwrap();
    worksheet.write_string(2, 6, "").unwrap();
    worksheet.write_string(2, 7, "").unwrap();

    // Sell 50 @ R$22.00
    worksheet.write_string(3, 0, "Debito").unwrap();
    worksheet.write_string(3, 1, "15/03/2025").unwrap();
    worksheet.write_string(3, 2, "Venda").unwrap();
    worksheet
        .write_string(3, 3, "MGLU3 - MAGAZINE LUIZA")
        .unwrap();
    worksheet.write_string(3, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(3, 5, 50.0).unwrap();
    worksheet.write_number(3, 6, 22.0).unwrap();
    worksheet.write_number(3, 7, 1100.0).unwrap();

    workbook.save("tests/data/05_reverse_split.xlsx").unwrap();
    println!("✓ Created: 05_reverse_split.xlsx");
}

#[test]
#[ignore]
fn generate_06_multiple_splits() {
    /*
    Test multiple splits on the same asset.

    Scenario:
    - Buy 50 ITSA4 @ R$10.00 = R$500.00 on 2025-01-05
    - Split 1:2 on 2025-02-10 (50 -> 100 shares @ R$5.00)
    - Buy 25 ITSA4 @ R$5.50 = R$137.50 on 2025-03-01
    - Split 1:2 again on 2025-04-15 (125 -> 250 shares)
      - First lot: 100 @ R$2.50 = R$500.00
      - Second lot: 50 @ R$2.75 = R$137.50
    - Sell 200 ITSA4 @ R$3.00 = R$600.00 on 2025-05-20
      - Average cost basis applied after multiple splits
      - Profit: R$75.00
    - Remaining: 50 shares from second lot
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    // Buy 50 @ R$10.00
    worksheet.write_string(1, 0, "Credito").unwrap();
    worksheet.write_string(1, 1, "05/01/2025").unwrap();
    worksheet.write_string(1, 2, "Compra").unwrap();
    worksheet.write_string(1, 3, "ITSA4 - ITAUSA PN").unwrap();
    worksheet.write_string(1, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(1, 5, 50.0).unwrap();
    worksheet.write_number(1, 6, 10.0).unwrap();
    worksheet.write_number(1, 7, 500.0).unwrap();

    // First split 1:2
    worksheet.write_string(2, 0, "Credito").unwrap();
    worksheet.write_string(2, 1, "10/02/2025").unwrap();
    worksheet.write_string(2, 2, "Desdobro").unwrap();
    worksheet.write_string(2, 3, "ITSA4 - ITAUSA PN").unwrap();
    worksheet.write_string(2, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(2, 5, 50.0).unwrap();
    worksheet.write_string(2, 6, "").unwrap();
    worksheet.write_string(2, 7, "").unwrap();

    // Buy 25 @ R$5.50
    worksheet.write_string(3, 0, "Credito").unwrap();
    worksheet.write_string(3, 1, "01/03/2025").unwrap();
    worksheet.write_string(3, 2, "Compra").unwrap();
    worksheet.write_string(3, 3, "ITSA4 - ITAUSA PN").unwrap();
    worksheet.write_string(3, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(3, 5, 25.0).unwrap();
    worksheet.write_number(3, 6, 5.5).unwrap();
    worksheet.write_number(3, 7, 137.5).unwrap();

    // Second split 1:2
    worksheet.write_string(4, 0, "Credito").unwrap();
    worksheet.write_string(4, 1, "15/04/2025").unwrap();
    worksheet.write_string(4, 2, "Desdobro").unwrap();
    worksheet.write_string(4, 3, "ITSA4 - ITAUSA PN").unwrap();
    worksheet.write_string(4, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(4, 5, 125.0).unwrap();
    worksheet.write_string(4, 6, "").unwrap();
    worksheet.write_string(4, 7, "").unwrap();

    // Sell 200 @ R$3.00
    worksheet.write_string(5, 0, "Debito").unwrap();
    worksheet.write_string(5, 1, "20/05/2025").unwrap();
    worksheet.write_string(5, 2, "Venda").unwrap();
    worksheet.write_string(5, 3, "ITSA4 - ITAUSA PN").unwrap();
    worksheet.write_string(5, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(5, 5, 200.0).unwrap();
    worksheet.write_number(5, 6, 3.0).unwrap();
    worksheet.write_number(5, 7, 600.0).unwrap();

    workbook.save("tests/data/06_multiple_splits.xlsx").unwrap();
    println!("✓ Created: 06_multiple_splits.xlsx");
}

#[test]
#[ignore]
fn generate_07_capital_return() {
    /*
    Test FII (Real Estate Fund) with capital return (amortização).

    Note: Capital returns reduce the cost basis of shares.

    Scenario:
    - Buy 100 MXRF11 @ R$10.00 = R$1,000.00 on 2025-01-10
    - Capital return of R$1.00/share on 2025-02-15 = R$100.00
      - Adjusted cost basis: R$900.00 (R$9.00/share)
    - Buy 50 MXRF11 @ R$10.50 = R$525.00 on 2025-03-01
    - Sell 120 MXRF11 @ R$11.00 = R$1,320.00 on 2025-04-20
      - Average cost basis applied after capital return
      - Cost basis: R$1,110.00
      - Profit: R$210.00
    - Remaining: 30 shares @ R$10.50
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    // Buy 100 MXRF11 @ R$10.00
    worksheet.write_string(1, 0, "Credito").unwrap();
    worksheet.write_string(1, 1, "10/01/2025").unwrap();
    worksheet.write_string(1, 2, "Compra").unwrap();
    worksheet
        .write_string(1, 3, "MXRF11 - MAXI RENDA FII")
        .unwrap();
    worksheet.write_string(1, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(1, 5, 100.0).unwrap();
    worksheet.write_number(1, 6, 10.0).unwrap();
    worksheet.write_number(1, 7, 1000.0).unwrap();

    // Capital return (Amortização) R$1.00/share
    worksheet.write_string(2, 0, "Credito").unwrap();
    worksheet.write_string(2, 1, "15/02/2025").unwrap();
    worksheet.write_string(2, 2, "Amortização").unwrap();
    worksheet
        .write_string(2, 3, "MXRF11 - MAXI RENDA FII")
        .unwrap();
    worksheet.write_string(2, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(2, 5, 100.0).unwrap();
    worksheet.write_number(2, 6, 1.0).unwrap();
    worksheet.write_number(2, 7, 100.0).unwrap();

    // Buy 50 more @ R$10.50
    worksheet.write_string(3, 0, "Credito").unwrap();
    worksheet.write_string(3, 1, "01/03/2025").unwrap();
    worksheet.write_string(3, 2, "Compra").unwrap();
    worksheet
        .write_string(3, 3, "MXRF11 - MAXI RENDA FII")
        .unwrap();
    worksheet.write_string(3, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(3, 5, 50.0).unwrap();
    worksheet.write_number(3, 6, 10.5).unwrap();
    worksheet.write_number(3, 7, 525.0).unwrap();

    // Sell 120 @ R$11.00
    worksheet.write_string(4, 0, "Debito").unwrap();
    worksheet.write_string(4, 1, "20/04/2025").unwrap();
    worksheet.write_string(4, 2, "Venda").unwrap();
    worksheet
        .write_string(4, 3, "MXRF11 - MAXI RENDA FII")
        .unwrap();
    worksheet.write_string(4, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(4, 5, 120.0).unwrap();
    worksheet.write_number(4, 6, 11.0).unwrap();
    worksheet.write_number(4, 7, 1320.0).unwrap();

    workbook.save("tests/data/07_capital_return.xlsx").unwrap();
    println!("✓ Created: 07_capital_return.xlsx");
}

#[test]
#[ignore]
fn generate_08_complex_scenario() {
    /*
    Complex scenario combining multiple events.

    Scenario:
    - Buy 200 BBAS3 @ R$40.00 = R$8,000.00 on 2025-01-10
    - Buy 100 BBAS3 @ R$42.00 = R$4,200.00 on 2025-01-25
    - Split 1:2 on 2025-02-15
      - Adjusted: 400 @ R$20.00 + 200 @ R$21.00
    - Sell 300 BBAS3 @ R$22.00 = R$6,600.00 on 2025-03-01
      - Cost: 300 @ R$20.00 = R$6,000.00
      - Profit: R$600.00
    - Buy 150 BBAS3 @ R$23.00 = R$3,450.00 on 2025-03-15
    - Term contract: Buy 200 BBAS3T @ R$24.00 = R$4,800.00 on 2025-04-01
    - Term liquidation: 200 BBAS3T -> BBAS3 on 2025-05-30
    - Sell 400 BBAS3 @ R$26.00 = R$10,400.00 on 2025-06-15
      - Average cost basis applied after multiple events
      - Cost: R$2,000 + R$4,200 + R$2,300 = R$8,500.00
      - Profit: R$1,900.00
    - Remaining: 50 @ R$23.00 + 200 @ R$24.00 = 250 shares
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    let mut row = 1;

    // Initial purchases
    worksheet.write_string(row, 0, "Credito").unwrap();
    worksheet.write_string(row, 1, "10/01/2025").unwrap();
    worksheet.write_string(row, 2, "Compra").unwrap();
    worksheet
        .write_string(row, 3, "BBAS3 - BANCO DO BRASIL")
        .unwrap();
    worksheet.write_string(row, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(row, 5, 200.0).unwrap();
    worksheet.write_number(row, 6, 40.0).unwrap();
    worksheet.write_number(row, 7, 8000.0).unwrap();
    row += 1;

    worksheet.write_string(row, 0, "Credito").unwrap();
    worksheet.write_string(row, 1, "25/01/2025").unwrap();
    worksheet.write_string(row, 2, "Compra").unwrap();
    worksheet
        .write_string(row, 3, "BBAS3 - BANCO DO BRASIL")
        .unwrap();
    worksheet.write_string(row, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(row, 5, 100.0).unwrap();
    worksheet.write_number(row, 6, 42.0).unwrap();
    worksheet.write_number(row, 7, 4200.0).unwrap();
    row += 1;

    // Split 1:2
    worksheet.write_string(row, 0, "Credito").unwrap();
    worksheet.write_string(row, 1, "15/02/2025").unwrap();
    worksheet.write_string(row, 2, "Desdobro").unwrap();
    worksheet
        .write_string(row, 3, "BBAS3 - BANCO DO BRASIL")
        .unwrap();
    worksheet.write_string(row, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(row, 5, 300.0).unwrap();
    worksheet.write_string(row, 6, "").unwrap();
    worksheet.write_string(row, 7, "").unwrap();
    row += 1;

    // First sale
    worksheet.write_string(row, 0, "Debito").unwrap();
    worksheet.write_string(row, 1, "01/03/2025").unwrap();
    worksheet.write_string(row, 2, "Venda").unwrap();
    worksheet
        .write_string(row, 3, "BBAS3 - BANCO DO BRASIL")
        .unwrap();
    worksheet.write_string(row, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(row, 5, 300.0).unwrap();
    worksheet.write_number(row, 6, 22.0).unwrap();
    worksheet.write_number(row, 7, 6600.0).unwrap();
    row += 1;

    // Another purchase
    worksheet.write_string(row, 0, "Credito").unwrap();
    worksheet.write_string(row, 1, "15/03/2025").unwrap();
    worksheet.write_string(row, 2, "Compra").unwrap();
    worksheet
        .write_string(row, 3, "BBAS3 - BANCO DO BRASIL")
        .unwrap();
    worksheet.write_string(row, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(row, 5, 150.0).unwrap();
    worksheet.write_number(row, 6, 23.0).unwrap();
    worksheet.write_number(row, 7, 3450.0).unwrap();
    row += 1;

    // Term contract purchase
    worksheet.write_string(row, 0, "Credito").unwrap();
    worksheet.write_string(row, 1, "01/04/2025").unwrap();
    worksheet.write_string(row, 2, "Compra").unwrap();
    worksheet
        .write_string(row, 3, "Termo de Ação BBAS3 - BBAS3T - BBAS")
        .unwrap();
    worksheet.write_string(row, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(row, 5, 200.0).unwrap();
    worksheet.write_number(row, 6, 24.0).unwrap();
    worksheet.write_number(row, 7, 4800.0).unwrap();
    row += 1;

    // Term liquidation
    worksheet.write_string(row, 0, "Credito").unwrap();
    worksheet.write_string(row, 1, "30/05/2025").unwrap();
    worksheet.write_string(row, 2, "Liquidação Termo").unwrap();
    worksheet
        .write_string(row, 3, "BBAS3 - BANCO DO BRASIL")
        .unwrap();
    worksheet.write_string(row, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(row, 5, 200.0).unwrap();
    worksheet.write_number(row, 6, 24.0).unwrap();
    worksheet.write_number(row, 7, 4800.0).unwrap();
    row += 1;

    // Final sale
    worksheet.write_string(row, 0, "Debito").unwrap();
    worksheet.write_string(row, 1, "15/06/2025").unwrap();
    worksheet.write_string(row, 2, "Venda").unwrap();
    worksheet
        .write_string(row, 3, "BBAS3 - BANCO DO BRASIL")
        .unwrap();
    worksheet.write_string(row, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(row, 5, 400.0).unwrap();
    worksheet.write_number(row, 6, 26.0).unwrap();
    worksheet.write_number(row, 7, 10400.0).unwrap();

    workbook
        .save("tests/data/08_complex_scenario.xlsx")
        .unwrap();
    println!("✓ Created: 08_complex_scenario.xlsx");
}

#[test]
#[ignore]
fn generate_09_fi_infra() {
    /*
    Test FI-Infra (Infrastructure Fund) with similar behavior to FII.

    Scenario:
    - Buy 500 RZTR11 @ R$100.00 = R$50,000.00 on 2025-01-15
    - Sell 200 RZTR11 @ R$105.00 = R$21,000.00 on 2025-03-20
      - Cost basis: 200 @ R$100.00 = R$20,000.00
      - Profit: R$1,000.00
    - Remaining: 300 shares
    */
    let mut workbook = create_workbook_with_header();
    let worksheet = workbook.worksheet_from_index(0).unwrap();

    // Buy FI-Infra
    worksheet.write_string(1, 0, "Credito").unwrap();
    worksheet.write_string(1, 1, "15/01/2025").unwrap();
    worksheet.write_string(1, 2, "Compra").unwrap();
    worksheet
        .write_string(1, 3, "RZTR11 - RIO BRAVO RENDA CORPORATIVA INFRA")
        .unwrap();
    worksheet.write_string(1, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(1, 5, 500.0).unwrap();
    worksheet.write_number(1, 6, 100.0).unwrap();
    worksheet.write_number(1, 7, 50000.0).unwrap();

    // Sell partial position
    worksheet.write_string(2, 0, "Debito").unwrap();
    worksheet.write_string(2, 1, "20/03/2025").unwrap();
    worksheet.write_string(2, 2, "Venda").unwrap();
    worksheet
        .write_string(2, 3, "RZTR11 - RIO BRAVO RENDA CORPORATIVA INFRA")
        .unwrap();
    worksheet.write_string(2, 4, "XP INVESTIMENTOS").unwrap();
    worksheet.write_number(2, 5, 200.0).unwrap();
    worksheet.write_number(2, 6, 105.0).unwrap();
    worksheet.write_number(2, 7, 21000.0).unwrap();

    workbook.save("tests/data/09_fi_infra.xlsx").unwrap();
    println!("✓ Created: 09_fi_infra.xlsx");
}
