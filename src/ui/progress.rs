/// Typed progress events for UI rendering and persistence
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// A single line of progress; `persist=true` means the message should be
    /// printed as a permanent line (newline), otherwise it's transient (spinner line).
    Line { text: String, persist: bool },
}

impl ProgressEvent {
    /// Create a Line event parsing the legacy string convention (__PERSIST__:prefix)
    pub fn from_message(msg: &str) -> Self {
        if let Some(content) = msg.strip_prefix("__PERSIST__:") {
            ProgressEvent::Line {
                text: content.to_string(),
                persist: true,
            }
        } else {
            ProgressEvent::Line {
                text: msg.to_string(),
                persist: false,
            }
        }
    }
}
