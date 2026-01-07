use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Asset types supported by the system
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AssetType {
    Stock,   // Brazilian stocks (ações)
    Etf,     // Exchange-traded funds
    Fii,     // Real estate investment funds
    Fiagro,  // Agribusiness investment funds
    FiInfra, // Infrastructure investment funds
    Bond,    // Corporate bonds
    GovBond, // Government bonds (Tesouro Direto)
}

impl AssetType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AssetType::Stock => "STOCK",
            AssetType::Etf => "ETF",
            AssetType::Fii => "FII",
            AssetType::Fiagro => "FIAGRO",
            AssetType::FiInfra => "FI_INFRA",
            AssetType::Bond => "BOND",
            AssetType::GovBond => "GOV_BOND",
        }
    }

    /// Detect asset type from ticker pattern
    /// Stocks end in 3-6, FIIs end in 11, FIAGROs/FI-INFRAs typically have specific patterns
    pub fn detect_from_ticker(ticker: &str) -> Option<Self> {
        if ticker.len() < 5 {
            return None;
        }

        let upper = ticker.to_uppercase();
        if matches!(
            upper.as_str(),
            "CRMG15" | "ELET23" | "LIGHD7" | "LAMEA6" | "UNEG11"
        ) {
            return Some(AssetType::Bond);
        }
        if matches!(
            upper.as_str(),
            "CDII11" | "JURO11" | "BODB11" | "BDIF11" | "IFRA11" | "XPID11" | "KDIF11"
        ) {
            return Some(AssetType::FiInfra);
        }
        if matches!(upper.as_str(), "CRAA11" | "FGAA11") {
            return Some(AssetType::Fiagro);
        }
        if matches!(upper.as_str(), "DIVD11" | "NDIV11" | "UTLL11" | "TIRB11") {
            return Some(AssetType::Etf);
        }

        // Extract the numeric suffix
        let suffix = &ticker[ticker.len() - 2..];

        match suffix {
            "11" => Some(AssetType::Fii),                  // Most FIIs end in 11
            "32" | "33" | "34" => Some(AssetType::Fiagro), // Common FIAGRO patterns
            _ if ticker.ends_with('3')
                || ticker.ends_with('4')
                || ticker.ends_with('5')
                || ticker.ends_with('6') =>
            {
                Some(AssetType::Stock)
            }
            _ => None, // Unknown pattern, will need manual classification
        }
    }
}

impl FromStr for AssetType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_uppercase().as_str() {
            "STOCK" => Ok(AssetType::Stock),
            "ETF" => Ok(AssetType::Etf),
            "FII" => Ok(AssetType::Fii),
            "FIAGRO" => Ok(AssetType::Fiagro),
            "FI_INFRA" => Ok(AssetType::FiInfra),
            "BOND" => Ok(AssetType::Bond),
            "GOV_BOND" => Ok(AssetType::GovBond),
            _ => Err(()),
        }
    }
}

/// Asset (stock, fund, bond)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: Option<i64>,
    pub ticker: String,
    pub asset_type: AssetType,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Transaction type (buy or sell)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransactionType {
    Buy,
    Sell,
}

impl TransactionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransactionType::Buy => "BUY",
            TransactionType::Sell => "SELL",
        }
    }
}

impl FromStr for TransactionType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_uppercase().as_str() {
            "BUY" | "COMPRA" | "C" => Ok(TransactionType::Buy),
            "SELL" | "VENDA" | "V" => Ok(TransactionType::Sell),
            _ => Err(()),
        }
    }
}

/// Transaction (buy or sell of an asset)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub id: Option<i64>,
    pub asset_id: i64,
    pub transaction_type: TransactionType,
    pub trade_date: NaiveDate,
    pub settlement_date: Option<NaiveDate>,
    pub quantity: Decimal,
    pub price_per_unit: Decimal,
    pub total_cost: Decimal,
    pub fees: Decimal,
    pub is_day_trade: bool,
    pub quota_issuance_date: Option<NaiveDate>, // For fund tax rules
    pub notes: Option<String>,
    pub source: String, // 'CEI', 'B3_PORTAL', 'MANUAL'
    pub created_at: DateTime<Utc>,
}

/// Corporate action type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CorporateActionType {
    Split,         // Stock split (desdobramento)
    ReverseSplit,  // Reverse split (grupamento)
    Bonus,         // Bonus shares (bonificação)
    CapitalReturn, // Capital return / Amortization (amortização)
}
impl CorporateActionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            CorporateActionType::Split => "SPLIT",
            CorporateActionType::ReverseSplit => "REVERSE_SPLIT",
            CorporateActionType::Bonus => "BONUS",
            CorporateActionType::CapitalReturn => "CAPITAL_RETURN",
        }
    }
}

