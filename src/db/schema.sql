-- Brazilian B3 Investment Tracker Database Schema
-- Compatible with SQLite/Limbo

-- Asset definitions (stocks, FIIs, FIAGROs, FI-INFRAs, bonds)
CREATE TABLE IF NOT EXISTS assets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker TEXT NOT NULL UNIQUE,  -- e.g., 'PETR4', 'MXRF11', 'AGRO3'
    asset_type TEXT NOT NULL,     -- 'STOCK', 'FII', 'FIAGRO', 'FI_INFRA', 'BOND', 'GOV_BOND'
    name TEXT,                     -- Full name of the asset
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Create index on ticker for fast lookups
CREATE INDEX IF NOT EXISTS idx_assets_ticker ON assets(ticker);
CREATE INDEX IF NOT EXISTS idx_assets_type ON assets(asset_type);

-- Transactions (buys and sells)
CREATE TABLE IF NOT EXISTS transactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id INTEGER NOT NULL,
    transaction_type TEXT NOT NULL,    -- 'BUY', 'SELL'
    trade_date DATE NOT NULL,           -- Date of the trade
    settlement_date DATE,               -- Settlement date (D+2 typically)
    quantity DECIMAL(15,4) NOT NULL,    -- Number of shares/quotas
    price_per_unit DECIMAL(15,4) NOT NULL,  -- Price per share/quota
    total_cost DECIMAL(15,4) NOT NULL,  -- Total including fees
    fees DECIMAL(15,4) DEFAULT 0,       -- Brokerage fees, taxes, etc.
    is_day_trade BOOLEAN DEFAULT 0,     -- True if same-day buy/sell
    quota_issuance_date DATE,           -- For funds: when quota was issued (for 2025 vs 2026 tax rules)
    notes TEXT,                         -- Optional notes
    source TEXT,                        -- 'CEI', 'B3_PORTAL', 'MANUAL'
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (asset_id) REFERENCES assets(id)
);

CREATE INDEX IF NOT EXISTS idx_transactions_asset ON transactions(asset_id);
CREATE INDEX IF NOT EXISTS idx_transactions_date ON transactions(trade_date);
CREATE INDEX IF NOT EXISTS idx_transactions_type ON transactions(transaction_type);

-- Corporate actions (splits, reverse splits, bonuses)
CREATE TABLE IF NOT EXISTS corporate_actions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id INTEGER NOT NULL,
    action_type TEXT NOT NULL,      -- 'SPLIT', 'REVERSE_SPLIT', 'BONUS'
    event_date DATE NOT NULL,        -- Announcement date
    ex_date DATE NOT NULL,           -- Date adjustment takes effect
    ratio_from INTEGER NOT NULL,     -- e.g., 1 for 1:2 split
    ratio_to INTEGER NOT NULL,       -- e.g., 2 for 1:2 split
    applied BOOLEAN DEFAULT 0,       -- Whether adjustment has been applied
    source TEXT,                     -- 'BRAPI', 'MANUAL', 'B3'
    notes TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (asset_id) REFERENCES assets(id)
);

CREATE INDEX IF NOT EXISTS idx_corporate_actions_asset ON corporate_actions(asset_id);
CREATE INDEX IF NOT EXISTS idx_corporate_actions_date ON corporate_actions(ex_date);

-- Price history (daily OHLCV data)
CREATE TABLE IF NOT EXISTS price_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id INTEGER NOT NULL,
    price_date DATE NOT NULL,
    close_price DECIMAL(15,4) NOT NULL,
    open_price DECIMAL(15,4),
    high_price DECIMAL(15,4),
    low_price DECIMAL(15,4),
    volume BIGINT,
    source TEXT,                     -- 'YAHOO', 'BRAPI'
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (asset_id) REFERENCES assets(id),
    UNIQUE(asset_id, price_date)
);

CREATE INDEX IF NOT EXISTS idx_price_history_asset ON price_history(asset_id);
CREATE INDEX IF NOT EXISTS idx_price_history_date ON price_history(price_date);

-- Current positions (calculated/cached for performance)
CREATE TABLE IF NOT EXISTS positions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id INTEGER NOT NULL UNIQUE,
    quantity DECIMAL(15,4) NOT NULL,           -- Current quantity held
    average_cost DECIMAL(15,4) NOT NULL,       -- Average cost per unit (FIFO)
    total_cost DECIMAL(15,4) NOT NULL,         -- Total invested
    adjusted_cost DECIMAL(15,4) NOT NULL,      -- After amortization adjustments
    last_updated DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (asset_id) REFERENCES assets(id)
);

CREATE INDEX IF NOT EXISTS idx_positions_asset ON positions(asset_id);

-- Tax events (monthly tracking for swing trade, day trade)
CREATE TABLE IF NOT EXISTS tax_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    year INTEGER NOT NULL,
    month INTEGER NOT NULL,              -- 1-12
    asset_type TEXT NOT NULL,            -- 'STOCK', 'FII', etc.
    event_type TEXT NOT NULL,            -- 'SWING_TRADE', 'DAY_TRADE'
    total_sales DECIMAL(15,4) NOT NULL,  -- Total sales volume
    total_profit DECIMAL(15,4) NOT NULL, -- Total profits
    total_loss DECIMAL(15,4) NOT NULL,   -- Total losses
    net_profit DECIMAL(15,4) NOT NULL,   -- Profit - loss
    tax_rate DECIMAL(5,4) NOT NULL,      -- Tax rate applied (0.15, 0.175, 0.20)
    tax_due DECIMAL(15,4) NOT NULL,      -- Tax amount due
    is_exempt BOOLEAN DEFAULT 0,         -- True if below R$20k threshold
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(year, month, asset_type, event_type)
);

CREATE INDEX IF NOT EXISTS idx_tax_events_date ON tax_events(year, month);
CREATE INDEX IF NOT EXISTS idx_tax_events_type ON tax_events(asset_type, event_type);

-- Income events (dividends, amortization, JCP)
CREATE TABLE IF NOT EXISTS income_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id INTEGER NOT NULL,
    event_date DATE NOT NULL,            -- Payment date
    ex_date DATE,                        -- Ex-dividend date
    event_type TEXT NOT NULL,            -- 'DIVIDEND', 'AMORTIZATION', 'JCP'
    amount_per_quota DECIMAL(15,4) NOT NULL,
    total_amount DECIMAL(15,4) NOT NULL,
    withholding_tax DECIMAL(15,4) DEFAULT 0,  -- Tax withheld at source
    is_quota_pre_2026 BOOLEAN,           -- Track quota vintage for tax rules
    source TEXT,                         -- 'BRAPI', 'CEI', 'MANUAL'
    notes TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (asset_id) REFERENCES assets(id)
);

CREATE INDEX IF NOT EXISTS idx_income_events_asset ON income_events(asset_id);
CREATE INDEX IF NOT EXISTS idx_income_events_date ON income_events(event_date);
CREATE INDEX IF NOT EXISTS idx_income_events_type ON income_events(event_type);

-- Metadata table for schema version and app settings
CREATE TABLE IF NOT EXISTS metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Insert initial schema version
INSERT OR IGNORE INTO metadata (key, value) VALUES ('schema_version', '1');
INSERT OR IGNORE INTO metadata (key, value) VALUES ('db_created_at', datetime('now'));
