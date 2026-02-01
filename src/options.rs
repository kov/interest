//! Output and privacy settings shared across CLI/TUI dispatchers and renderers.

/// Output mode for command results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Human-readable terminal table (default)
    Table,
    /// JSON (machine-readable, preserves all precision)
    Json,
}

impl OutputMode {
    /// Create from legacy boolean flag.
    pub fn from_json_flag(json_output: bool) -> Self {
        if json_output {
            Self::Json
        } else {
            Self::Table
        }
    }
}

/// Privacy mode for rendered output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyMode {
    Full,
    Private,
}

/// Combined output options for command rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputOptions {
    pub output_mode: OutputMode,
    pub privacy: PrivacyMode,
}

impl OutputOptions {
    pub fn from_flags(json_output: bool, privacy: bool) -> Self {
        Self {
            output_mode: OutputMode::from_json_flag(json_output),
            privacy: if privacy {
                PrivacyMode::Private
            } else {
                PrivacyMode::Full
            },
        }
    }

    pub fn is_json(self) -> bool {
        self.output_mode == OutputMode::Json
    }

    pub fn is_private(self) -> bool {
        self.privacy == PrivacyMode::Private
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_mode_from_json_flag() {
        assert_eq!(OutputMode::from_json_flag(false), OutputMode::Table);
        assert_eq!(OutputMode::from_json_flag(true), OutputMode::Json);
    }

    #[test]
    fn test_output_options_from_flags() {
        let options = OutputOptions::from_flags(false, false);
        assert_eq!(options.output_mode, OutputMode::Table);
        assert_eq!(options.privacy, PrivacyMode::Full);

        let options = OutputOptions::from_flags(true, true);
        assert_eq!(options.output_mode, OutputMode::Json);
        assert_eq!(options.privacy, PrivacyMode::Private);
    }
}
