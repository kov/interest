//! Lightweight rendering helpers for the TUI.
//! These functions return strings so they are easy to test without a terminal.

use colored::Colorize;
use std::cell::Cell;
use tabled::{settings::Style, Table, Tabled};

/// Render a vector of `Tabled` items into a modern-styled table string.
#[allow(dead_code)] // Kept for Phase 3+ TUI implementation
pub fn draw_table<T: Tabled>(data: &[T]) -> String {
    let mut table = Table::new(data);
    table.with(Style::modern());
    table.to_string()
}

/// Render a simple message (placeholder for richer layouts later).
#[allow(dead_code)] // Kept for Phase 3+ TUI implementation
pub fn draw_message(msg: &str) -> String {
    msg.to_string()
}

/// Simple textual menu with a selected index highlighted.
#[allow(dead_code)] // Kept for Phase 3+ TUI implementation
pub fn draw_menu(title: &str, items: &[&str], selected: usize) -> String {
    let mut out = String::new();
    out.push_str(&format!("{}\n", title.bold()));
    for (idx, item) in items.iter().enumerate() {
        if idx == selected {
            out.push_str(&format!("> {}\n", item));
        } else {
            out.push_str(&format!("  {}\n", item));
        }
    }
    out
}

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
    use tabled::Tabled;

    #[derive(Tabled)]
    struct Row {
        name: String,
    }

    #[test]
    fn test_draw_table_formats_modern() {
        let rows = vec![Row {
            name: "alpha".to_string(),
        }];
        let rendered = draw_table(&rows);
        assert!(rendered.contains("name"));
        assert!(rendered.contains("alpha"));
    }

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

    #[test]
    fn test_draw_menu_marks_selected() {
        let rendered = draw_menu("Menu", &["One", "Two"], 1);
        assert!(rendered.contains("> Two"));
        assert!(rendered.contains("  One"));
    }
}