impl FromStr for CorporateActionType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_uppercase().as_str() {
            "SPLIT" | "DESDOBRAMENTO" => Ok(CorporateActionType::Split),
            "REVERSE_SPLIT" | "GRUPAMENTO" => Ok(CorporateActionType::ReverseSplit),
            "BONUS" | "BONIFICAÇÃO" | "BONIFICACAO" => Ok(CorporateActionType::Bonus),
            "CAPITAL_RETURN" | "AMORTIZAÇÃO" | "AMORTIZACAO" => {
                Ok(CorporateActionType::CapitalReturn)
            }
            _ => Err(()),
        }
    }
}

/// Corporate action (split, reverse split, bonus shares)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorporateAction {
    pub id: Option<i64>,
    pub asset_id: i64,
    pub action_type: CorporateActionType,
    pub event_date: NaiveDate,
    pub ex_date: NaiveDate,
    pub ratio_from: i32, // e.g., 1 for 1:2 split
    pub ratio_to: i32,   // e.g., 2 for 1:2 split
    pub applied: bool,
    pub source: String,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Price history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceHistory {
    pub id: Option<i64>,
    pub asset_id: i64,
    pub price_date: NaiveDate,
    pub close_price: Decimal,
    pub open_price: Option<Decimal>,
    pub high_price: Option<Decimal>,
    pub low_price: Option<Decimal>,
    pub volume: Option<i64>,
    pub source: String,
    pub created_at: DateTime<Utc>,
}

/// Current position (holdings)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Position {
    pub id: Option<i64>,
    pub asset_id: i64,
    pub quantity: Decimal,
    pub average_cost: Decimal,
    pub total_cost: Decimal,
    pub adjusted_cost: Decimal, // After amortization
    pub last_updated: DateTime<Utc>,
}

/// Income event type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IncomeEventType {
    Dividend,     // Regular dividend (rendimento)
    Amortization, // Capital return (amortização)
    Jcp,          // Juros sobre Capital Próprio
}

impl IncomeEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            IncomeEventType::Dividend => "DIVIDEND",
            IncomeEventType::Amortization => "AMORTIZATION",
            IncomeEventType::Jcp => "JCP",
        }
    }
}

impl FromStr for IncomeEventType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_uppercase().as_str() {
            "DIVIDEND" | "DIVIDENDO" | "RENDIMENTO" => Ok(IncomeEventType::Dividend),
            "AMORTIZATION" | "AMORTIZAÇÃO" | "AMORTIZACAO" => Ok(IncomeEventType::Amortization),
            "JCP" => Ok(IncomeEventType::Jcp),
            _ => Err(()),
        }
    }
}

