//! Status codes and the last error message, kept per thread so two hosts
//! on two threads never read each other's.

use std::cell::RefCell;
use std::ffi::{CString, c_char};

/// What a call reports. Zero is success, as C expects.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok = 0,
    /// A required pointer was null.
    Null = 1,
    /// The bytes were not a model this library reads.
    BadModel = 2,
    /// An index pointed past the model.
    Range = 3,
    /// An output array was shorter than what it has to hold.
    TooSmall = 4,
    /// Something inside went wrong; `ay2d_last_error` says what.
    Internal = 5,
    /// Another thread is in a call that drives this model (an update, a
    /// motion or expression call, a physics setting). Nothing changed; try
    /// again from the thread that owns the frame.
    Busy = 6,
}

thread_local! {
    static LAST: RefCell<CString> = RefCell::new(CString::default());
}

/// Remember why the last call failed, for `ay2d_last_error`.
pub(crate) fn set_error(msg: impl Into<String>) {
    let msg: String = msg.into();
    // a NUL inside the message would end it early; drop any
    let clean: String = msg.chars().filter(|c| *c != '\0').collect();
    LAST.with(|l| {
        if let Ok(c) = CString::new(clean) {
            *l.borrow_mut() = c;
        }
    });
}

/// Run an entry point so a panic inside becomes a status, never an abort
/// across the boundary.
pub(crate) fn guard<R>(on_panic: R, f: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(e) => {
            let what = e
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| e.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".to_string());
            set_error(format!("internal: {what}"));
            on_panic
        }
    }
}

/// Why the last call on this thread failed, NUL-terminated; empty when it
/// did not. The pointer is good until the next failing call on the thread.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_last_error() -> *const c_char {
    LAST.with(|l| l.borrow().as_ptr())
}
