pub mod agents;
pub mod ai;
pub mod buffer;
pub mod cache;
pub mod capture;
pub mod config;
pub mod export;
pub mod history;
pub mod origin;
pub mod parse;
pub mod query;
pub mod record;
pub mod search;
pub mod selection;
pub mod session;
pub mod time;
pub mod workspace;

pub use agents::claude;
pub use agents::codex;
pub use agents::cursor;
pub use agents::grok;
pub use agents::hermes;
pub use agents::openclaw;
pub use agents::opencode;
pub use agents::pi;

/// Serialize tests that mutate process-global env vars.
///
/// `std::env` is process-global, so any two tests that point e.g.
/// `SIVTR_DATA_DIR` (or a provider home) at different temp dirs race unless
/// they hold one shared lock. Every env-touching test module must use this.
#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
