//! Command modules grouped by product domain.
//!
//! ```text
//! terminal/  write terminal memory (init/flush/clear + run/pipe/import ingest)
//! memory/    read surface: workset + search/show/… + copy + terminal-only diff
//! browse/    workspace TUI product surface
//! select     relative dialogue selection (1 / A..B)
//! remote/    share/mount/serve CLI
//! system/    doctor/hotkey/mcp/…
//! ```

pub mod browse;
pub mod interactive;
pub mod memory;
pub mod remote;
pub mod select;
pub mod system;
pub mod terminal;
