//! Host callbacks: engine events delivered to C through the host's dispatcher.
//!
//! The headers are strict about how this works, and the rules exist for
//! reasons that show up as crashes when ignored:
//!
//! * a non-null callback requires a non-null dispatcher — engine events arrive
//!   on the render or host thread, and delivering them there would run host
//!   code on a thread it never agreed to;
//! * callbacks install once, before the first attach, so a queued task can
//!   never observe a replaced function pointer or `user_data`;
//! * a task handed to the dispatcher must run exactly once, and it owns its
//!   payload until it does.

use std::{
    ffi::{c_void, CString},
    os::raw::c_char,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use crate::abi::{MigoResult, VersionedHeader, MIGO_ERROR_INVALID_ARGUMENT, MIGO_OK};

pub type MigoTaskFn = unsafe extern "C" fn(*mut c_void);
pub type MigoDispatchFn = unsafe extern "C" fn(*mut c_void, MigoTaskFn, *mut c_void) -> MigoResult;
pub type MigoOnReadyFn = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub type MigoOnErrorFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *const MigoError);
pub type MigoOnExitRequestedFn = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub type MigoOnSurfaceLostFn = unsafe extern "C" fn(*mut c_void, *mut c_void, u64, u32);

/// Mirrors `MigoError` in `include/migo/types.h`.
#[repr(C)]
pub struct MigoError {
    pub header: VersionedHeader,
    pub code: MigoResult,
    pub flags: u32,
    pub message_utf8: *const c_char,
    pub message_length: u32,
    pub reserved0: u32,
}

/// Mirrors `MigoHostCallbacks` in `include/migo/session.h`.
#[repr(C)]
pub struct MigoHostCallbacks {
    pub header: VersionedHeader,
    pub user_data: *mut c_void,
    pub dispatcher_data: *mut c_void,
    pub dispatch: Option<MigoDispatchFn>,
    pub on_ready: Option<MigoOnReadyFn>,
    pub on_error: Option<MigoOnErrorFn>,
    pub on_exit_requested: Option<MigoOnExitRequestedFn>,
    pub on_surface_lost: Option<MigoOnSurfaceLostFn>,
}

/// The copy the session keeps.
///
/// Only the fields above are retained — never the caller's struct, which the
/// ABI borrows for the duration of the call only.
#[derive(Clone, Copy)]
pub struct HostCallbacks {
    user_data: *mut c_void,
    dispatcher_data: *mut c_void,
    dispatch: MigoDispatchFn,
    on_ready: Option<MigoOnReadyFn>,
    on_error: Option<MigoOnErrorFn>,
    on_exit_requested: Option<MigoOnExitRequestedFn>,
    on_surface_lost: Option<MigoOnSurfaceLostFn>,
}

// SAFETY: the pointers are opaque tokens owned by the host and are only ever
// handed back to host-provided function pointers. The engine never
// dereferences them, and the ABI requires the host to keep them valid for the
// session's lifetime, so moving them between engine threads adds no aliasing.
unsafe impl Send for HostCallbacks {}
unsafe impl Sync for HostCallbacks {}

impl HostCallbacks {
    /// Copy and validate a caller-supplied callback set.
    ///
    /// # Safety
    /// `callbacks` must point to a validated [`MigoHostCallbacks`].
    pub unsafe fn from_c(callbacks: &MigoHostCallbacks) -> Result<Self, MigoResult> {
        let has_callback = callbacks.on_ready.is_some()
            || callbacks.on_error.is_some()
            || callbacks.on_exit_requested.is_some()
            || callbacks.on_surface_lost.is_some();
        let Some(dispatch) = callbacks.dispatch else {
            // Without a dispatcher there is nowhere safe to run host code.
            return if has_callback {
                Err(MIGO_ERROR_INVALID_ARGUMENT)
            } else {
                Err(MIGO_ERROR_INVALID_ARGUMENT)
            };
        };
        Ok(Self {
            user_data: callbacks.user_data,
            dispatcher_data: callbacks.dispatcher_data,
            dispatch,
            on_ready: callbacks.on_ready,
            on_error: callbacks.on_error,
            on_exit_requested: callbacks.on_exit_requested,
            on_surface_lost: callbacks.on_surface_lost,
        })
    }
}

/// What a queued task should invoke once the dispatcher runs it.
enum Event {
    Ready,
    ExitRequested,
    Error { code: MigoResult, message: CString },
    SurfaceLost { generation: u64, reason: u32 },
}

/// Payload owned by a dispatched task until it runs.
struct Task {
    callbacks: HostCallbacks,
    session: *mut c_void,
    alive: Arc<AtomicBool>,
    event: Event,
}

