use crate::ui::progress::ProgressPrinter;
use anyhow::{Context, Result};
use scraper::{Html, Selector};
use std::time::Duration;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

const AMBIMA_BASE_URL: &str = "https://data.anbima.com.br/debentures";

pub fn is_debenture(ticker: &str) -> Result<bool> {
    let url = format!("{}/{}/caracteristicas", AMBIMA_BASE_URL, ticker);
    let prefix = format!("Checking if {} is a bond: ", ticker);
    let printer = ProgressPrinter::new(false);
    printer.handle_event(&crate::ui::progress::ProgressEvent::Spinner {
        message: format!("{}Opening Ambima page", prefix),
    });
    let browser = headless_chrome::Browser::default()
        .context("Failed to start headless Chrome for Ambima lookup")?;
    let tab = browser
        .new_tab()
        .context("Failed to open tab for Ambima lookup")?;

    tab.navigate_to(&url)
        .context("Failed to navigate Ambima page")?;
    printer.handle_event(&crate::ui::progress::ProgressEvent::Spinner {
        message: format!("{}Waiting for Ambima page shell", prefix),
    });
    tab.wait_for_element_with_custom_timeout("body", Duration::from_secs(5))
        .context("Timed out waiting for Ambima body")?;

    if tab
        .wait_for_element_with_custom_timeout("main.container", Duration::from_secs(10))
        .is_err()
    {
        let html = tab.get_content().context("Failed to read Ambima HTML")?;
        if is_not_found_page(&html) {
            printer.handle_event(&crate::ui::progress::ProgressEvent::Info {
                message: format!("{} not found on Ambima", ticker),
            });
            return Ok(false);
        }
    }

    printer.handle_event(&crate::ui::progress::ProgressEvent::Spinner {
        message: format!("{}Waiting for Ambima debenture details", prefix),
    });
    let html = wait_for_ambima_details(&tab, ticker, &printer, &prefix)?;
    let details = parse_debenture_details(&html, Some(ticker));
    if let Some(details) = details {
        printer.handle_event(&crate::ui::progress::ProgressEvent::Success {
            message: format!(
                "{} is a bond expiring on {} ({})",
                details.ticker, details.maturity_date, details.remuneration
            ),
        });
        Ok(true)
    } else {
        printer.handle_event(&crate::ui::progress::ProgressEvent::Info {
            message: format!("{} not found on Ambima", ticker),
        });
        Ok(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebentureDetails {
    pub ticker: String,
    pub issuer: String,
    pub sector: Option<String>,
    pub issue_date: Option<String>,
    pub maturity_date: String,
    pub remuneration: String,
    pub law_flag: Option<String>,
}

fn parse_debenture_details(html: &str, fallback_ticker: Option<&str>) -> Option<DebentureDetails> {
    let document = Html::parse_document(html);
    let title_sel = Selector::parse("h2.title-normal").ok()?;
    let page_title_sel = Selector::parse("title").ok()?;
    let flag_sel = Selector::parse("span.anbima-ui-flag label.flag__children").ok()?;
    let item_sel = Selector::parse("ul > li").ok()?;
    let label_sel = Selector::parse("span.small-text").ok()?;
    let value_sel = Selector::parse("span.normal-text").ok()?;
    let output_sel = Selector::parse("div.anbima-ui-output__container").ok()?;
    let output_label_sel = Selector::parse("span.anbima-ui-output__label").ok()?;
    let output_value_sel = Selector::parse("span.anbima-ui-output__value").ok()?;

    let title_text = document
        .select(&page_title_sel)
        .next()
        .map(|node| node.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let mut ticker = extract_ticker_from_text(&title_text);

    if ticker.is_none() {
        for node in document.select(&title_sel) {
            let title = node.text().collect::<Vec<_>>().join(" ");
            ticker = extract_ticker_from_text(&title);
            if ticker.is_some() {
                break;
            }
        }
    }

    let ticker = ticker
        .or_else(|| fallback_ticker.map(|value| value.trim().to_string()))
        .unwrap_or_else(|| "UNKNOWN".to_string());

    let law_flag = document.select(&flag_sel).next().map(|node| {
        node.text()
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    });

    let mut pairs: Vec<(String, String)> = Vec::new();

    for item in document.select(&item_sel) {
        let label = item
            .select(&label_sel)
            .next()
            .map(|node| node.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        let value = item
            .select(&value_sel)
            .next()
            .map(|node| node.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        let label = label.trim();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        pairs.push((label.to_string(), value.to_string()));
    }

    for item in document.select(&output_sel) {
        let label = item
            .select(&output_label_sel)
            .next()
            .map(|node| node.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        let value = item
            .select(&output_value_sel)
            .next()
            .map(|node| node.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        let label = label.trim();
        let value = value.trim();
        if label.is_empty() || value.is_empty() {
            continue;
        }
        pairs.push((label.to_string(), value.to_string()));
    }

    let issuer = find_value(&pairs, &["Emissor", "Empresa"])?;
    let sector = find_value(&pairs, &["Setor"]);
    let issue_date = find_value(&pairs, &["Data de emissão", "Data de emissao"]);
    let maturity_date = find_value(&pairs, &["Data de vencimento"])?;
    let remuneration = find_value(&pairs, &["Remuneração", "Remuneracao"])?;

    Some(DebentureDetails {
        ticker,
        issuer,
        sector,
        issue_date,
        maturity_date,
        remuneration,
        law_flag,
    })
}

fn extract_ticker_from_text(text: &str) -> Option<String> {
    let title_token = text.split('|').next().unwrap_or(text).trim();
    for token in title_token.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        let token_upper = token.to_ascii_uppercase();
        if is_ticker_candidate(&token_upper) {
            return Some(token_upper);
        }
    }
    None
}

fn is_ticker_candidate(token: &str) -> bool {
    let mut letter_count = 0;
    let mut digit_count = 0;
    let mut saw_digit = false;

    for ch in token.chars() {
        if ch.is_ascii_uppercase() {
            if saw_digit {
                return false;
            }
            letter_count += 1;
        } else if ch.is_ascii_digit() {
            saw_digit = true;
            digit_count += 1;
        } else {
            return false;
        }
    }

    (4..=6).contains(&letter_count) && (1..=2).contains(&digit_count)
}

fn find_value(pairs: &[(String, String)], keys: &[&str]) -> Option<String> {
    let normalized_keys: Vec<String> = keys.iter().map(|k| normalize_label(k)).collect();
    for (label, value) in pairs {
        let normalized_label = normalize_label(label);
        if normalized_keys.iter().any(|k| k == &normalized_label) {
            return Some(value.clone());
        }
    }
    None
}

fn normalize_label(input: &str) -> String {
    let upper = input.to_uppercase();
    let mut out = String::with_capacity(upper.len());
    for ch in upper.nfkd() {
        if is_combining_mark(ch) {
            continue;
        }
        if ch.is_ascii_alphanumeric() || ch == ' ' {
            out.push(ch);
        } else if ch == '-' {
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_not_found_page(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("página não encontrada")
        || lower.contains("pagina nao encontrada")
        || lower.contains("não encontramos")
        || lower.contains("nao encontramos")
}

fn wait_for_ambima_details(
    tab: &headless_chrome::Tab,
    ticker: &str,
    printer: &ProgressPrinter,
    prefix: &str,
) -> Result<String> {
    let ticker = ticker.trim().to_ascii_uppercase();
    let agenda_path = format!("/debentures/{}/agenda", ticker);
    let mut attempts = 0;
    let max_attempts = 20;
    let delay = Duration::from_millis(500);
    let quick_bail_attempts = 3;

    loop {
        let html = tab.get_content().context("Failed to read Ambima HTML")?;
        if is_not_found_page(&html) {
            tracing::debug!("Ambima check {} found not-found page", ticker);
            printer.handle_event(&crate::ui::progress::ProgressEvent::Spinner {
                message: format!("{}Ambima not found page detected", prefix),
            });
            return Ok(html);
        }
        let lower = html.to_ascii_lowercase();
        let has_agenda = lower.contains(&agenda_path.to_ascii_lowercase());
        let has_remuneracao = lower.contains("remuneração") || lower.contains("remuneracao");
        let has_card = lower.contains("anbima-ui-card__content");
        tracing::debug!(
            "Ambima check {} attempt {} agenda={} remuneracao={} card={}",
            ticker,
            attempts + 1,
            has_agenda,
            has_remuneracao,
            has_card
        );
        printer.handle_event(&crate::ui::progress::ProgressEvent::Spinner {
            message: format!(
                "{}Ambima details (agenda={}, remuneracao={}, card={}, attempt {}/{})",
                prefix,
                has_agenda,
                has_remuneracao,
                has_card,
                attempts + 1,
                max_attempts
            ),
        });

        if has_agenda && has_remuneracao {
            return Ok(html);
        }
        attempts += 1;
        if attempts >= quick_bail_attempts && !has_agenda && !has_remuneracao {
            tracing::debug!(
                "Ambima check {} bailing early (no agenda/remuneracao)",
                ticker
            );
            printer.handle_event(&crate::ui::progress::ProgressEvent::Spinner {
                message: format!("{}No Ambima debenture markers found yet", prefix),
            });
            return Ok(html);
        }
        if attempts >= max_attempts {
            printer.handle_event(&crate::ui::progress::ProgressEvent::Spinner {
                message: format!("{}Timed out waiting for Ambima debenture details", prefix),
            });
            return Ok(html);
        }
        std::thread::sleep(delay);
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_ticker_from_text, parse_debenture_details};

    #[test]
    fn ambima_html_detects_debenture_page() {
        let html = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/ambima_LAMEA6.html"
        ));
        let details = parse_debenture_details(html, None).expect("expected debenture details");
        assert_eq!(details.ticker, "LAMEA6");
        assert!(details.issuer.contains("AMERICANAS"));
        assert_eq!(details.maturity_date, "19/01/2023");
        assert_eq!(details.remuneration, "IPCA + 7,4000%");
    }

    #[test]
    fn ambima_html_ignores_non_debenture_page() {
        let html = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/ambima_LUGG11.html"
        ));
        assert!(parse_debenture_details(html, None).is_none());
    }

    #[test]
    fn ambima_title_extracts_ticker() {
        assert_eq!(
            extract_ticker_from_text("LAMEA6 | ANBIMA Data"),
            Some("LAMEA6".to_string())
        );
        assert_eq!(extract_ticker_from_text("1ª Série"), None);
    }

    #[test]
    #[ignore]
    fn ambima_online_is_debenture() {
        let result = super::is_debenture("LAMEA6").unwrap();
        assert!(result);
    }
}
