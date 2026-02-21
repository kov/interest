# Design: `comms` and `financial` Commands

## Overview

Two new top-level commands to help investors stay informed about their holdings:

- **`comms`** — Fetches and displays regulatory communications (material facts, market
  notices, fund reports) for the assets held in the portfolio, with LLM-powered
  summaries via the same endpoint already used by the `chat` command.
- **`financial`** — Downloads and displays structured financial data (quarterly and
  annual results, FII monthly reports) for portfolio assets.

Both commands follow the same caching / retry / rate-limiting patterns already
established by `prices import-b3` (COTAHIST) and the Tesouro CSV importer.

---

## Data Sources

### 1. CVM ENET — primary source for regulatory filings

The Comissão de Valores Mobiliários (CVM) runs the **ENET** (Empresas NET) portal.
Every publicly-traded company and regulated fund must file material facts,
market communications, quarterly/annual reports and more through this system.

**Full-text search API** (no auth, no rate limit documented, public):
```
GET https://efts.cvm.gov.br/EFTS/full_search
  ?q=<cnpj or company name>
  &dateRange=custom&startdt=YYYY-MM-DD&enddt=YYYY-MM-DD
  &category=<category_code>
```

Returns JSON with an `hits.hits` array; each hit contains:
- `_id` — document identifier
- `_source.NomEmissor` — company name
- `_source.CodCvm` — CVM registration number
- `_source.SigEmissor` — ticker (not always present)
- `_source.DtEntrega` — filing date (format varies by endpoint; verify at
  integration time — may be `YYYY-MM-DDTHH:MM:SS` or `DD/MM/YYYY`)
- `_source.DescTipo` — document type description (e.g. `"Fato Relevante"`,
  `"Comunicado ao Mercado"`, `"Aviso aos Cotistas"`)
- `_source.LinkArquivo` — relative path for document download

**Document type codes** used in the `category` query parameter (partial list):

| Code | Portuguese name | English meaning |
|------|-----------------|-----------------|
| `30` | Fato Relevante | Material Fact |
| `358` | Comunicado ao Mercado | Market Communication |
| `57` | Aviso aos Cotistas (FII) | Notice to Unitholders |
| `59` | Informe Trimestral de FII | FII Quarterly Report |
| `61` | Informe Mensal de FII | FII Monthly Report |
| `44` | DFP | Annual Financial Report |
| `48` | ITR | Quarterly Financial Report |

**Document download URL** (no auth, public):
```
GET https://www.rad.cvm.gov.br/ENET/frmDownloadDocumento.aspx
  ?Tela=ext
  &numSequencia=<_id>
  &numVersao=1
  &numProtocolo=<protocol from _source>
  &descTipo=<DescTipo>
  &CodigoInstituicao=1
```
Returns the actual filing file (PDF, ZIP, or HTML).

### 2. CVM Open Data flat files — bulk data download

For bulk / historical imports, CVM publishes structured CSV datasets at
`https://dados.cvm.gov.br/dados/`.  No auth or API key required.

Relevant paths:

| Dataset | URL |
|---------|-----|
| Company registry | `CIA_ABERTA/CAD/DADOS/cad_cia_aberta.csv` |
| Material Facts index | `CIA_ABERTA/DOC/FRE/DADOS/` (annual ZIPs with CSV index) |
| Annual reports (DFP) | `CIA_ABERTA/DOC/DFP/DADOS/dfp_cia_aberta_YYYY.zip` |
| Quarterly reports (ITR) | `CIA_ABERTA/DOC/ITR/DADOS/itr_cia_aberta_YYYY.zip` |
| FII monthly report | `FII/DOC/INF_MENSAL/DADOS/inf_mensal_fii_YYYY_MM.csv` |
| FII quarterly report | `FII/DOC/INF_TRIM/DADOS/inf_trim_fii_YYYY.zip` |

The base URL for all flat files is:
```
https://dados.cvm.gov.br/dados/
```

### 3. B3 Issuer Platform — secondary source, equities focus

B3 exposes an API for company-filed market notices:
```
GET https://sistemasweb.b3.com.br/PlataformaInformacaoB3/ComunicadoInfo
  ?empresasSelecionadas=<B3-registered code>
  &idioma=pt-br
  &tipoPublicacao=<type>
  &dataInicio=YYYY-MM-DD
  &dataFim=YYYY-MM-DD
  &pagina=1
  &tamanhoPagina=20
```

Returns JSON.  Each record contains the filing text or a link to download it.

**Recommendation**: Use CVM ENET as the primary source (covers all asset types)
and B3 as a supplementary source when CNPJ matching is unavailable.

---

## Ticker → CNPJ / CVM Code Mapping

