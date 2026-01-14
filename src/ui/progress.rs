use crate::ui::crossterm_engine::Spinner;
use colored::Colorize;
use std::io::{self, IsTerminal, Write};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

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

#[derive(Debug)]
struct SpinnerState {
    spinner: Spinner,
    message: String,
    active: bool,
    finished: bool,
}

impl SpinnerState {
    fn new() -> Self {
        Self {
            spinner: Spinner::new(),
            message: String::new(),
            active: false,
            finished: false,
        }
    }
}

/// Shared progress printer with optional background spinner ticks.
pub struct ProgressPrinter {
    enabled: bool,
    state: Arc<Mutex<SpinnerState>>,
    tick_handle: Option<JoinHandle<()>>,
}

impl ProgressPrinter {
    pub fn new(json_output: bool) -> Self {
        let enabled = !json_output
            && io::stdout().is_terminal()
            && std::env::var("INTEREST_NO_SPINNER").ok().as_deref() != Some("1");
        let state = Arc::new(Mutex::new(SpinnerState::new()));
        let tick_handle = if enabled && spinner_ticks_enabled() {
            Some(start_spinner_thread(state.clone()))
        } else {
            None
        };

        Self {
            enabled,
            state,
            tick_handle,
        }
    }

    #[allow(dead_code)]
    pub fn start(&self) {
        if !self.enabled {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.active = true;
        }
    }

    #[allow(dead_code)]
    pub fn stop(&self) {
        if !self.enabled {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.active = false;
        }
    }

    pub fn update(&self, message: &str) {
        if !self.enabled {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.message = message.to_string();
            state.active = true;
        }
    }

    pub fn persist(&self, message: &str) {
        if !self.enabled {
            return;
        }
        clear_line();
        let formatted = if message.starts_with("✓") {
            message.green().to_string()
        } else if message.starts_with("❌") || message.starts_with("✗") {
            message.red().to_string()
        } else if message.starts_with("ℹ") {
            message.blue().to_string()
        } else {
            message.to_string()
        };
        println!("{}", formatted);
        let _ = io::stdout().flush();
    }

    pub fn finish(&self, success: bool, message: &str) {
        if !self.enabled {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.finished = true;
            state.active = false;
        }
        let icon = if success {
            "✓".green().to_string()
        } else {
            "✗".red().to_string()
        };
        clear_line();
        println!("{} {}", icon, message);
        let _ = io::stdout().flush();
    }

    pub fn handle_event(&self, event: ProgressEvent) {
        match event {
            ProgressEvent::Line { text, persist } => {
                if persist {
                    self.persist(&text);
                } else {
                    self.update(&text);
                }
            }
        }
    }
}

impl Drop for ProgressPrinter {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.finished = true;
            state.active = false;
        }
        if let Some(handle) = self.tick_handle.take() {
            let _ = handle.join();
        }
    }
}

fn spinner_ticks_enabled() -> bool {
    std::env::var("INTEREST_DISABLE_SPINNER_TICKER")
        .ok()
        .as_deref()
        != Some("1")
}

fn start_spinner_thread(state: Arc<Mutex<SpinnerState>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let delay = Duration::from_millis(100);
        loop {
            {
                let guard = match state.lock() {
                    Ok(guard) => guard,
                    Err(_) => return,
                };
                if guard.finished {
                    break;
                }
                if guard.active {
                    let frame = guard.spinner.tick().to_string();
                    let msg = guard.message.clone();
                    drop(guard);
                    render_spinner_line(&frame, &msg);
                }
            }
            std::thread::sleep(delay);
        }
    })
}

fn render_spinner_line(frame: &str, message: &str) {
    print!("\r\x1B[2K{} {}", frame, message);
    let _ = io::stdout().flush();
}

fn clear_line() {
    print!("\r\x1B[2K");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_event_parses_persist_prefix() {
        let event = ProgressEvent::from_message("__PERSIST__:hello");
        match event {
            ProgressEvent::Line { text, persist } => {
                assert_eq!(text, "hello");
                assert!(persist);
            }
        }
    }
}
