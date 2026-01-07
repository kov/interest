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

-- Corporate action adjustments (tracks which transactions have been adjusted by which actions)
CREATE TABLE IF NOT EXISTS corporate_action_adjustments (
    action_id INTEGER NOT NULL,
    transaction_id INTEGER NOT NULL,
    adjusted_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    old_quantity DECIMAL(15,4) NOT NULL,      -- Quantity before adjustment
    new_quantity DECIMAL(15,4) NOT NULL,      -- Quantity after adjustment
    old_price DECIMAL(15,4) NOT NULL,         -- Price before adjustment
    new_price DECIMAL(15,4) NOT NULL,         -- Price after adjustment
    PRIMARY KEY (action_id, transaction_id),
    FOREIGN KEY (action_id) REFERENCES corporate_actions(id),
    FOREIGN KEY (transaction_id) REFERENCES transactions(id)
);

CREATE INDEX IF NOT EXISTS idx_action_adjustments_action ON corporate_action_adjustments(action_id);
CREATE INDEX IF NOT EXISTS idx_action_adjustments_transaction ON corporate_action_adjustments(transaction_id);

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
    average_cost DECIMAL(15,4) NOT NULL,       -- Average cost per unit
    total_cost DECIMAL(15,4) NOT NULL,         -- Total invested
    adjusted_cost DECIMAL(15,4) NOT NULL,      -- After amortization adjustments
    last_updated DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (asset_id) REFERENCES assets(id)
);

CREATE INDEX IF NOT EXISTS idx_positions_asset ON positions(asset_id);

-- Import state (tracks last imported date per source/type)
CREATE TABLE IF NOT EXISTS import_state (
    source TEXT NOT NULL,
    entry_type TEXT NOT NULL,
    last_date DATE NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (source, entry_type)
);

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

-- Loss carryforward tracking (prejuízos a compensar)
CREATE TABLE IF NOT EXISTS loss_carryforward (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    year INTEGER NOT NULL,
    month INTEGER NOT NULL,              -- Month the loss occurred
    tax_category TEXT NOT NULL,          -- 'STOCK_SWING', 'STOCK_DAY', 'FII_SWING', 'FII_DAY', 'FIAGRO_SWING', 'FIAGRO_DAY'
    loss_amount DECIMAL(15,4) NOT NULL,  -- Amount of loss to carry forward
    remaining_amount DECIMAL(15,4) NOT NULL,  -- Amount not yet offset
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_loss_carryforward_category ON loss_carryforward(tax_category);
CREATE INDEX IF NOT EXISTS idx_loss_carryforward_date ON loss_carryforward(year, month);

-- Loss carryforward snapshots (idempotent, per-year per-category)
CREATE TABLE IF NOT EXISTS loss_carryforward_snapshots (
    year INTEGER NOT NULL,
    tax_category TEXT NOT NULL,
    ending_remaining_amount DECIMAL(15,4) NOT NULL,
    tx_fingerprint TEXT NOT NULL,
    computed_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (year, tax_category)
);

CREATE INDEX IF NOT EXISTS idx_loss_carryforward_snapshots_year ON loss_carryforward_snapshots(year);

-- Metadata table for schema version and app settings
CREATE TABLE IF NOT EXISTS metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Insert initial schema version
INSERT OR IGNORE INTO metadata (key, value) VALUES ('schema_version', '2');
INSERT OR IGNORE INTO metadata (key, value) VALUES ('db_created_at', datetime('now'));
