//! Configuration for chat interface.

use anyhow::{Context, Result};
use dir_spec;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Chat configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatConfig {
    /// LLM endpoint configuration
    #[serde(default)]
    pub endpoint: EndpointConfig,

    /// Approval settings
    #[serde(default)]
    pub approvals: ApprovalsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    /// Base URL for the LLM endpoint (e.g., http://localhost:11434/v1)
    #[serde(default = "default_endpoint")]
    pub base_url: String,

    /// Model name (e.g., gpt-oss-20b)
    #[serde(default = "default_model")]
    pub model: String,

    /// Optional API key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Request timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

impl Default for EndpointConfig {
    fn default() -> Self {
        Self {
            base_url: default_endpoint(),
            model: default_model(),
            api_key: None,
            timeout_secs: default_timeout(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalsConfig {
    /// Default policy for tool groups (prompt/allow/deny)
    #[serde(default)]
    pub default_policy: DefaultPolicy,

    /// Tools/categories that are always allowed without prompting
    #[serde(default)]
    pub always_allow: Vec<String>,
}

impl Default for ApprovalsConfig {
    fn default() -> Self {
        Self {
            default_policy: DefaultPolicy::Prompt,
            always_allow: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum DefaultPolicy {
    /// Ask user before executing (default)
    #[default]
    Prompt,
    /// Allow all tools
    Allow,
    /// Deny all tools
    Deny,
}

fn default_endpoint() -> String {
    "http://localhost:11434/v1".to_string()
}

fn default_model() -> String {
    "gpt-oss-20b".to_string()
}

fn default_timeout() -> u64 {
    120
}

impl ChatConfig {
    /// Get the config file path
    pub fn config_path() -> Result<PathBuf> {
        let home = dir_spec::home().context("Home directory not found")?;
        let config_dir = home.join(".interest");

        // Create directory if it doesn't exist
        std::fs::create_dir_all(&config_dir).context("Failed to create .interest directory")?;

        Ok(config_dir.join("config.toml"))
    }

    /// Load config from file, or return default if file doesn't exist
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;

        let config: Self = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config from {}", path.display()))?;

        Ok(config)
    }

    /// Save config to file
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self).context("Failed to serialize config to TOML")?;

        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write config to {}", path.display()))?;

        Ok(())
    }

    /// Add a tool/category to always_allow list
    pub fn add_always_allow(&mut self, name: String) {
        if !self.approvals.always_allow.contains(&name) {
            self.approvals.always_allow.push(name);
        }
    }

    /// Remove a tool/category from always_allow list
    pub fn remove_always_allow(&mut self, name: &str) {
        self.approvals.always_allow.retain(|n| n != name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ChatConfig::default();
        assert_eq!(config.endpoint.base_url, "http://localhost:11434/v1");
        assert_eq!(config.endpoint.model, "gpt-oss-20b");
        assert_eq!(config.approvals.default_policy, DefaultPolicy::Prompt);
        assert!(config.approvals.always_allow.is_empty());
    }

    #[test]
    fn test_config_serialization() {
        let config = ChatConfig::default();
        let toml = toml::to_string(&config).unwrap();
        let parsed: ChatConfig = toml::from_str(&toml).unwrap();
        assert_eq!(parsed.endpoint.base_url, config.endpoint.base_url);
        assert_eq!(parsed.endpoint.model, config.endpoint.model);
    }

    #[test]
    fn test_add_remove_always_allow() {
        let mut config = ChatConfig::default();
        config.add_always_allow("portfolio_show".to_string());
        assert_eq!(config.approvals.always_allow.len(), 1);

        config.add_always_allow("portfolio_show".to_string());
        assert_eq!(config.approvals.always_allow.len(), 1); // No duplicates

        config.remove_always_allow("portfolio_show");
        assert!(config.approvals.always_allow.is_empty());
    }
}
