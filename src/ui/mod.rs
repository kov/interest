//! TUI foundation module
//!
//! Provides the building blocks for the interactive terminal UI: rendering
//! helpers, readline wrapper, overlays, and a lightweight event loop skeleton.

pub mod crossterm_engine;
pub mod progress;

#[cfg(feature = "tui")]
mod chat;
#[cfg(feature = "tui")]
mod readline;
#[cfg(feature = "tui")]
mod tui;

#[cfg(feature = "tui")]
pub use chat::launch_chat;
#[cfg(feature = "tui")]
pub use tui::launch_tui;

#[cfg(not(feature = "tui"))]
use anyhow::Result;

#[cfg(not(feature = "tui"))]
pub async fn launch_tui(_output_options: crate::options::OutputOptions) -> Result<()> {
    Err(anyhow::anyhow!(
        "Interactive TUI is disabled; rebuild with --features tui"
    ))
}

#[cfg(not(feature = "tui"))]
pub async fn launch_chat(_output_options: crate::options::OutputOptions) -> Result<()> {
    Err(anyhow::anyhow!(
        "Chat TUI is disabled; rebuild with --features tui"
    ))
}
