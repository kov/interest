// Scraper for br.investing.com corporate actions data
//
// Uses headless Chrome to bypass Cloudflare protection and extract
// corporate action data (splits, reverse splits, bonuses) from the website.
//
// Note: B3 COTAHIST downloader no longer uses Chrome (uses direct URLs),
// but this scraper still needs it for investing.com.

use anyhow::{Context, Result};
use headless_chrome::{Browser, LaunchOptions};
use std::time::Duration;
use tracing::{info, warn};

use crate::db::{CorporateAction, CorporateActionType};

/// Scraper for investing.com corporate actions
pub struct InvestingScraper {
    browser: Browser,
}

impl InvestingScraper {
    /// Create a new scraper instance with headless Chrome
    pub fn new() -> Result<Self> {
        info!("Launching headless Chrome browser");

        let options = LaunchOptions {
            headless: true,
            sandbox: false, // May be needed on some systems
            args: vec![
                // Disable automation flags to avoid detection
                std::ffi::OsStr::new("--disable-blink-features=AutomationControlled"),
                // Set a realistic user agent
                std::ffi::OsStr::new("--user-agent=Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"),
                // Disable various detection methods
                std::ffi::OsStr::new("--disable-dev-shm-usage"),
                std::ffi::OsStr::new("--disable-web-security"),
                // Set window size
                std::ffi::OsStr::new("--window-size=1920,1080"),
            ],
            ..Default::default()
        };

        let browser = Browser::new(options)
            .context("Failed to launch headless Chrome. Is Chrome/Chromium installed?")?;

        Ok(Self { browser })
    }

    /// Scrape corporate actions for a ticker from investing.com
    ///
    /// Returns a list of corporate actions found on the page.
    /// URL must be the full investing.com splits/reverse-splits page URL.
    pub fn scrape_corporate_actions(
        &self,
        url: &str,
        original_url: &str,
    ) -> Result<Vec<CorporateAction>> {
        info!("Scraping corporate actions from: {}", url);

        // Create a new tab
        let tab = self
            .browser
            .new_tab()
            .context("Failed to create new browser tab")?;

        // Navigate to the URL
        tab.navigate_to(url).context("Failed to navigate to URL")?;

        // Wait for the page to load and Cloudflare to complete
        info!("Waiting for page to load (Cloudflare check)...");

        // First wait for body to ensure page has started loading
        tab.wait_for_element_with_custom_timeout("body", Duration::from_secs(10))
            .context("Timed out waiting for page to load")?;

        // Cloudflare Turnstile challenge can take up to 10 seconds
        info!("Waiting for Cloudflare Turnstile challenge to complete (this may take 10-60 seconds)...");
        std::thread::sleep(Duration::from_secs(15));

        // Try to detect if we're still on the challenge page
        let html_check = tab
            .get_content()
            .context("Failed to get page content for challenge check")?;

        if html_check.contains("Just a moment") || html_check.contains("Verify you are human") {
            info!("Still on Cloudflare challenge page, waiting longer...");
            std::thread::sleep(Duration::from_secs(30));
        }

        // Now try to find the table - try multiple selectors
        info!("Looking for splits table...");
        let table_found = tab
            .wait_for_element_with_custom_timeout("table", Duration::from_secs(20))
            .is_ok();

        if !table_found {
            warn!("No table element found, will attempt to extract and check content");
        }

        // Extract the table HTML
        let html = tab.get_content().context("Failed to get page content")?;

        // Debug: save HTML to file for inspection
        if let Err(e) = std::fs::write("/tmp/investing_page.html", &html) {
            warn!("Failed to save debug HTML: {}", e);
        } else {
            info!("Saved page HTML to /tmp/investing_page.html for debugging");
        }

        // Parse the HTML for split data
        self.parse_splits_html(&html, original_url)
    }

