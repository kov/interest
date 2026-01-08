//! Minimal event loop wrapper around crossterm polling.

use std::time::Duration;

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Kept for Phase 3+ TUI implementation
pub enum AppEvent {
    Input(KeyEvent),
    Tick,
    Other,
}

#[allow(dead_code)] // Kept for Phase 3+ TUI implementation
pub fn map_event(ev: CrosstermEvent) -> AppEvent {
    match ev {
        CrosstermEvent::Key(key) => AppEvent::Input(key),
        _ => AppEvent::Other,
    }
}

/// Poll for a terminal event; returns Tick when the timeout expires.
#[allow(dead_code)] // Kept for Phase 3+ TUI implementation
pub fn poll_event(timeout: Duration) -> anyhow::Result<AppEvent> {
    if event::poll(timeout)? {
        let ev = event::read()?;
        Ok(map_event(ev))
    } else {
        Ok(AppEvent::Tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn map_key_event_to_input() {
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(map_event(CrosstermEvent::Key(key)), AppEvent::Input(key));
    }

    #[test]
    fn tick_when_no_event_ready() {
        let ev = poll_event(Duration::from_millis(0)).unwrap();
        // Polling with zero duration should immediately timeout.
        assert_eq!(ev, AppEvent::Tick);
    }
}