All CVM queries require either a CNPJ or the CVM registration code (`CodCvm`).
The mapping can be obtained from two places already in the codebase:

1. **Asset registry** — the `asset_registry` table already stores CNPJ when
   synced from Mais Retorno (`sync-maisretorno`).  Prefer this when available.
2. **CVM company CSV** — `cad_cia_aberta.csv` from dados.cvm.gov.br maps
   ticker → CNPJ → CodCvm.  This file (~2 MB) should be cached locally
   (24-hour TTL, same pattern as the Tesouro CSV).

FII tickers follow the same process; their CNPJ is in the same CVM registry.

---

## Command Design

### `comms`

```
interest comms [SUBCOMMAND]
```

#### Sub-commands

| Sub-command | Description |
|-------------|-------------|
| `show` | Show recent communications for portfolio assets (default: last 30 days) |
| `fetch <TICKER>` | Fetch and display communications for a specific ticker |
| `download <TICKER>` | Download and cache all historical filings for a ticker |
| `clear-cache [TICKER]` | Clear cached communications (optional ticker filter) |

#### `comms show` flags

| Flag | Default | Description |
|------|---------|-------------|
| `--days <N>` | `30` | How many days back to look |
| `--since <YYYY-MM-DD>` | — | Explicit start date (overrides `--days`) |
| `--type <type>` | all | Filter: `material`, `notice`, `all` |
| `--ticker <T>` | — | Restrict to a single portfolio asset |
| `--no-summary` | — | Skip LLM summaries, show raw titles |
| `--json` | — | JSON output (global flag, also honoured here) |

#### `comms download` flags

| Flag | Default | Description |
|------|---------|-------------|
| `--since <YYYY-MM-DD>` | `2010-01-01` | Start of historical window |
| `--no-cache` | — | Force re-download even if cached |

### `financial`

```
interest financial [SUBCOMMAND]
```

#### Sub-commands

| Sub-command | Description |
|-------------|-------------|
| `show` | Show most recent financial highlights for portfolio assets |
| `fetch <TICKER>` | Fetch financial data for a specific ticker |
| `download <TICKER>` | Download and cache historical financials |
| `clear-cache [TICKER]` | Clear cached financial data |

#### `financial show` flags

| Flag | Default | Description |
|------|---------|-------------|
| `--ticker <T>` | — | Restrict to a single asset |
| `--year <YYYY>` | current year | Year to display |
| `--no-summary` | — | Skip LLM summaries |
| `--json` | — | JSON output |

---

## Caching Strategy

Cache directories follow the existing XDG convention (`~/.cache/interest/`):

```
~/.cache/interest/
  comms/
    <CNPJ>/
      index.json          # JSON index of known filings (updated per fetch)
      <filing-id>.pdf     # downloaded PDF
      <filing-id>.html    # downloaded HTML (if applicable)
      <filing-id>.txt     # pdftotext output (extracted text)
  financial/
    cvm_registry.csv      # cad_cia_aberta.csv (24h TTL)
    <CNPJ>/
      inf_mensal_<YYYY_MM>.csv
      dfp_<YYYY>.zip
      itr_<YYYY>_QN.zip
```

Cache validity tracking in the `metadata` table (same as COTAHIST):

| Key | Value | Description |
|-----|-------|-------------|
| `comms_last_fetched_<CNPJ>` | Unix timestamp | Last ENET search for this CNPJ |
| `comms_registry_refreshed_at` | Unix timestamp | Last refresh of CVM registry CSV |
| `financial_last_fetched_<CNPJ>` | Unix timestamp | Last financial data fetch for this CNPJ |

### TTL recommendations

| Resource | TTL | Rationale |
|----------|-----|-----------|
| ENET index (new filings check) | 1 hour | Filings arrive during market hours |
| Cached filing files (PDF/HTML) | Permanent (immutable once filed) | CVM filings never change |
| CVM company registry CSV | 24 hours | Low churn, same as Tesouro CSV |
| FII monthly report CSV | 24 hours | Published monthly |
| DFP / ITR ZIP | Permanent (once published) | Published once per period |

---

## Rate Limiting and Retries

### Rate limiting

CVM ENET is a public API with no documented rate limits, but should be treated
politely.  Recommended defaults:

- **Per-request delay**: 500 ms between requests within the same ticker batch
- **Concurrent requests**: maximum 2 concurrent in-flight (using a semaphore)
- **Batch delay**: 2 s between different tickers in `comms show`

B3 platform: same defaults.

The delay values should be runtime-configurable via `~/.interest/config.toml`:

```toml
[comms]
request_delay_ms = 500
max_concurrent = 2
```

### Retry strategy

Adopt the same pattern used elsewhere (reqwest client with retry logic):