/// Income event (dividend, amortization, JCP)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomeEvent {
    pub id: Option<i64>,
    pub asset_id: i64,
    pub event_date: NaiveDate,
    pub ex_date: Option<NaiveDate>,
    pub event_type: IncomeEventType,
    pub amount_per_quota: Decimal,
    pub total_amount: Decimal,
    pub withholding_tax: Decimal,
    pub is_quota_pre_2026: Option<bool>, // For tax rule tracking
    pub source: String,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Tax event (monthly summary)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TaxEvent {
    pub id: Option<i64>,
    pub year: i32,
    pub month: i32,
    pub asset_type: AssetType,
    pub event_type: String, // 'SWING_TRADE', 'DAY_TRADE'
    pub total_sales: Decimal,
    pub total_profit: Decimal,
    pub total_loss: Decimal,
    pub net_profit: Decimal,
    pub tax_rate: Decimal,
    pub tax_due: Decimal,
    pub is_exempt: bool,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asset_type_conversions() {
        // Test to_str and from_str roundtrip
        assert_eq!(AssetType::Stock.as_str(), "STOCK");
        assert_eq!(AssetType::Etf.as_str(), "ETF");
        assert_eq!(AssetType::Fii.as_str(), "FII");
        assert_eq!(AssetType::Fiagro.as_str(), "FIAGRO");
        assert_eq!(AssetType::FiInfra.as_str(), "FI_INFRA");
        assert_eq!(AssetType::Bond.as_str(), "BOND");
        assert_eq!(AssetType::GovBond.as_str(), "GOV_BOND");

        assert_eq!("STOCK".parse::<AssetType>().ok(), Some(AssetType::Stock));
        assert_eq!("ETF".parse::<AssetType>().ok(), Some(AssetType::Etf));
        assert_eq!("stock".parse::<AssetType>().ok(), Some(AssetType::Stock));
        assert_eq!("FII".parse::<AssetType>().ok(), Some(AssetType::Fii));
        assert_eq!("FIAGRO".parse::<AssetType>().ok(), Some(AssetType::Fiagro));
        assert_eq!(
            "FI_INFRA".parse::<AssetType>().ok(),
            Some(AssetType::FiInfra)
        );
        assert_eq!("BOND".parse::<AssetType>().ok(), Some(AssetType::Bond));
        assert_eq!(
            "GOV_BOND".parse::<AssetType>().ok(),
            Some(AssetType::GovBond)
        );
        assert_eq!("INVALID".parse::<AssetType>().ok(), None);
    }

    #[test]
    fn test_asset_type_detect_from_ticker() {
        // Stock patterns
        assert_eq!(
            AssetType::detect_from_ticker("PETR4"),
            Some(AssetType::Stock)
        );
        assert_eq!(
            AssetType::detect_from_ticker("VALE3"),
            Some(AssetType::Stock)
        );
        assert_eq!(
            AssetType::detect_from_ticker("ITSA4"),
            Some(AssetType::Stock)
        );
        assert_eq!(
            AssetType::detect_from_ticker("BBDC3"),
            Some(AssetType::Stock)
        );
        assert_eq!(
            AssetType::detect_from_ticker("MGLU3"),
            Some(AssetType::Stock)
        );

        // FII patterns (ending in 11)
        assert_eq!(
            AssetType::detect_from_ticker("MXRF11"),
            Some(AssetType::Fii)
        );
        assert_eq!(
            AssetType::detect_from_ticker("HGLG11"),
            Some(AssetType::Fii)
        );

        // FIAGRO patterns
        assert_eq!(
            AssetType::detect_from_ticker("TEST32"),
            Some(AssetType::Fiagro)
        );
        assert_eq!(
            AssetType::detect_from_ticker("TEST33"),
            Some(AssetType::Fiagro)
        );
        assert_eq!(
            AssetType::detect_from_ticker("TEST34"),
            Some(AssetType::Fiagro)
        );

        // Bond overrides
        assert_eq!(
            AssetType::detect_from_ticker("CRMG15"),
            Some(AssetType::Bond)
        );
        assert_eq!(
            AssetType::detect_from_ticker("ELET23"),
            Some(AssetType::Bond)
        );
        assert_eq!(
            AssetType::detect_from_ticker("LIGHD7"),
            Some(AssetType::Bond)
        );
        assert_eq!(
            AssetType::detect_from_ticker("LAMEA6"),
            Some(AssetType::Bond)
        );
        assert_eq!(
            AssetType::detect_from_ticker("UNEG11"),
            Some(AssetType::Bond)
        );
        assert_eq!(
            AssetType::detect_from_ticker("CDII11"),
            Some(AssetType::FiInfra)
        );
        assert_eq!(
            AssetType::detect_from_ticker("JURO11"),
            Some(AssetType::FiInfra)
        );
        assert_eq!(
            AssetType::detect_from_ticker("BODB11"),
            Some(AssetType::FiInfra)
        );
        assert_eq!(
            AssetType::detect_from_ticker("BDIF11"),
            Some(AssetType::FiInfra)
        );
        assert_eq!(
            AssetType::detect_from_ticker("IFRA11"),
            Some(AssetType::FiInfra)
        );
        assert_eq!(
            AssetType::detect_from_ticker("XPID11"),
            Some(AssetType::FiInfra)
        );
        assert_eq!(
            AssetType::detect_from_ticker("KDIF11"),
            Some(AssetType::FiInfra)
        );
        assert_eq!(
            AssetType::detect_from_ticker("CRAA11"),
            Some(AssetType::Fiagro)
        );
        assert_eq!(
            AssetType::detect_from_ticker("FGAA11"),
            Some(AssetType::Fiagro)
        );
        assert_eq!(
            AssetType::detect_from_ticker("DIVD11"),
            Some(AssetType::Etf)
        );
        assert_eq!(
            AssetType::detect_from_ticker("NDIV11"),
            Some(AssetType::Etf)
        );
        assert_eq!(
            AssetType::detect_from_ticker("UTLL11"),
            Some(AssetType::Etf)
        );
        assert_eq!(
            AssetType::detect_from_ticker("TIRB11"),
            Some(AssetType::Etf)
        );

        // Unknown patterns
        assert_eq!(AssetType::detect_from_ticker("SHORT"), None);
        assert_eq!(AssetType::detect_from_ticker("TEST99"), None);
    }

    #[test]
    fn test_transaction_type_conversions() {
        assert_eq!(TransactionType::Buy.as_str(), "BUY");
        assert_eq!(TransactionType::Sell.as_str(), "SELL");

        // Test various input formats
        assert_eq!(
            "BUY".parse::<TransactionType>().ok(),
            Some(TransactionType::Buy)
        );
        assert_eq!(
            "buy".parse::<TransactionType>().ok(),
            Some(TransactionType::Buy)
        );
        assert_eq!(
            "COMPRA".parse::<TransactionType>().ok(),
            Some(TransactionType::Buy)
        );
        assert_eq!(
            "C".parse::<TransactionType>().ok(),
            Some(TransactionType::Buy)
        );

        assert_eq!(
            "SELL".parse::<TransactionType>().ok(),
            Some(TransactionType::Sell)
        );
        assert_eq!(
            "sell".parse::<TransactionType>().ok(),
            Some(TransactionType::Sell)
        );
        assert_eq!(
            "VENDA".parse::<TransactionType>().ok(),
            Some(TransactionType::Sell)
        );
        assert_eq!(
            "V".parse::<TransactionType>().ok(),
            Some(TransactionType::Sell)
        );

        assert_eq!("INVALID".parse::<TransactionType>().ok(), None);
    }

    #[test]
    fn test_corporate_action_type_conversions() {
        assert_eq!(CorporateActionType::Split.as_str(), "SPLIT");
        assert_eq!(CorporateActionType::ReverseSplit.as_str(), "REVERSE_SPLIT");
        assert_eq!(CorporateActionType::Bonus.as_str(), "BONUS");
        assert_eq!(
            CorporateActionType::CapitalReturn.as_str(),
            "CAPITAL_RETURN"
        );

        // Test English inputs
        assert_eq!(
            "SPLIT".parse::<CorporateActionType>().ok(),
            Some(CorporateActionType::Split)
        );
        assert_eq!(
            "split".parse::<CorporateActionType>().ok(),
            Some(CorporateActionType::Split)
        );
        assert_eq!(
            "REVERSE_SPLIT".parse::<CorporateActionType>().ok(),
            Some(CorporateActionType::ReverseSplit)
        );
        assert_eq!(
            "BONUS".parse::<CorporateActionType>().ok(),
            Some(CorporateActionType::Bonus)
        );
        assert_eq!(
            "CAPITAL_RETURN".parse::<CorporateActionType>().ok(),
            Some(CorporateActionType::CapitalReturn)
        );

        // Test Portuguese inputs
        assert_eq!(
            "DESDOBRAMENTO".parse::<CorporateActionType>().ok(),
            Some(CorporateActionType::Split)
        );
        assert_eq!(
            "GRUPAMENTO".parse::<CorporateActionType>().ok(),
            Some(CorporateActionType::ReverseSplit)
        );
        assert_eq!(
            "BONIFICAÇÃO".parse::<CorporateActionType>().ok(),
            Some(CorporateActionType::Bonus)
        );
        assert_eq!(
            "BONIFICACAO".parse::<CorporateActionType>().ok(),
            Some(CorporateActionType::Bonus)
        );
        assert_eq!(
            "AMORTIZAÇÃO".parse::<CorporateActionType>().ok(),
            Some(CorporateActionType::CapitalReturn)
        );
        assert_eq!(
            "AMORTIZACAO".parse::<CorporateActionType>().ok(),
            Some(CorporateActionType::CapitalReturn)
        );

        assert_eq!("INVALID".parse::<CorporateActionType>().ok(), None);
    }

    #[test]
    fn test_income_event_type_conversions() {
        assert_eq!(IncomeEventType::Dividend.as_str(), "DIVIDEND");
        assert_eq!(IncomeEventType::Amortization.as_str(), "AMORTIZATION");
        assert_eq!(IncomeEventType::Jcp.as_str(), "JCP");

        // Test English inputs
        assert_eq!(
            "DIVIDEND".parse::<IncomeEventType>().ok(),
            Some(IncomeEventType::Dividend)
        );
        assert_eq!(
            "dividend".parse::<IncomeEventType>().ok(),
            Some(IncomeEventType::Dividend)
        );
        assert_eq!(
            "AMORTIZATION".parse::<IncomeEventType>().ok(),
            Some(IncomeEventType::Amortization)
        );
        assert_eq!(
            "JCP".parse::<IncomeEventType>().ok(),
            Some(IncomeEventType::Jcp)
        );

        // Test Portuguese inputs
        assert_eq!(
            "DIVIDENDO".parse::<IncomeEventType>().ok(),
            Some(IncomeEventType::Dividend)
        );
        assert_eq!(
            "RENDIMENTO".parse::<IncomeEventType>().ok(),
            Some(IncomeEventType::Dividend)
        );
        assert_eq!(
            "AMORTIZAÇÃO".parse::<IncomeEventType>().ok(),
            Some(IncomeEventType::Amortization)
        );
        assert_eq!(
            "AMORTIZACAO".parse::<IncomeEventType>().ok(),
            Some(IncomeEventType::Amortization)
        );

        assert_eq!("INVALID".parse::<IncomeEventType>().ok(), None);
    }
}
