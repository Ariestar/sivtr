//! Restore the terminal before reporting a panic.
//!
//! The panic hook runs on whatever thread panicked. A TUI can only be torn down safely from the
//! thread that owns it (the main thread), so the restore closure is stored in a thread-local and
//! only invoked when the panicking thread is the one that registered it. Background-thread panics
//! still get the default report without tearing down a live interface.

use std::cell::{Cell, RefCell};
use std::panic::AssertUnwindSafe;
use std::sync::Once;

thread_local! {
    /// Stack of panic-time restore closures, innermost (most recent) session
    /// last. A session's entry is cleared when that session restores or drops,
    /// so a later panic in a still-live outer session pops the outer closure.
    static TERMINAL_RESTORE: RestoreSlots = const { RefCell::new(Vec::new()) };
    /// Set while a guard intends to catch and report a TUI panic itself; the
    /// hook then restores the terminal without invoking the default reporter,
    /// so the user does not see an uncaught-panic report followed by the
    /// guard's own message.
    static SUPPRESS_DEFAULT_REPORT: Cell<bool> = const { Cell::new(false) };
}

/// Registered restore closures; see [`TERMINAL_RESTORE`].
type RestoreSlots = RefCell<Vec<Option<Box<dyn FnOnce()>>>>;

static INSTALL: Once = Once::new();

/// A session's panic-restore registration. Dropping it (TUI finish/drop)
/// clears this session's slot without shifting later indices; the panic hook
/// takes the innermost live closure.
pub struct RestoreRegistration {
    index: usize,
}

/// Scope guard that suppresses the default panic report for a recovered TUI
/// panic on the thread that owns the terminal.
///
/// The terminal-restoring hook still runs; only the default reporter is
/// skipped, because the caller reports the recovered panic itself. Background
/// thread panics are unaffected — they never own a restore closure, so their
/// default report is kept.
pub struct SuppressDefaultReport;

impl SuppressDefaultReport {
    pub fn enter() -> Self {
        SUPPRESS_DEFAULT_REPORT.with(|flag| flag.set(true));
        SuppressDefaultReport
    }
}

impl Drop for SuppressDefaultReport {
    fn drop(&mut self) {
        SUPPRESS_DEFAULT_REPORT.with(|flag| flag.set(false));
    }
}

/// Register the closure the panic hook runs to restore the terminal.
///
/// The closure is stored on the calling thread and only runs when that thread panics, so a panic
/// on a background thread cannot tear down a TUI that is still drawing on the main thread. Nested
/// TUI sessions stack on their owning thread; drop the returned registration when the session
/// ends so a later panic in an outer session still restores the outer state.
pub fn register_restore(restore: Box<dyn FnOnce()>) -> RestoreRegistration {
    let index = TERMINAL_RESTORE.with(|slot| {
        let mut slot = slot.borrow_mut();
        slot.push(Some(restore));
        slot.len() - 1
    });
    RestoreRegistration { index }
}

impl Drop for RestoreRegistration {
    fn drop(&mut self) {
        // Teardown-safe: the thread-local may be destroyed when this runs at
        // process exit; a panic here would abort the process.
        TERMINAL_RESTORE
            .try_with(|slot| {
                if let Some(entry) = slot.borrow_mut().get_mut(self.index) {
                    *entry = None;
                }
            })
            .ok();
    }
}

/// Install a panic hook that restores the terminal before reporting the panic.
///
/// The previously installed hook is kept and invoked afterward, so panic output formatting and
/// backtrace hints are unchanged — unless a [`SuppressDefaultReport`] guard is active and the
/// panic is on the terminal-owning thread, in which case the guard reports it. Installing more
/// than once is a no-op.
pub fn install() {
    INSTALL.call_once(|| {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Teardown-safe: the thread-local may already be destroyed when
            // the hook runs at process exit; a panic here would abort the
            // report of the original panic.
            let restore = TERMINAL_RESTORE
                .try_with(|slot| {
                    slot.try_borrow_mut()
                        .ok()
                        .and_then(|mut slot| slot.iter_mut().rev().find_map(|entry| entry.take()))
                })
                .ok()
                .flatten();
            // Only the terminal-owning thread holds a restore closure, so a
            // restored panic is exactly the one a guard may be recovering.
            let suppressed = restore.is_some() && SUPPRESS_DEFAULT_REPORT.with(Cell::get);
            if let Some(restore) = restore {
                // A restore that panics (e.g. I/O against a dying console) must not abort the
                // process before the panic is reported.
                let _ = std::panic::catch_unwind(AssertUnwindSafe(restore));
            }
            if !suppressed {
                default_hook(info);
            }
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::{install, register_restore, SuppressDefaultReport};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    #[test]
    fn restore_closure_runs_on_same_thread_panic() {
        install();
        let restored = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&restored);
        let _registration = register_restore(Box::new(move || {
            flag.store(true, Ordering::Relaxed);
        }));

        let result = std::panic::catch_unwind(|| panic!("deliberate panic"));
        assert!(result.is_err());
        assert!(restored.load(Ordering::Relaxed));
    }

    #[test]
    fn ended_session_clears_its_restore_and_keeps_the_outer_one() {
        install();
        let inner = Arc::new(AtomicBool::new(false));
        let outer = Arc::new(AtomicBool::new(false));
        // An inner session registers and ends normally: its slot must clear,
        // so a later panic in the still-live outer session pops the outer
        // closure instead of the stale inner one.
        let inner_flag = Arc::clone(&inner);
        let registration = register_restore(Box::new(move || {
            inner_flag.store(true, Ordering::Relaxed);
        }));
        let outer_flag = Arc::clone(&outer);
        let _outer_registration = register_restore(Box::new(move || {
            outer_flag.store(true, Ordering::Relaxed);
        }));
        drop(registration);

        let result = std::panic::catch_unwind(|| panic!("deliberate panic"));
        assert!(result.is_err());
        assert!(
            !inner.load(Ordering::Relaxed),
            "inner restore must be cleared"
        );
        assert!(
            outer.load(Ordering::Relaxed),
            "outer restore must still run"
        );
    }

    #[test]
    fn suppressed_default_report_still_restores_the_terminal() {
        install();
        let restored = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&restored);
        let _registration = register_restore(Box::new(move || {
            flag.store(true, Ordering::Relaxed);
        }));

        let _guard = SuppressDefaultReport::enter();
        let result = std::panic::catch_unwind(|| panic!("recovered panic"));
        assert!(result.is_err());
        assert!(restored.load(Ordering::Relaxed));
    }
}