- **Max retries**: 3
- **Retry on**: HTTP 429 (Too Many Requests), 503, 504, and network timeouts
- **Backoff**: exponential, starting at 1 s, doubling per attempt, capped at 30 s
- **Respect `Retry-After`**: if the 429 response includes a `Retry-After` header,
  honour it exactly

### Offline mode

Honour `INTEREST_OFFLINE=1` (same as COTAHIST):
- Use cached data if available
- Return an error if cache is missing

---

## LLM Integration

The `comms show` and `financial show` commands call the same LLM endpoint
already used by the `chat` command (`src/chat/llm.rs`).

### Workflow

1. Retrieve the list of recent filings for the portfolio (from cache or ENET).
2. For each filing:
   a. If the file has not been downloaded yet, download it.
   b. For PDF files, run `pdftotext -layout <file.pdf> -` to extract text.
   c. Truncate the text to a configurable character limit (default: 4 000 characters)
      before sending to the LLM, to stay within a reasonable context budget.
3. Send the extracted text to the LLM with a system prompt such as:
   > "You are summarising a Brazilian regulatory filing for an investor.
   > Provide a concise summary (3–5 sentences) in the same language as the
   > document.  Highlight any price-sensitive information, risks, or
   > significant operational developments."
4. Display the LLM summary below each filing title/date in the terminal output.

### pdftotext invocation

```
pdftotext -layout -enc UTF-8 <input.pdf> <output.txt>
```

> **Note**: the `-enc` flag name may vary by Poppler version; `-enc UTF-8` and
> `-encoding UTF-8` are both seen in the wild.  At integration time, verify with
> `pdftotext --help` and use the form supported by the installed version.
> If no encoding flag is needed (UTF-8 is the default in modern Poppler builds),
> the flag can be omitted.

- The binary is called via `std::process::Command` (no shell).
- If `pdftotext` is not found on `PATH`, fall back to displaying only the title
  with a warning message.
- Extracted text is cached as `<filing-id>.txt` alongside the PDF.

### LLM configuration

Re-uses the existing `ChatConfig` / `EndpointConfig` from `src/chat/config.rs`.
No new config keys required.  When the LLM endpoint is unreachable,
`--no-summary` behaviour is applied automatically (with a warning).

---

## Database Schema Changes

A new table tracks filings metadata for efficient querying and deduplication:

```sql
CREATE TABLE IF NOT EXISTS comms_filings (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    cnpj          TEXT    NOT NULL,
    ticker        TEXT,                      -- nullable (not always available from CVM)
    filing_id     TEXT    NOT NULL,          -- CVM _id (unique across all filings)
    filing_date   DATE    NOT NULL,
    category      TEXT    NOT NULL,          -- e.g. 'Fato Relevante', 'Comunicado ao Mercado'
    title         TEXT,
    file_path     TEXT,                      -- relative path inside cache dir (nullable until downloaded)
    text_path     TEXT,                      -- path to pdftotext output (nullable)
    llm_summary   TEXT,                      -- cached LLM summary (nullable)
    source        TEXT    NOT NULL DEFAULT 'CVM_ENET',
    created_at    DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(source, filing_id)
);

CREATE INDEX IF NOT EXISTS idx_comms_filings_cnpj_date
    ON comms_filings(cnpj, filing_date DESC);

CREATE INDEX IF NOT EXISTS idx_comms_filings_ticker_date
    ON comms_filings(ticker, filing_date DESC);
```

A second table for financial data reports:

```sql
CREATE TABLE IF NOT EXISTS financial_reports (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    cnpj        TEXT    NOT NULL,
    ticker      TEXT,
    report_type TEXT    NOT NULL,  -- 'DFP', 'ITR', 'INF_MENSAL_FII', 'INF_TRIM_FII'
    period      TEXT    NOT NULL,  -- 'YYYY' for DFP, 'YYYY-QN' for ITR, 'YYYY-MM' for FII monthly
    file_path   TEXT,
    llm_summary TEXT,
    source      TEXT    NOT NULL DEFAULT 'CVM',
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(cnpj, report_type, period)
);
```

---

## Terminal Output Format

### `comms show` — with summaries

```
Communications for your portfolio (last 30 days)
─────────────────────────────────────────────────────────────────────────────

PETR4 — Petróleo Brasileiro SA (CNPJ 33.000.167/0001-01)

  2025-11-15  Fato Relevante
  Title: Petrobras informa sobre acordo com ANP
  ✦ Summary: Petrobras announced a settlement with ANP regarding royalty
    payments worth R$ 2.1 billion. The agreement resolves a dispute from
    2019 and is expected to be paid in three instalments by mid-2026.
    No material impact on production guidance.

MXRF11 — Maxi Renda FII

  2025-11-14  Aviso aos Cotistas
  Title: MXRF11 - Rendimento referente ao mês de novembro/2025
  ✦ Summary: Monthly dividend notice for November 2025.  Payout of R$ 0.10
    per quota, ex-date 2025-11-20, payment 2025-11-28.  No extraordinary
    items.
```

