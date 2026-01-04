#!/usr/bin/env python3
"""
Generate test XLS files for the interest tracker test suite.

These files test various scenarios:
1. Basic purchases and sales
2. Term contracts (purchase with T suffix, liquidation)
3. Splits and reverse splits
4. Capital returns
5. Complex scenarios with multiple events
"""

from openpyxl import Workbook
from datetime import datetime


def create_workbook_with_header():
    """Create a new workbook with the standard Movimentação header."""
    wb = Workbook()
    ws = wb.active
    ws.title = "Movimentação"

    # Header row
    headers = [
        "Entrada/Saída",
        "Data",
        "Movimentação",
        "Produto",
        "Instituição",
        "Quantidade",
        "Preço unitário",
        "Valor da Operação"
    ]
    ws.append(headers)

    return wb, ws


def test_basic_purchase_and_sale():
    """
    Test basic purchase and sale with cost basis calculation.

    Scenario:
    - Buy 100 PETR4 @ R$25.00 = R$2,500.00 on 2025-01-10
    - Buy 50 PETR4 @ R$30.00 = R$1,500.00 on 2025-02-15
    - Sell 80 PETR4 @ R$35.00 = R$2,800.00 on 2025-03-20
      - Average cost basis applied to the sale
      - Profit: R$2,800.00 - R$2,000.00 = R$800.00
    - Remaining: 20 from first lot + 50 from second lot = 70 shares
    """
    wb, ws = create_workbook_with_header()

    # Buy 100 PETR4 @ R$25.00
    ws.append([
        "Credito",
        "10/01/2025",
        "Compra",
        "PETR4 - PETROBRAS PN",
        "XP INVESTIMENTOS",
        100,
        25.00,
        2500.00
    ])

    # Buy 50 PETR4 @ R$30.00
    ws.append([
        "Credito",
        "15/02/2025",
        "Compra",
        "PETR4 - PETROBRAS PN",
        "XP INVESTIMENTOS",
        50,
        30.00,
        1500.00
    ])

    # Sell 80 PETR4 @ R$35.00
    ws.append([
        "Debito",
        "20/03/2025",
        "Venda",
        "PETR4 - PETROBRAS PN",
        "XP INVESTIMENTOS",
        80,
        35.00,
        2800.00
    ])

    wb.save('tests/data/01_basic_purchase_sale.xlsx')
    print("✓ Created: 01_basic_purchase_sale.xlsx")


def test_term_contract_lifecycle():
    """
    Test term contract purchase, expiry, and sale.

    Scenario:
    - Buy 200 ANIM3T (term) @ R$10.00 = R$2,000.00 on 2025-01-15
    - Term liquidation: 200 ANIM3T -> ANIM3 on 2025-02-28
      - Cost basis transfers from ANIM3T to ANIM3 @ R$10.00
    - Sell 100 ANIM3 @ R$12.00 = R$1,200.00 on 2025-03-15
      - Cost basis: 100 @ R$10.00 = R$1,000.00
      - Profit: R$200.00
    - Remaining: 100 ANIM3 @ R$10.00
    """
    wb, ws = create_workbook_with_header()

    # Buy term contract ANIM3T @ R$10.00
    ws.append([
        "Credito",
        "15/01/2025",
        "Compra",
        "Termo de Ação ANIM3 - ANIM3T - ANIM",
        "XP INVESTIMENTOS",
        200,
        10.00,
        2000.00
    ])

    # Term liquidation - receives ANIM3 shares
    ws.append([
        "Credito",
        "28/02/2025",
        "Liquidação Termo",
        "ANIM3 - ANIMA HOLDING",
        "XP INVESTIMENTOS",
        200,
        10.00,
        2000.00
    ])

    # Sell ANIM3 @ R$12.00
    ws.append([
        "Debito",
        "15/03/2025",
        "Venda",
        "ANIM3 - ANIMA HOLDING",
        "XP INVESTIMENTOS",
        100,
        12.00,
        1200.00
    ])

    wb.save('tests/data/02_term_contract_lifecycle.xlsx')
    print("✓ Created: 02_term_contract_lifecycle.xlsx")


def test_term_contract_sold_before_expiry():
    """
    Test selling a term contract before it expires.

    Scenario:
    - Buy 150 SHUL4T (term) @ R$8.00 = R$1,200.00 on 2025-01-10
    - Sell 150 SHUL4T (term) @ R$9.00 = R$1,350.00 on 2025-02-05
      - Profit: R$150.00
    - No liquidation (sold before expiry)
    """
    wb, ws = create_workbook_with_header()

    # Buy term contract
    ws.append([
        "Credito",
        "10/01/2025",
        "Compra",
        "Termo de Ação SHUL4 - SHUL4T - SHUL",
        "XP INVESTIMENTOS",
        150,
        8.00,
        1200.00
    ])

    # Sell term contract before expiry
    ws.append([
        "Debito",
        "05/02/2025",
        "Venda",
        "Termo de Ação SHUL4 - SHUL4T - SHUL",
        "XP INVESTIMENTOS",
        150,
        9.00,
        1350.00
    ])

    wb.save('tests/data/03_term_contract_sold.xlsx')
    print("✓ Created: 03_term_contract_sold.xlsx")


