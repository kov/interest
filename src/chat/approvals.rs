//! Approval system for tool execution.

use std::collections::{HashMap, HashSet};

use crate::chat::config::{ApprovalsConfig, DefaultPolicy};

/// Policy group for categorizing tools
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PolicyGroup {
    /// Read-only operations (portfolio show, tax report, etc.)
    Read,
    /// Data-mutating operations (import, add transaction, etc.)
    Mutate,
    /// SQL query execution (read-only by default)
    Sql,
}

impl PolicyGroup {
    pub fn as_str(&self) -> &'static str {
        match self {
            PolicyGroup::Read => "read",
            PolicyGroup::Mutate => "mutate",
            PolicyGroup::Sql => "sql",
        }
    }
}

/// Approval decision for a tool
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Allow this one time
    AllowOnce,
    /// Allow this tool in this session
    AllowSession,
    /// Allow this tool always (persisted to config)
    AllowAlways,
    /// Cancel execution
    Cancel,
}

/// Session state for approvals
pub struct ApprovalState {
    /// Tools allowed in this session
    session_allow: HashSet<String>,

    /// Category-level session overrides
    category_session: HashMap<PolicyGroup, bool>,
}

impl Default for ApprovalState {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalState {
    pub fn new() -> Self {
        Self {
            session_allow: HashSet::new(),
            category_session: HashMap::new(),
        }
    }

    fn is_allowed_without_prompt(
        &self,
        tool_name: &str,
        group: PolicyGroup,
        config: &ApprovalsConfig,
    ) -> bool {
        // Check always_allow in config
        if config.always_allow.contains(&tool_name.to_string()) {
            return true;
        }

        // Check category in always_allow
        if config.always_allow.contains(&group.as_str().to_string()) {
            return true;
        }

        // Check session allow
        if self.session_allow.contains(tool_name) {
            return true;
        }

        // Check category session override
        if let Some(&allowed) = self.category_session.get(&group) {
            return allowed;
        }

        false
    }

    /// Check if a tool should be executed based on config and session state
    pub fn should_prompt(
        &self,
        tool_name: &str,
        group: PolicyGroup,
        config: &ApprovalsConfig,
    ) -> bool {
        if self.is_allowed_without_prompt(tool_name, group, config) {
            return false;
        }

        // Default to prompting for mutate/sql, allow for read
        match group {
            PolicyGroup::Read => false,
            PolicyGroup::Mutate | PolicyGroup::Sql => {
                matches!(config.default_policy, DefaultPolicy::Prompt)
            }
        }
    }

    /// Check if a tool is denied by default policy (unless explicitly allowed)
    pub fn is_denied(&self, tool_name: &str, group: PolicyGroup, config: &ApprovalsConfig) -> bool {
        if group == PolicyGroup::Read {
            return false;
        }

        if !matches!(config.default_policy, DefaultPolicy::Deny) {
            return false;
        }

        !self.is_allowed_without_prompt(tool_name, group, config)
    }

    /// Record an approval decision
    pub fn record_decision(
        &mut self,
        tool_name: &str,
        _group: PolicyGroup,
        decision: ApprovalDecision,
    ) {
        match decision {
            ApprovalDecision::AllowSession => {
                self.session_allow.insert(tool_name.to_string());
            }
            ApprovalDecision::AllowAlways => {
                // This will be handled by the caller to persist to config
            }
            ApprovalDecision::AllowOnce | ApprovalDecision::Cancel => {
                // No state change needed
            }
        }
    }

    /// Reset session state
    pub fn reset(&mut self) {
        self.session_allow.clear();
        self.category_session.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_prompt_read_group() {
        let state = ApprovalState::new();
        let config = ApprovalsConfig::default();

        // Read operations don't prompt by default
        assert!(!state.should_prompt("portfolio_show", PolicyGroup::Read, &config));
    }

    #[test]
    fn test_should_prompt_mutate_group() {
        let state = ApprovalState::new();
        let config = ApprovalsConfig::default();

        // Mutate operations prompt by default
        assert!(state.should_prompt("import_file", PolicyGroup::Mutate, &config));
    }

    #[test]
    fn test_default_policy_deny_blocks_mutate() {
        let state = ApprovalState::new();
        let config = ApprovalsConfig {
            default_policy: DefaultPolicy::Deny,
            ..Default::default()
        };

        // Deny does not prompt, but blocks execution
        assert!(!state.should_prompt("import_file", PolicyGroup::Mutate, &config));
        assert!(state.is_denied("import_file", PolicyGroup::Mutate, &config));
    }

    #[test]
    fn test_default_policy_deny_allows_explicit() {
        let state = ApprovalState::new();
        let mut config = ApprovalsConfig {
            default_policy: DefaultPolicy::Deny,
            ..Default::default()
        };
        config.always_allow.push("import_file".to_string());

        // Explicit allow overrides deny
        assert!(!state.should_prompt("import_file", PolicyGroup::Mutate, &config));
        assert!(!state.is_denied("import_file", PolicyGroup::Mutate, &config));
    }

    #[test]
    fn test_session_allow() {
        let mut state = ApprovalState::new();
        let config = ApprovalsConfig::default();

        // Initially prompts
        assert!(state.should_prompt("import_file", PolicyGroup::Mutate, &config));

        // Record session decision
        state.record_decision(
            "import_file",
            PolicyGroup::Mutate,
            ApprovalDecision::AllowSession,
        );

        // No longer prompts in this session
        assert!(!state.should_prompt("import_file", PolicyGroup::Mutate, &config));
    }

    #[test]
    fn test_always_allow_in_config() {
        let state = ApprovalState::new();
        let mut config = ApprovalsConfig::default();
        config.always_allow.push("import_file".to_string());

        // Should not prompt when in always_allow
        assert!(!state.should_prompt("import_file", PolicyGroup::Mutate, &config));
    }

    #[test]
    fn test_category_always_allow() {
        let state = ApprovalState::new();
        let mut config = ApprovalsConfig::default();
        config.always_allow.push("mutate".to_string());

        // Should not prompt when category is in always_allow
        assert!(!state.should_prompt("import_file", PolicyGroup::Mutate, &config));
        assert!(!state.should_prompt("transaction_add", PolicyGroup::Mutate, &config));
    }
}