/// Invoked by the host's dispatcher. Consumes the payload exactly once.
///
/// # Safety
/// `context` must be the pointer produced by [`Notifier::post`].
unsafe extern "C" fn run_task(context: *mut c_void) {
    if context.is_null() {
        return;
    }
    let task = unsafe { Box::from_raw(context as *mut Task) };
    // The session may have been destroyed between queueing and running; the
    // header promises queued callbacks are cancelled, and this is where that
    // promise is kept.
    if !task.alive.load(Ordering::Acquire) {
        return;
    }
    let user_data = task.callbacks.user_data;
    match &task.event {
        Event::Ready => {
            if let Some(on_ready) = task.callbacks.on_ready {
                unsafe { on_ready(user_data, task.session) };
            }
        }
        Event::ExitRequested => {
            if let Some(on_exit) = task.callbacks.on_exit_requested {
                unsafe { on_exit(user_data, task.session) };
            }
        }
        Event::Error { code, message } => {
            if let Some(on_error) = task.callbacks.on_error {
                let bytes = message.as_bytes();
                let error = MigoError {
                    header: VersionedHeader {
                        struct_size: size_of::<MigoError>() as u32,
                        abi_version: crate::abi::MIGO_ABI_VERSION_CURRENT,
                    },
                    code: *code,
                    flags: 0,
                    message_utf8: message.as_ptr(),
                    message_length: bytes.len() as u32,
                    reserved0: 0,
                };
                // `message` outlives the call because `task` is dropped after.
                unsafe { on_error(user_data, task.session, &error) };
            }
        }
        Event::SurfaceLost { generation, reason } => {
            if let Some(on_surface_lost) = task.callbacks.on_surface_lost {
                unsafe { on_surface_lost(user_data, task.session, *generation, *reason) };
            }
        }
    }
}

/// Routes engine notifications to the host's callbacks.
pub struct Notifier {
    callbacks: HostCallbacks,
    session: NonNull<c_void>,
    alive: Arc<AtomicBool>,
}

// SAFETY: as for `HostCallbacks` — the session pointer is an opaque token
// handed back to the host, never dereferenced by the engine.
unsafe impl Send for Notifier {}
unsafe impl Sync for Notifier {}

impl Notifier {
    pub fn new(callbacks: HostCallbacks, session: NonNull<c_void>, alive: Arc<AtomicBool>) -> Self {
        Self {
            callbacks,
            session,
            alive,
        }
    }

    /// Hand one event to the host's dispatcher.
    ///
    /// If the dispatcher refuses the task, ownership stays here and the payload
    /// is dropped — the header's rule that a rejected dispatch leaves the task
    /// with Migo.
    fn post(&self, event: Event) {
        if !self.alive.load(Ordering::Acquire) {
            return;
        }
        let task = Box::new(Task {
            callbacks: self.callbacks,
            session: self.session.as_ptr(),
            alive: Arc::clone(&self.alive),
            event,
        });
        let context = Box::into_raw(task) as *mut c_void;
        // SAFETY: `dispatch` came from the host and is called with the token it
        // supplied plus a task it must run exactly once.
        let result = unsafe { (self.callbacks.dispatch)(self.callbacks.dispatcher_data, run_task, context) };
        if result != MIGO_OK {
            // Rejected: take the payload back and drop it, so the task neither
            // leaks nor runs.
            drop(unsafe { Box::from_raw(context as *mut Task) });
            tracing::warn!("host dispatcher rejected a callback task: {result}");
        }
    }

    pub fn ready(&self) {
        self.post(Event::Ready);
    }

    pub fn exit_requested(&self) {
        self.post(Event::ExitRequested);
    }

    pub fn error(&self, code: MigoResult, message: impl Into<Vec<u8>>) {
        // Interior NULs cannot travel through a C string; replace rather than
        // drop the report, since an error is exactly when detail matters.
        let message = CString::new(message).unwrap_or_else(|_| {
            CString::new("engine error (message contained an interior NUL)").expect("static")
        });
        self.post(Event::Error { code, message });
    }

