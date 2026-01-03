// Web scraping module for extracting data from websites
// Uses headless Chrome to bypass Cloudflare protection

pub mod investing;

pub use investing::InvestingScraper;
