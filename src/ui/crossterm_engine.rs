//! Lightweight rendering helpers for the TUI.

use std::cell::Cell;

/// A minimal spinner with braille frames.
#[derive(Debug, Clone)]
pub struct Spinner {
    frames: &'static [&'static str],
    index: Cell<usize>,
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Spinner {
    pub fn new() -> Self {
        Self {
            frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            index: Cell::new(0),
        }
    }

    pub fn tick(&self) -> &str {
        let idx = self.index.get();
        let frame = self.frames[idx];
        self.index.set((idx + 1) % self.frames.len());
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spinner_cycles_frames() {
        let spinner = Spinner::new();
        let first = spinner.tick().to_string();
        for _ in 0..spinner.frames.len() - 1 {
            spinner.tick();
        }
        let wrap = spinner.tick();
        assert_eq!(first, wrap);
    }
}
