//! Command modules grouped by product domain.
//! Prefer domain paths (`commands::memory::show`), but keep flat re-exports so
//! existing `commands::show` call sites stay valid.

pub mod capture;
pub mod memory;
pub mod remote;
pub mod system;

// --- memory ---
pub use memory::filter;
pub use memory::nav;
pub use memory::records;
pub use memory::search;
pub use memory::show;
pub use memory::time_filter;
pub use memory::var;
pub use memory::work;
pub use memory::work_json;
pub use memory::workset;
pub use memory::zoom;

// --- capture ---
pub use capture::clear;
pub use capture::command_block_selector;
pub use capture::copy;
pub use capture::diff;
pub use capture::flush;
pub use capture::import;
pub use capture::init;
pub use capture::pipe;
pub use capture::run;

// --- remote (domain module is `remote`; command execute is re-exported there) ---
pub use remote::peer;
pub use remote::serve;
pub use remote::share;
pub use remote::workspace;

// --- system ---
pub use system::codex;
pub use system::config;
pub use system::doctor;
pub use system::history;
pub use system::hotkey;
pub use system::migrate;
pub use system::version;
