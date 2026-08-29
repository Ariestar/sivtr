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

use anyhow::{bail, Result};

pub mod browse;
pub mod interactive;
pub mod memory;
pub mod publish;
pub mod remote;
pub mod select;
pub mod system;
pub mod terminal;

pub(crate) fn finish_picker(
    result: browse::PickerResult,
    print_full: bool,
    regex: Option<&str>,
    lines: Option<&str>,
    ansi: bool,
) -> Result<()> {
    match result {
        browse::PickerResult::Picked(picked) => {
            memory::copy::export_picked(&picked, print_full, regex, lines, ansi)
        }
        browse::PickerResult::Publish {
            set,
            draft,
            expires,
            save_name,
        } => {
            if regex.is_some() || lines.is_some() {
                bail!("cannot publish a picker selection with copy filters; remove --regex and --lines");
            }
            publish::create_from_picker(set, draft, expires, save_name)
        }
    }
}