    #[allow(dead_code)]
    pub fn surface_lost(&self, generation: u64, reason: u32) {
        self.post(Event::SurfaceLost { generation, reason });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    static READY_CALLS: AtomicUsize = AtomicUsize::new(0);
    static DISPATCH_CALLS: AtomicUsize = AtomicUsize::new(0);
    static LAST_ERROR_CODE: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn inline_dispatch(
        _dispatcher: *mut c_void,
        task: MigoTaskFn,
        context: *mut c_void,
    ) -> MigoResult {
        DISPATCH_CALLS.fetch_add(1, Ordering::SeqCst);
        unsafe { task(context) };
        MIGO_OK
    }

    unsafe extern "C" fn rejecting_dispatch(
        _dispatcher: *mut c_void,
        _task: MigoTaskFn,
        _context: *mut c_void,
    ) -> MigoResult {
        DISPATCH_CALLS.fetch_add(1, Ordering::SeqCst);
        MIGO_ERROR_INVALID_ARGUMENT
    }

    unsafe extern "C" fn on_ready(_user: *mut c_void, _session: *mut c_void) {
        READY_CALLS.fetch_add(1, Ordering::SeqCst);
    }

    unsafe extern "C" fn on_error(_user: *mut c_void, _session: *mut c_void, error: *const MigoError) {
        let error = unsafe { &*error };
        LAST_ERROR_CODE.store((-error.code) as usize, Ordering::SeqCst);
        // The message must be readable for the duration of this callback.
        assert!(!error.message_utf8.is_null());
        let text = unsafe { std::ffi::CStr::from_ptr(error.message_utf8) };
        assert!(!text.to_bytes().is_empty());
    }

    fn callbacks(dispatch: MigoDispatchFn) -> HostCallbacks {
        HostCallbacks {
            user_data: std::ptr::null_mut(),
            dispatcher_data: std::ptr::null_mut(),
            dispatch,
            on_ready: Some(on_ready),
            on_error: Some(on_error),
            on_exit_requested: None,
            on_surface_lost: None,
        }
    }

    fn notifier(dispatch: MigoDispatchFn, alive: Arc<AtomicBool>) -> Notifier {
        Notifier::new(
            callbacks(dispatch),
            NonNull::new(0x1000usize as *mut c_void).expect("non-null session token"),
            alive,
        )
    }

    #[test]
    fn events_reach_the_host_through_its_dispatcher() {
        // The engine must never call host code directly: the dispatcher is what
        // moves it to a thread the host chose.
        READY_CALLS.store(0, Ordering::SeqCst);
        DISPATCH_CALLS.store(0, Ordering::SeqCst);
        let notifier = notifier(inline_dispatch, Arc::new(AtomicBool::new(true)));
        notifier.ready();
        assert_eq!(DISPATCH_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(READY_CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_destroyed_session_cancels_queued_callbacks() {
        // Header rule: destroy cancels queued callbacks. Without this the host
        // would be handed a session pointer it has already released.
        READY_CALLS.store(0, Ordering::SeqCst);
        let alive = Arc::new(AtomicBool::new(true));
        let notifier = notifier(inline_dispatch, Arc::clone(&alive));
        alive.store(false, Ordering::Release);
        notifier.ready();
        assert_eq!(
            READY_CALLS.load(Ordering::SeqCst),
            0,
            "no callback may run after the session is gone"
        );
    }

    #[test]
    fn a_rejected_dispatch_drops_the_task_instead_of_leaking_or_running() {
        READY_CALLS.store(0, Ordering::SeqCst);
        DISPATCH_CALLS.store(0, Ordering::SeqCst);
        let notifier = notifier(rejecting_dispatch, Arc::new(AtomicBool::new(true)));
        notifier.ready();
        assert_eq!(DISPATCH_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(READY_CALLS.load(Ordering::SeqCst), 0);
        // Miri/leak checkers would catch a leak here; the point of the test is
        // that ownership returned to us on rejection.
    }

    #[test]
    fn error_messages_are_delivered_as_readable_c_strings() {
        LAST_ERROR_CODE.store(0, Ordering::SeqCst);
        let notifier = notifier(inline_dispatch, Arc::new(AtomicBool::new(true)));
        notifier.error(MIGO_ERROR_INVALID_ARGUMENT, "content failed to load");
        assert_eq!(LAST_ERROR_CODE.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn interior_nul_messages_still_report_the_error() {
        // Losing the report entirely would be worse than losing the detail.
        LAST_ERROR_CODE.store(0, Ordering::SeqCst);
        let notifier = notifier(inline_dispatch, Arc::new(AtomicBool::new(true)));
        notifier.error(MIGO_ERROR_INVALID_ARGUMENT, "bad\0message");
        assert_eq!(LAST_ERROR_CODE.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn callbacks_without_a_dispatcher_are_refused() {
        let raw = MigoHostCallbacks {
            header: VersionedHeader {
                struct_size: size_of::<MigoHostCallbacks>() as u32,
                abi_version: crate::abi::MIGO_ABI_VERSION_CURRENT,
            },
            user_data: std::ptr::null_mut(),
            dispatcher_data: std::ptr::null_mut(),
            dispatch: None,
            on_ready: Some(on_ready),
            on_error: None,
            on_exit_requested: None,
            on_surface_lost: None,
        };
        assert_eq!(
            unsafe { HostCallbacks::from_c(&raw) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }
}
