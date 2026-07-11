pub mod daemon;
pub mod identity;
pub mod ipc;
pub mod protocol;
pub mod redact;
pub mod state;

// Backward-compatible alias used by existing command modules.
pub use ipc as local;