def test_stock_split():
    """
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
    """
    wb, ws = create_workbook_with_header()

    # Buy 100 VALE3 @ R$80.00
    ws.append([
        "Credito",
        "10/01/2025",
        "Compra",
        "VALE3 - VALE SA",
        "XP INVESTIMENTOS",
        100,
        80.00,
        8000.00
    ])

    # Stock split 1:2
    # Note: The split entry shows the NEW shares received
    ws.append([
        "Credito",
        "15/02/2025",
        "Desdobro",
        "VALE3 - VALE SA",
        "XP INVESTIMENTOS",
        100,  # Receives 100 additional shares
        "",
        ""
    ])

    # Buy 50 more after split @ R$42.00
    ws.append([
        "Credito",
        "01/03/2025",
        "Compra",
        "VALE3 - VALE SA",
        "XP INVESTIMENTOS",
        50,
        42.00,
        2100.00
    ])

    # Sell 150 @ R$45.00
    ws.append([
        "Debito",
        "10/04/2025",
        "Venda",
        "VALE3 - VALE SA",
        "XP INVESTIMENTOS",
        150,
        45.00,
        6750.00
    ])

    wb.save('tests/data/04_stock_split.xlsx')
    print("✓ Created: 04_stock_split.xlsx")


def test_reverse_split():
    """
    Test reverse split (grupamento) adjustment.

    Scenario:
    - Buy 1000 MGLU3 @ R$2.00 = R$2,000.00 on 2025-01-10
    - Reverse split 10:1 on 2025-02-20 (10 shares become 1)
      - Adjusted: 100 MGLU3 @ R$20.00 = R$2,000.00
    - Sell 50 MGLU3 @ R$22.00 = R$1,100.00 on 2025-03-15
      - Cost basis: 50 @ R$20.00 = R$1,000.00
      - Profit: R$100.00
    - Remaining: 50 shares
    """
    wb, ws = create_workbook_with_header()

    # Buy 1000 MGLU3 @ R$2.00
    ws.append([
        "Credito",
        "10/01/2025",
        "Compra",
        "MGLU3 - MAGAZINE LUIZA",
        "XP INVESTIMENTOS",
        1000,
        2.00,
        2000.00
    ])

    # Reverse split 10:1
    # Note: This is recorded as Incorporação in B3 files
    ws.append([
        "Debito",
        "20/02/2025",
        "Incorporação",
        "MGLU3 - MAGAZINE LUIZA",
        "XP INVESTIMENTOS",
        900,  # 900 shares removed (1000 -> 100)
        "",
        ""
    ])

    # Sell 50 @ R$22.00
    ws.append([
        "Debito",
        "15/03/2025",
        "Venda",
        "MGLU3 - MAGAZINE LUIZA",
        "XP INVESTIMENTOS",
        50,
        22.00,
        1100.00
    ])

    wb.save('tests/data/05_reverse_split.xlsx')
    print("✓ Created: 05_reverse_split.xlsx")


def test_multiple_splits():
    """
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
    """
    wb, ws = create_workbook_with_header()

    # Buy 50 @ R$10.00
    ws.append([
        "Credito",
        "05/01/2025",
        "Compra",
        "ITSA4 - ITAUSA PN",
        "XP INVESTIMENTOS",
        50,
        10.00,
        500.00
    ])

    # First split 1:2
    ws.append([
        "Credito",
        "10/02/2025",
        "Desdobro",
        "ITSA4 - ITAUSA PN",
        "XP INVESTIMENTOS",
        50,
        "",
        ""
    ])

    # Buy 25 @ R$5.50
    ws.append([
        "Credito",
        "01/03/2025",
        "Compra",
        "ITSA4 - ITAUSA PN",
        "XP INVESTIMENTOS",
        25,
        5.50,
        137.50
    ])

    # Second split 1:2
    ws.append([
        "Credito",
        "15/04/2025",
        "Desdobro",
        "ITSA4 - ITAUSA PN",
        "XP INVESTIMENTOS",
        125,
        "",
        ""
    ])

    # Sell 200 @ R$3.00
    ws.append([
        "Debito",
        "20/05/2025",
        "Venda",
        "ITSA4 - ITAUSA PN",
        "XP INVESTIMENTOS",
        200,
        3.00,
        600.00
    ])

    wb.save('tests/data/06_multiple_splits.xlsx')
    print("✓ Created: 06_multiple_splits.xlsx")


def test_fii_with_capital_return():
    """
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
    """
    wb, ws = create_workbook_with_header()

    # Buy 100 MXRF11 @ R$10.00
    ws.append([
        "Credito",
        "10/01/2025",
        "Compra",
        "MXRF11 - MAXI RENDA FII",
        "XP INVESTIMENTOS",
        100,
        10.00,
        1000.00
    ])

    # Capital return (Amortização) R$1.00/share
    ws.append([
        "Credito",
        "15/02/2025",
        "Amortização",
        "MXRF11 - MAXI RENDA FII",
        "XP INVESTIMENTOS",
        100,
        1.00,
        100.00
    ])

    # Buy 50 more @ R$10.50
    ws.append([
        "Credito",
        "01/03/2025",
        "Compra",
        "MXRF11 - MAXI RENDA FII",
        "XP INVESTIMENTOS",
        50,
        10.50,
        525.00
    ])

    # Sell 120 @ R$11.00
    ws.append([
        "Debito",
        "20/04/2025",
        "Venda",
        "MXRF11 - MAXI RENDA FII",
        "XP INVESTIMENTOS",
        120,
        11.00,
        1320.00
    ])

    wb.save('tests/data/07_capital_return.xlsx')
    print("✓ Created: 07_capital_return.xlsx")


