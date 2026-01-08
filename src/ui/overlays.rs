//! Overlay helpers for the TUI.

use crate::ui::crossterm_engine;

/// A simple trait all overlays implement so they can render into strings.
#[allow(dead_code)] // Kept for Phase 3+ TUI implementation
pub trait Overlay {
    fn render(&self) -> String;
}

/// Menu overlay that highlights the selected item.
#[allow(dead_code)] // Kept for Phase 3+ TUI implementation
pub struct MenuOverlay<'a> {
    title: &'a str,
    items: Vec<&'a str>,
    selected: usize,
}

impl<'a> MenuOverlay<'a> {
    #[allow(dead_code)] // Kept for Phase 3+ TUI implementation
    pub fn new(title: &'a str, items: Vec<&'a str>) -> Self {
        Self {
            title,
            items,
            selected: 0,
        }
    }

    #[allow(dead_code)] // Kept for Phase 3+ TUI implementation
    pub fn select(&mut self, idx: usize) {
        if idx < self.items.len() {
            self.selected = idx;
        }
    }

    #[allow(dead_code)] // Kept for Phase 3+ TUI implementation
    pub fn selected(&self) -> Option<&str> {
        self.items.get(self.selected).copied()
    }
}

impl<'a> Overlay for MenuOverlay<'a> {
    fn render(&self) -> String {
        crossterm_engine::draw_menu(self.title, &self.items, self.selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_selected_item() {
        let mut overlay = MenuOverlay::new("Menu", vec!["one", "two"]);
        overlay.select(1);
        let rendered = overlay.render();
        assert!(rendered.contains("> two"));
        assert_eq!(overlay.selected(), Some("two"));
    }

    #[test]
    fn ignore_out_of_bounds_selection() {
        let mut overlay = MenuOverlay::new("Menu", vec!["one"]);
        overlay.select(5);
        assert_eq!(overlay.selected(), Some("one"));
    }
}
