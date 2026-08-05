//! Restore the terminal before reporting a panic.
//!
//! The panic hook runs on whatever thread panicked. A TUI can only be torn down safely from the
//! thread that owns it (the main thread), so the restore closure is stored in a thread-local and
//! only invoked when the panicking thread is the one that registered it. Background-thread panics
//! still get the default report without tearing down a live interface.

use std::cell::RefCell;
use std::panic::AssertUnwindSafe;
use std::sync::Once;

thread_local! {
    /// Closure that restores the terminal, registered by the TUI on the thread that owns it.
    static TERMINAL_RESTORE: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

static INSTALL: Once = Once::new();

/// Register the closure the panic hook runs to restore the terminal.
///
/// The closure is stored on the calling thread and only runs when that thread panics, so a panic
/// on a background thread cannot tear down a TUI that is still drawing on the main thread.
pub fn register_restore(restore: Box<dyn FnOnce()>) {
    TERMINAL_RESTORE.with(|slot| slot.replace(Some(restore)));
}

/// Install a panic hook that restores the terminal before reporting the panic.
///
/// The previously installed hook is kept and invoked afterward, so panic output formatting and
/// backtrace hints are unchanged. Installing more than once is a no-op.
pub fn install() {
    INSTALL.call_once(|| {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let restore = TERMINAL_RESTORE
                .with(|slot| slot.try_borrow_mut().ok().and_then(|mut slot| slot.take()));
            if let Some(restore) = restore {
                // A restore that panics (e.g. I/O against a dying console) must not abort the
                // process before the panic is reported.
                let _ = std::panic::catch_unwind(AssertUnwindSafe(restore));
            }
            default_hook(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::{install, register_restore};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    #[test]
    fn restore_closure_runs_on_same_thread_panic() {
        install();
        let restored = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&restored);
        register_restore(Box::new(move || {
            flag.store(true, Ordering::Relaxed);
        }));

        let result = std::panic::catch_unwind(|| panic!("deliberate panic"));
        assert!(result.is_err());
        assert!(restored.load(Ordering::Relaxed));
    }
}
