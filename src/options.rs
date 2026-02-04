//! Output and privacy settings shared across CLI/TUI dispatchers and renderers.

use std::io::Write;

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

/// Output destination for rendered text.
#[derive(Debug, Clone)]
pub enum OutputTarget {
    Stdout,
    Stderr,
    Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>),
}

impl OutputTarget {
    pub(crate) fn write_all(&self, content: &[u8]) -> std::io::Result<()> {
        match self {
            OutputTarget::Stdout => {
                let mut out = std::io::stdout();
                out.write_all(content)?;
                out.flush()
            }
            OutputTarget::Stderr => {
                let mut out = std::io::stderr();
                out.write_all(content)?;
                out.flush()
            }
            OutputTarget::Capture(buffer) => {
                let mut guard = buffer
                    .lock()
                    .map_err(|_| std::io::Error::other("lock poisoned"))?;
                guard.extend_from_slice(content);
                Ok(())
            }
        }
    }
}

/// Combined output options for command rendering.
#[derive(Debug, Clone)]
pub struct OutputOptions {
    pub output_mode: OutputMode,
    pub privacy: PrivacyMode,
    pub color_enabled: bool,
    pub output: OutputTarget,
    pub error_output: OutputTarget,
    pub secondary_output_mode: Option<OutputMode>,
    pub secondary_output: Option<OutputTarget>,
}

pub struct OutputWriter<'a> {
    target: &'a OutputTarget,
}

impl<'a> OutputWriter<'a> {
    pub fn write(&self, content: &str) -> std::io::Result<()> {
        self.target.write_all(content.as_bytes())
    }

    pub fn writeln(&self, content: &str) -> std::io::Result<()> {
        self.target.write_all(content.as_bytes())?;
        self.target.write_all(b"\n")
    }
}

impl OutputOptions {
    pub fn new(output_mode: OutputMode, privacy: PrivacyMode) -> Self {
        Self {
            output_mode,
            privacy,
            color_enabled: true,
            output: OutputTarget::Stdout,
            error_output: OutputTarget::Stderr,
            secondary_output_mode: None,
            secondary_output: None,
        }
    }

    pub fn from_flags(json_output: bool, privacy: bool) -> Self {
        Self::new(
            OutputMode::from_json_flag(json_output),
            if privacy {
                PrivacyMode::Private
            } else {
                PrivacyMode::Full
            },
        )
    }

    pub fn with_capture(mut self, buffer: std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> Self {
        self.output = OutputTarget::Capture(buffer.clone());
        self.error_output = OutputTarget::Capture(buffer);
        self
    }

    pub fn with_color_enabled(mut self, enabled: bool) -> Self {
        self.color_enabled = enabled;
        self
    }

    pub fn with_secondary_output(mut self, output_mode: OutputMode, target: OutputTarget) -> Self {
        self.secondary_output_mode = Some(output_mode);
        self.secondary_output = Some(target);
        self
    }

    pub fn is_json(&self) -> bool {
        self.output_mode == OutputMode::Json
    }

    pub fn is_private(&self) -> bool {
        self.privacy == PrivacyMode::Private
    }

    pub fn is_color_enabled(&self) -> bool {
        self.color_enabled
    }

    pub fn is_capturing(&self) -> bool {
        matches!(self.output, OutputTarget::Capture(_))
    }

    pub fn writer(&self) -> OutputWriter<'_> {
        OutputWriter {
            target: &self.output,
        }
    }

    pub fn error_writer(&self) -> OutputWriter<'_> {
        OutputWriter {
            target: &self.error_output,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

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
        assert!(options.color_enabled);

        let options = OutputOptions::from_flags(true, true);
        assert_eq!(options.output_mode, OutputMode::Json);
        assert_eq!(options.privacy, PrivacyMode::Private);
        assert!(options.color_enabled);
    }

    #[test]
    fn test_output_options_capture_writes_to_buffer() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let options = OutputOptions::from_flags(false, false).with_capture(buffer.clone());

        options.writer().write("hello").unwrap();
        options.writer().writeln("world").unwrap();
        options.error_writer().writeln("oops").unwrap();

        let captured = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        assert_eq!(captured, "helloworld\noops\n");
    }
}
