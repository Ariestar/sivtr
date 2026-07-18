//! Command modules grouped by product domain.
//!
//! ```text
//! terminal/  write terminal memory (init/flush/clear + run/pipe/import ingest)
//! memory/    read/search/filter/show via workset
//! browse/    workspace TUI product surface
//! copy/      export to clipboard
//! diff       compare two terminal dialogues
//! select     relative dialogue selection (1 / A..B)
//! remote/    share/mount/serve CLI
//! system/    doctor/hotkey/mcp/…
//! ```

pub mod browse;
pub mod copy;
pub mod diff;
pub mod interactive;
pub mod memory;
pub mod remote;
pub mod select;
pub mod system;
pub mod terminal;