    /// Parse HTML content to extract split information from Next.js JSON data
    ///
    /// investing.com uses Next.js with data embedded as JSON in __NEXT_DATA__ script tag
    /// The splits data is in: pageProps.splitsStore.splits
    ///
    /// Note: investing.com shows both forward and reverse splits on the same page.
    /// The ratio indicates the type:
    /// - ratio > 1 = forward split (e.g., 8 = 8:1 split)
    /// - ratio < 1 = reverse split (e.g., 0.125 = 1:8 reverse split)
    fn parse_splits_html(&self, html: &str, _url: &str) -> Result<Vec<CorporateAction>> {
        use chrono::DateTime;

        info!("Parsing HTML for splits data");

        // Extract the __NEXT_DATA__ JSON from the page
        let json_start = html
            .find("__NEXT_DATA__")
            .context("Could not find __NEXT_DATA__ in page")?;

        let json_content_start = html[json_start..]
            .find(">")
            .context("Could not find start of JSON data")?;

        let json_start_pos = json_start + json_content_start + 1;

        let json_end = html[json_start_pos..]
            .find("</script>")
            .context("Could not find end of JSON data")?;

        let json_str = &html[json_start_pos..json_start_pos + json_end];

        // Parse the JSON
        let json: serde_json::Value =
            serde_json::from_str(json_str).context("Failed to parse JSON data")?;

        // Navigate to the splits array: props.pageProps.state.historicalDataSplitsStore.splits
        let splits = json
            .get("props")
            .and_then(|p| p.get("pageProps"))
            .and_then(|pp| pp.get("state"))
            .and_then(|s| s.get("historicalDataSplitsStore"))
            .and_then(|hs| hs.get("splits"))
            .and_then(|s| s.as_array())
            .context("Could not find splits array in JSON data")?;

        let mut actions = Vec::new();

        for split in splits {
            // Parse date from ISO 8601 format
            let date_str = split
                .get("date")
                .and_then(|d| d.as_str())
                .context("Missing or invalid date field")?;

            let datetime = DateTime::parse_from_rfc3339(date_str)
                .with_context(|| format!("Failed to parse date: {}", date_str))?;
            let date = datetime.date_naive();

            // Parse ratio - investing.com can use either integer or decimal
            // ratio >= 1 means forward split (e.g., 8 = 1:8 split, 1.05 = 1:1.05 split)
            // ratio < 1 means reverse split (e.g., 0.125 = 1:8 reverse split)
            let ratio_value = split
                .get("ratio")
                .and_then(|r| r.as_f64())
                .context("Missing or invalid ratio field")?;

            // Determine split type based on ratio value
            let (action_type, ratio_from, ratio_to) = if ratio_value >= 1.0 {
                // Forward split (desdobramento): 1 share becomes 'ratio' shares
                // e.g., ratio 8.0 = 1:8 split (price ÷8, shares ×8)
                // e.g., ratio 1.05 = 1:1.05 split (price ÷1.05, shares ×1.05)
                let ratio_int = ratio_value.round() as i32;
                (CorporateActionType::Split, 1, ratio_int)
            } else {
                // Reverse split (grupamento): 'ratio' shares become 1
                // e.g., ratio 0.125 = 1:8 reverse split → 8:1 (price ×8, shares ÷8)
                // e.g., ratio 0.1 = 1:10 reverse split → 10:1 (price ×10, shares ÷10)
                let inverse = (1.0 / ratio_value).round() as i32;
                (CorporateActionType::ReverseSplit, inverse, 1)
            };

            // Create corporate action (without asset_id, will be set by caller)
            let action = CorporateAction {
                id: None,
                asset_id: 0, // Placeholder, will be set by caller
                action_type: action_type.clone(),
                event_date: date,
                ex_date: date, // investing.com doesn't distinguish, use same date
                ratio_from,
                ratio_to,
                source: "INVESTING.COM".to_string(),
                notes: Some(format!(
                    "Scraped from investing.com (ratio: {})",
                    ratio_value
                )),
                created_at: chrono::Utc::now(),
            };

            actions.push(action);
            info!(
                "Found split: {} {}:{} on {}",
                action_type.as_str(),
                ratio_from,
                ratio_to,
                date
            );
        }

        if actions.is_empty() {
            warn!("No splits found in JSON data");
        } else {
            info!("Parsed {} split(s) from page", actions.len());
        }

        Ok(actions)
    }

    /// Build investing.com URL for a ticker's splits page
    ///
    /// This is a heuristic approach - may not work for all tickers.
    /// Better approach would be to search for the ticker first.
    pub fn build_splits_url(_ticker: &str, company_name: &str) -> String {
        // investing.com uses URL slugs like:
        // /equities/advanced-micro-devices-inc-sao-historical-data-splits

        let slug = company_name
            .to_lowercase()
            .replace(" ", "-")
            .replace(".", "")
            .replace(",", "");

        format!(
            "https://br.investing.com/equities/{}-sao-historical-data-splits",
            slug
        )
    }
}

impl Default for InvestingScraper {
    fn default() -> Self {
        Self::new().expect("Failed to create default InvestingScraper")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_splits_url() {
        let url = InvestingScraper::build_splits_url("A1MD34", "Advanced Micro Devices Inc");
        assert_eq!(
            url,
            "https://br.investing.com/equities/advanced-micro-devices-inc-sao-historical-data-splits"
        );
    }
}