def test_complex_scenario():
    """
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
    """
    wb, ws = create_workbook_with_header()

    # Initial purchases
    ws.append([
        "Credito",
        "10/01/2025",
        "Compra",
        "BBAS3 - BANCO DO BRASIL",
        "XP INVESTIMENTOS",
        200,
        40.00,
        8000.00
    ])

    ws.append([
        "Credito",
        "25/01/2025",
        "Compra",
        "BBAS3 - BANCO DO BRASIL",
        "XP INVESTIMENTOS",
        100,
        42.00,
        4200.00
    ])

    # Split 1:2
    ws.append([
        "Credito",
        "15/02/2025",
        "Desdobro",
        "BBAS3 - BANCO DO BRASIL",
        "XP INVESTIMENTOS",
        300,
        "",
        ""
    ])

    # First sale
    ws.append([
        "Debito",
        "01/03/2025",
        "Venda",
        "BBAS3 - BANCO DO BRASIL",
        "XP INVESTIMENTOS",
        300,
        22.00,
        6600.00
    ])

    # Another purchase
    ws.append([
        "Credito",
        "15/03/2025",
        "Compra",
        "BBAS3 - BANCO DO BRASIL",
        "XP INVESTIMENTOS",
        150,
        23.00,
        3450.00
    ])

    # Term contract purchase
    ws.append([
        "Credito",
        "01/04/2025",
        "Compra",
        "Termo de Ação BBAS3 - BBAS3T - BBAS",
        "XP INVESTIMENTOS",
        200,
        24.00,
        4800.00
    ])

    # Term liquidation
    ws.append([
        "Credito",
        "30/05/2025",
        "Liquidação Termo",
        "BBAS3 - BANCO DO BRASIL",
        "XP INVESTIMENTOS",
        200,
        24.00,
        4800.00
    ])

    # Final sale
    ws.append([
        "Debito",
        "15/06/2025",
        "Venda",
        "BBAS3 - BANCO DO BRASIL",
        "XP INVESTIMENTOS",
        400,
        26.00,
        10400.00
    ])

    wb.save('tests/data/08_complex_scenario.xlsx')
    print("✓ Created: 08_complex_scenario.xlsx")


def test_fi_infra():
    """
    Test FI-Infra (Infrastructure Fund) with similar behavior to FII.

    Scenario:
    - Buy 500 RZTR11 @ R$100.00 = R$50,000.00 on 2025-01-15
    - Sell 200 RZTR11 @ R$105.00 = R$21,000.00 on 2025-03-20
      - Cost basis: 200 @ R$100.00 = R$20,000.00
      - Profit: R$1,000.00
    - Remaining: 300 shares
    """
    wb, ws = create_workbook_with_header()

    # Buy FI-Infra
    ws.append([
        "Credito",
        "15/01/2025",
        "Compra",
        "RZTR11 - RIO BRAVO RENDA CORPORATIVA INFRA",
        "XP INVESTIMENTOS",
        500,
        100.00,
        50000.00
    ])

    # Sell partial position
    ws.append([
        "Debito",
        "20/03/2025",
        "Venda",
        "RZTR11 - RIO BRAVO RENDA CORPORATIVA INFRA",
        "XP INVESTIMENTOS",
        200,
        105.00,
        21000.00
    ])

    wb.save('tests/data/09_fi_infra.xlsx')
    print("✓ Created: 09_fi_infra.xlsx")


if __name__ == "__main__":
    print("Generating test data files...")
    print()

    test_basic_purchase_and_sale()
    test_term_contract_lifecycle()
    test_term_contract_sold_before_expiry()
    test_stock_split()
    test_reverse_split()
    test_multiple_splits()
    test_fii_with_capital_return()
    test_complex_scenario()
    test_fi_infra()

    print()
    print("✅ All test files generated successfully!")
    print()
    print("Test files created:")
    print("  01_basic_purchase_sale.xlsx - Basic buy/sell with average cost")
    print("  02_term_contract_lifecycle.xlsx - Term contract purchase, expiry, sale")
    print("  03_term_contract_sold.xlsx - Term contract sold before expiry")
    print("  04_stock_split.xlsx - Stock split adjustment")
    print("  05_reverse_split.xlsx - Reverse split adjustment")
    print("  06_multiple_splits.xlsx - Multiple splits on same asset")
    print("  07_capital_return.xlsx - FII with capital return")
    print("  08_complex_scenario.xlsx - Complex multi-event scenario")
    print("  09_fi_infra.xlsx - FI-Infra fund")