### `comms show --no-summary`

```
PETR4   2025-11-15  Fato Relevante             Petrobras informa sobre acordo…
MXRF11  2025-11-14  Aviso aos Cotistas          MXRF11 - Rendimento referente…
```

### `comms show --json`

Returns a JSON array with all filing metadata plus the `llm_summary` field.

---

## Implementation Plan

The work can be broken into the following phases:

### Phase 1 — Infrastructure

1. **CVM company registry client** (`src/comms/registry.rs`)
   - Download and cache `cad_cia_aberta.csv`
   - Provide `ticker_to_cnpj(ticker) -> Option<String>` lookup
   - Also populate `CodCvm` for ENET queries

2. **ENET client** (`src/comms/enet.rs`)
   - Search filings by CNPJ and date range
   - Download individual filing files (PDF / HTML)
   - Rate-limiting semaphore + exponential back-off retry wrapper

3. **Database schema migration** — add `comms_filings` and `financial_reports`
   tables in `src/db/schema.sql`

4. **pdftotext extraction helper** (`src/comms/pdf.rs`)
   - Wrap `std::process::Command` for `pdftotext -layout`
   - Check for binary availability; emit warning if missing

### Phase 2 — `comms` command

5. **Command definition** — add `Comms` subcommand to `src/cli/mod.rs`
   (matching the sub-command table above)

6. **Dispatcher** — `src/dispatcher/comms.rs`
   - `dispatch_comms_show` — queries `comms_filings` (or fetches if stale),
     renders table/detailed view
   - `dispatch_comms_fetch` — fetches and stores filings for one ticker
   - `dispatch_comms_download` — historical download with progress events
   - `dispatch_comms_clear_cache` — clears cached files and DB rows

7. **LLM summary integration** — shared helper that calls
   `crate::chat::llm::LlmClient` with the filing text

### Phase 3 — `financial` command

8. **CVM financial data client** (`src/comms/financial.rs`)
   - Download DFP, ITR ZIPs and FII monthly CSVs
   - Parse key financial metrics (revenue, net income, dividends paid, NAV for FIIs)

9. **Command definition + dispatcher** — same structure as phase 2

### Phase 4 — Polish

10. **TUI completion** — add `comms` and `financial` to
    `src/ui/tui.rs` `COMMAND_PATTERNS`

11. **Chat tool** — expose `comms_show` as an LLM tool in `src/chat/tools.rs`
    so the chat interface can answer "what's new with PETR4?"

12. **Tests**
    - Unit tests for ENET JSON parsing, pdftotext wrapper, registry CSV parsing
    - Integration tests using recorded HTTP fixtures (similar to existing tests)

---

## Error Handling

| Situation | Behaviour |
|-----------|-----------|
| `pdftotext` not installed | Warn once; skip summaries; show titles only |
| LLM endpoint unreachable | Warn once; show filing titles without summary |
| ENET search returns 429 | Retry with exponential back-off (up to 3 attempts) |
| Ticker has no CNPJ in registry | Print warning; skip ticker; continue with others |
| PDF download fails | Skip file; store metadata without `file_path` |
| Network error in offline mode | Return clear error message, cite `INTEREST_OFFLINE` |

---

## Open Questions

1. **Language of summaries** — Should the prompt force Portuguese summaries
   (most filings are in Portuguese) or let the LLM detect and match?
   Suggested default: match document language, configurable via config.toml.

2. **Debenture / bond CNPJ** — Debenture tickers are not always directly mapped
   to a CNPJ in the CVM company registry; the CNPJ is that of the issuing company.
   The maisretorno registry stores CNPJ for bonds but requires a sync first.

3. **FIP / ETF coverage** — CVM covers FIPs and ETFs but the filing types differ.
   Initial implementation should focus on stocks, FIIs, and FIAGROs (highest value
   to typical users) and add other asset types iteratively.

4. **B3 API stability** — The B3 Issuer Platform API (`sistemasweb.b3.com.br`) is
   undocumented and could change without notice.  Consider it optional / secondary
   to the more stable CVM ENET.

5. **Rate limit discovery** — Actual CVM ENET rate limits should be empirically
   tested; start conservatively (500 ms delay) and the user can tune via config.

6. **`financial` scope for stocks** — For stocks, "financial data" means DFP/ITR
   filings (structured accounting CSVs).  These are large files; consider whether
   to download the full ZIP or just the specific company's rows via the ENET search.
