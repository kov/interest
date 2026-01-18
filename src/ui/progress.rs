use crate::ui::crossterm_engine::Spinner;
use colored::Colorize;
use std::io::{self, IsTerminal, Write};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Progress tracking data for operations with countable steps
#[derive(Debug, Clone)]
pub struct ProgressData {
    pub current: usize,
    pub total: Option<usize>,
}

impl ProgressData {
    /// Format progress data as string: "(N/M)" or "(N)" if total unknown
    fn format(&self) -> String {
        if let Some(total) = self.total {
            let percentage = if total > 0 {
                (self.current * 100) / total
            } else {
                0
            };
            format!("({}/{} {}%)", self.current, total, percentage)
        } else {
            format!("({})", self.current)
        }
    }
}

/// Semantic progress events for UI rendering
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// Operation completed successfully - persisted with green ✓
    Success { message: String },

    /// Operation failed - persisted with red ✗
    Error { message: String },

    /// Informational message - persisted with blue ℹ
    Info { message: String },

    /// Downloading resource - transient spinner with 📥
    Downloading { resource: String },

    /// Decompressing file - transient spinner with 📦
    Decompressing { file: String },

    /// Parsing file - transient spinner with 📝
    Parsing {
        file: String,
        progress: Option<ProgressData>,
    },

    /// Recomputing/recalculating - transient spinner with ↻
    Recomputing {
        what: String,
        progress: Option<ProgressData>,
    },

    /// Individual ticker price fetch result - persisted
    TickerResult {
        ticker: String,
        price: Result<String, String>, // Ok(price) or Err(reason)
        current: usize,
        total: usize,
    },

    /// Generic transient spinner message (fallback for uncategorized updates)
    Spinner { message: String },
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

    pub fn handle_event(&self, event: &ProgressEvent) {
        if !self.enabled {
            return;
        }

        match event {
            ProgressEvent::Success { message } => {
                // Don't finish operation - just persist the message
                // The spinner keeps running until printer is dropped
                self.persist_line(&format!("{} {}", "✓".green(), message));
            }
            ProgressEvent::Error { message } => {
                // Don't finish operation - just persist the message
                // The spinner keeps running until printer is dropped
                self.persist_line(&format!("{} {}", "✗".red(), message));
            }
            ProgressEvent::Info { message } => {
                self.persist_line(&format!("{} {}", "ℹ".blue(), message));
            }
            ProgressEvent::Downloading { resource } => {
                self.update_spinner(&format!("📥 Downloading {}...", resource));
            }
            ProgressEvent::Decompressing { file } => {
                self.update_spinner(&format!("📦 Decompressing {}...", file));
            }
            ProgressEvent::Parsing { file, progress } => {
                let msg = if let Some(p) = progress {
                    format!("📝 Parsing {} {}", file, p.format())
                } else {
                    format!("📝 Parsing {}...", file)
                };
                self.update_spinner(&msg);
            }
            ProgressEvent::Recomputing { what, progress } => {
                let msg = if let Some(p) = progress {
                    format!("↻ Recomputing {} {}", what, p.format())
                } else {
                    format!("↻ Recomputing {}...", what)
                };
                self.update_spinner(&msg);
            }
            ProgressEvent::TickerResult {
                ticker,
                price,
                current,
                total,
            } => {
                let result_str = match price {
                    Ok(p) => format!("{} → {} ({}/{})", ticker, p, current, total),
                    Err(reason) => format!("{} → {} ({}/{})", ticker, reason, current, total),
                };
                self.persist_line(&result_str);
            }
            ProgressEvent::Spinner { message } => {
                self.update_spinner(message);
            }
        }
    }

    /// Internal: Update spinner message (transient)
    fn update_spinner(&self, message: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.message = message.to_string();
            state.active = true;
        }
    }

    /// Internal: Persist a line to output (permanent)
    fn persist_line(&self, formatted_message: &str) {
        clear_line();
        println!("{}", formatted_message);
        let _ = io::stdout().flush();
    }
}

/// Clear any in-place spinner line.
pub fn clear_progress_line() {
    clear_line();
    let _ = io::stdout().flush();
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
    fn progress_data_formats_with_percentage() {
        let data = ProgressData {
            current: 25,
            total: Some(100),
        };
        assert_eq!(data.format(), "(25/100 25%)");
    }

    #[test]
    fn progress_data_formats_without_total() {
        let data = ProgressData {
            current: 42,
            total: None,
        };
        assert_eq!(data.format(), "(42)");
    }
}
