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
    ffi::{CString, c_void},
    os::raw::c_char,
    ptr::NonNull,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::abi::{MIGO_ERROR_INVALID_ARGUMENT, MIGO_OK, MigoResult, VersionedHeader};

pub type MigoTaskFn = unsafe extern "C" fn(*mut c_void);
pub type MigoDispatchFn = unsafe extern "C" fn(*mut c_void, MigoTaskFn, *mut c_void) -> MigoResult;
pub type MigoOnReadyFn = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub type MigoOnErrorFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *const MigoError);
pub type MigoOnExitRequestedFn = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub type MigoOnSurfaceLostFn = unsafe extern "C" fn(*mut c_void, *mut c_void, u64, u32);
pub type MigoOnRequestFrameFn = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub type MigoOnShowKeyboardFn =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *const MigoKeyboardShowOptions);
pub type MigoOnHideKeyboardFn = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub type MigoOnUpdateKeyboardFn =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_char, u32);

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

/// `MIGO_KEYBOARD_FLAG_*` from `include/migo/session.h`.
pub const MIGO_KEYBOARD_FLAG_NONE: u32 = 0;
pub const MIGO_KEYBOARD_FLAG_MULTIPLE: u32 = 1 << 0;
pub const MIGO_KEYBOARD_FLAG_CONFIRM_HOLD: u32 = 1 << 1;

/// `MIGO_KEYBOARD_CONFIRM_*`, in wx's order.
pub const MIGO_KEYBOARD_CONFIRM_DONE: u32 = 0;
pub const MIGO_KEYBOARD_CONFIRM_NEXT: u32 = 1;
pub const MIGO_KEYBOARD_CONFIRM_SEARCH: u32 = 2;
pub const MIGO_KEYBOARD_CONFIRM_GO: u32 = 3;
pub const MIGO_KEYBOARD_CONFIRM_SEND: u32 = 4;

/// `MIGO_KEYBOARD_TYPE_*`.
pub const MIGO_KEYBOARD_TYPE_TEXT: u32 = 0;
pub const MIGO_KEYBOARD_TYPE_NUMBER: u32 = 1;

/// Mirrors `MigoKeyboardShowOptions` in `include/migo/session.h`.
///
/// Borrowed for the duration of the callback, like `MigoError`. A host that
/// needs the default value afterwards copies it.
#[repr(C)]
pub struct MigoKeyboardShowOptions {
    pub header: VersionedHeader,
    pub flags: u32,
    pub max_length: u32,
    pub confirm_type: u32,
    pub keyboard_type: u32,
    pub default_value_utf8: *const c_char,
    pub default_value_length: u32,
    pub reserved0: u32,
}

/// The engine-side owned form, which a queued task holds until it runs.
///
/// The C struct borrows a pointer; a task that outlives the call that created
/// it cannot, so the two shapes are deliberately different.
#[derive(Default, Clone)]
pub struct ShowOptions {
    pub flags: u32,
    pub max_length: u32,
    pub confirm_type: u32,
    pub keyboard_type: u32,
    pub default_value: String,
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
    pub on_request_frame: Option<MigoOnRequestFrameFn>,
    pub on_show_keyboard: Option<MigoOnShowKeyboardFn>,
    pub on_hide_keyboard: Option<MigoOnHideKeyboardFn>,
    pub on_update_keyboard: Option<MigoOnUpdateKeyboardFn>,
}

// SAFETY: every member is a `u32`, a raw pointer or an `Option<fn>`, all of
// which are valid when zeroed -- a zeroed callback is `None`, which is exactly
// what an older caller who never had the field meant.
//
// The minimum stops after `dispatch` because everything past it is an optional
// callback, and `session.h` has promised since the frame-pacing slice that a
// host omitting one simply does not get that behaviour. A host with a
// dispatcher and no callbacks at all is a legitimate, if quiet, host.
unsafe impl crate::abi::AbiStruct for MigoHostCallbacks {
    const MINIMUM_SIZE: usize = 32;
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
    on_request_frame: Option<MigoOnRequestFrameFn>,
    on_show_keyboard: Option<MigoOnShowKeyboardFn>,
    on_hide_keyboard: Option<MigoOnHideKeyboardFn>,
    on_update_keyboard: Option<MigoOnUpdateKeyboardFn>,
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
            || callbacks.on_surface_lost.is_some()
            || callbacks.on_request_frame.is_some();
        let Some(dispatch) = callbacks.dispatch else {
            // Without a dispatcher there is nowhere safe to run host code.
            return if has_callback {
                Err(MIGO_ERROR_INVALID_ARGUMENT)
            } else {
                Err(MIGO_ERROR_INVALID_ARGUMENT)
            };
        };
        // A host that can show a keyboard but not hide it strands it on screen
        // with no event that corrects the state, so partial support is refused
        // at install time -- while the host can still fix it.
        let keyboard_verbs = callbacks.on_show_keyboard.is_some() as u8
            + callbacks.on_hide_keyboard.is_some() as u8
            + callbacks.on_update_keyboard.is_some() as u8;
        if keyboard_verbs != 0 && keyboard_verbs != 3 {
            return Err(MIGO_ERROR_INVALID_ARGUMENT);
        }
        Ok(Self {
            user_data: callbacks.user_data,
            dispatcher_data: callbacks.dispatcher_data,
            dispatch,
            on_ready: callbacks.on_ready,
            on_error: callbacks.on_error,
            on_exit_requested: callbacks.on_exit_requested,
            on_surface_lost: callbacks.on_surface_lost,
            on_request_frame: callbacks.on_request_frame,
            on_show_keyboard: callbacks.on_show_keyboard,
            on_hide_keyboard: callbacks.on_hide_keyboard,
            on_update_keyboard: callbacks.on_update_keyboard,
        })
    }

    /// Whether the host offered to service the soft keyboard.
    ///
    /// All three verbs are present or none are, enforced above, so testing one
    /// is testing the set.
    pub fn supplies_keyboard(&self) -> bool {
        self.on_show_keyboard.is_some()
    }
}

/// What a queued task should invoke once the dispatcher runs it.
enum Event {
    Ready,
    ExitRequested,
    Error { code: MigoResult, message: CString },
    SurfaceLost { generation: u64, reason: u32 },
    RequestFrame,
    ShowKeyboard { options: ShowOptions },
    HideKeyboard,
    UpdateKeyboard { value: String },
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
        Event::RequestFrame => {
            if let Some(on_request_frame) = task.callbacks.on_request_frame {
                unsafe { on_request_frame(user_data, task.session) };
            }
        }
        Event::ShowKeyboard { options } => {
            if let Some(on_show_keyboard) = task.callbacks.on_show_keyboard {
                let default_value = options.default_value.as_bytes();
                let c_options = MigoKeyboardShowOptions {
                    header: VersionedHeader {
                        struct_size: size_of::<MigoKeyboardShowOptions>() as u32,
                        abi_version: crate::abi::MIGO_ABI_VERSION_CURRENT,
                    },
                    flags: options.flags,
                    max_length: options.max_length,
                    confirm_type: options.confirm_type,
                    keyboard_type: options.keyboard_type,
                    default_value_utf8: default_value.as_ptr() as *const c_char,
                    default_value_length: default_value.len() as u32,
                    reserved0: 0,
                };
                // `options` outlives the call because `task` is dropped after.
                unsafe { on_show_keyboard(user_data, task.session, &c_options) };
            }
        }
        Event::HideKeyboard => {
            if let Some(on_hide_keyboard) = task.callbacks.on_hide_keyboard {
                unsafe { on_hide_keyboard(user_data, task.session) };
            }
        }
        Event::UpdateKeyboard { value } => {
            if let Some(on_update_keyboard) = task.callbacks.on_update_keyboard {
                let bytes = value.as_bytes();
                unsafe {
                    on_update_keyboard(
                        user_data,
                        task.session,
                        bytes.as_ptr() as *const c_char,
                        bytes.len() as u32,
                    )
                };
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
    ///
    /// Returns whether the dispatcher accepted the task, which is as much as
    /// this side can know: acceptance is not evidence the host has acted.
    /// Notifications discard it, but the keyboard verbs answer content with it,
    /// because content that believes its keyboard is opening waits for input
    /// that will never arrive.
    fn post(&self, event: Event) -> bool {
        if !self.alive.load(Ordering::Acquire) {
            return false;
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
        let result =
            unsafe { (self.callbacks.dispatch)(self.callbacks.dispatcher_data, run_task, context) };
        if result != MIGO_OK {
            // Rejected: take the payload back and drop it, so the task neither
            // leaks nor runs.
            drop(unsafe { Box::from_raw(context as *mut Task) });
            tracing::warn!("host dispatcher rejected a callback task: {result}");
            return false;
        }
        true
    }

    pub fn ready(&self) {
        let _ = self.post(Event::Ready);
    }

    pub fn exit_requested(&self) {
        let _ = self.post(Event::ExitRequested);
    }

    pub fn error(&self, code: MigoResult, message: impl Into<Vec<u8>>) {
        // Interior NULs cannot travel through a C string; replace rather than
        // drop the report, since an error is exactly when detail matters.
        let message = CString::new(message).unwrap_or_else(|_| {
            CString::new("engine error (message contained an interior NUL)").expect("static")
        });
        let _ = self.post(Event::Error { code, message });
    }

    #[allow(dead_code)]
    /// Ask the host to schedule one frame. Silent when the host installed no
    /// frame callback: that host is engine-paced and never asked to be told.
    pub fn request_frame(&self) {
        if self.callbacks.on_request_frame.is_some() {
            let _ = self.post(Event::RequestFrame);
        }
    }

    /// Ask the host to show, hide or update its keyboard.
    ///
    /// The returned flag means the dispatcher accepted the task, not that the
    /// host has done anything: the ABI is asynchronous and cannot promise more.
    /// A refusal has to reach content as a failure.
    pub fn show_keyboard(&self, options: ShowOptions) -> bool {
        self.post(Event::ShowKeyboard { options })
    }

    pub fn hide_keyboard(&self) -> bool {
        self.post(Event::HideKeyboard)
    }

    pub fn update_keyboard(&self, value: String) -> bool {
        self.post(Event::UpdateKeyboard { value })
    }

    /// Whether the host offered to service the soft keyboard, which is what
    /// decides whether the capability is offered to content at all — the same
    /// rule `drives_frames` follows for the frame clock.
    pub fn supplies_keyboard(&self) -> bool {
        self.callbacks.supplies_keyboard()
    }

    /// Whether the host offered to drive frames, which is the honest answer to
    /// `FrameClock::uses_external_vsync` rather than a fixed one.
    pub fn drives_frames(&self) -> bool {
        self.callbacks.on_request_frame.is_some()
    }

    pub fn surface_lost(&self, generation: u64, reason: u32) {
        let _ = self.post(Event::SurfaceLost { generation, reason });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    static READY_CALLS: AtomicUsize = AtomicUsize::new(0);
    static DISPATCH_CALLS: AtomicUsize = AtomicUsize::new(0);

    /// The counters above are process-global, and cargo runs tests in parallel,
    /// so any two tests that reset and read them race. Serialising the ones that
    /// do is what makes their assertions mean what they say.
    static COUNTER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
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

    unsafe extern "C" fn on_error(
        _user: *mut c_void,
        _session: *mut c_void,
        error: *const MigoError,
    ) {
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
            on_request_frame: None,
            on_show_keyboard: None,
            on_hide_keyboard: None,
            on_update_keyboard: None,
        }
    }

    static SHOWN_MAX_LENGTH: AtomicUsize = AtomicUsize::new(0);
    static SHOWN_DEFAULT_VALUE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

    unsafe extern "C" fn noop_show_keyboard(
        _user: *mut c_void,
        _session: *mut c_void,
        _options: *const MigoKeyboardShowOptions,
    ) {
    }
    unsafe extern "C" fn noop_hide_keyboard(_user: *mut c_void, _session: *mut c_void) {}
    unsafe extern "C" fn noop_update_keyboard(
        _user: *mut c_void,
        _session: *mut c_void,
        _value_utf8: *const c_char,
        _value_length: u32,
    ) {
    }

    unsafe extern "C" fn recording_show_keyboard(
        _user: *mut c_void,
        _session: *mut c_void,
        options: *const MigoKeyboardShowOptions,
    ) {
        let options = unsafe { &*options };
        SHOWN_MAX_LENGTH.store(options.max_length as usize, Ordering::SeqCst);
        let bytes = unsafe {
            std::slice::from_raw_parts(
                options.default_value_utf8 as *const u8,
                options.default_value_length as usize,
            )
        };
        *SHOWN_DEFAULT_VALUE
            .lock()
            .unwrap_or_else(|e| e.into_inner()) =
            std::str::from_utf8(bytes).expect("utf-8").to_string();
    }

    /// A raw callback set with a working dispatcher and no keyboard, which the
    /// install tests then vary one field at a time.
    fn raw_callbacks() -> MigoHostCallbacks {
        MigoHostCallbacks {
            header: VersionedHeader {
                struct_size: size_of::<MigoHostCallbacks>() as u32,
                abi_version: crate::abi::MIGO_ABI_VERSION_CURRENT,
            },
            user_data: std::ptr::null_mut(),
            dispatcher_data: std::ptr::null_mut(),
            dispatch: Some(inline_dispatch),
            on_ready: None,
            on_error: None,
            on_exit_requested: None,
            on_surface_lost: None,
            on_request_frame: None,
            on_show_keyboard: None,
            on_hide_keyboard: None,
            on_update_keyboard: None,
        }
    }

    fn keyboard_notifier(
        dispatch: MigoDispatchFn,
        on_show_keyboard: Option<MigoOnShowKeyboardFn>,
    ) -> Notifier {
        let callbacks = HostCallbacks {
            user_data: std::ptr::null_mut(),
            dispatcher_data: std::ptr::null_mut(),
            dispatch,
            on_ready: None,
            on_error: None,
            on_exit_requested: None,
            on_surface_lost: None,
            on_request_frame: None,
            on_show_keyboard,
            on_hide_keyboard: Some(noop_hide_keyboard),
            on_update_keyboard: Some(noop_update_keyboard),
        };
        Notifier::new(
            callbacks,
            NonNull::new(0x1000usize as *mut c_void).expect("non-null session token"),
            Arc::new(AtomicBool::new(true)),
        )
    }

    /// Storage aligned like the real struct, so a short caller can be built
    /// byte-exactly. A `Vec<u8>` would not be guaranteed 8-aligned, and reading
    /// a `struct_size` out of a misaligned buffer is not the thing under test.
    #[repr(align(8))]
    struct ShortCaller([u8; size_of::<MigoHostCallbacks>()]);

    /// Build the first `size` bytes of a full callback set, as an older client's
    /// compiled code would have written them.
    fn truncated_caller(size: usize) -> ShortCaller {
        let mut full = raw_callbacks();
        full.header.struct_size = size as u32;
        let mut buffer = ShortCaller([0u8; size_of::<MigoHostCallbacks>()]);
        // SAFETY: both are exactly `size_of::<MigoHostCallbacks>()` bytes, and
        // the struct is plain data.
        unsafe {
            std::ptr::copy_nonoverlapping(
                &full as *const MigoHostCallbacks as *const u8,
                buffer.0.as_mut_ptr(),
                size_of::<MigoHostCallbacks>(),
            );
        }
        // Everything past the announced size belongs to nobody: an older client
        // never allocated it, so leaving real bytes there would let a passing
        // test hide a read past the caller's storage.
        for byte in &mut buffer.0[size..] {
            *byte = 0xAA;
        }
        buffer
    }

    /// A host compiled before the keyboard existed must still install.
    ///
    /// This is the case `session.h` has promised since the frame-pacing slice
    /// and the code did not honour: exact-size validation rejected it outright.
    #[test]
    fn a_client_from_before_the_keyboard_still_installs() {
        // 72 bytes: through `on_request_frame`, which is what the struct was
        // before the keyboard verbs were appended.
        let buffer = truncated_caller(72);
        let copied = unsafe {
            crate::abi::copy_versioned::<MigoHostCallbacks>(
                buffer.0.as_ptr() as *const VersionedHeader
            )
        }
        .expect("a client from the previous header must be accepted");

        assert!(
            copied.on_show_keyboard.is_none()
                && copied.on_hide_keyboard.is_none()
                && copied.on_update_keyboard.is_none(),
            "fields that client never had must read as absent, not as its neighbouring bytes"
        );
        assert!(
            unsafe { HostCallbacks::from_c(&copied) }
                .expect("valid")
                .supplies_keyboard()
                == false
        );
    }

    /// The documented floor: a dispatcher and nothing else. A host that wants no
    /// notifications at all is quiet, not wrong.
    #[test]
    fn a_client_with_only_a_dispatcher_installs() {
        let buffer = truncated_caller(32);
        let copied = unsafe {
            crate::abi::copy_versioned::<MigoHostCallbacks>(
                buffer.0.as_ptr() as *const VersionedHeader
            )
        }
        .expect("the minimum must be accepted");
        assert!(copied.dispatch.is_some());
        assert!(copied.on_ready.is_none());
    }

    /// One byte below the floor is missing part of `dispatch`, and there is no
    /// call to make without it.
    #[test]
    fn a_client_below_the_minimum_is_rejected() {
        let buffer = truncated_caller(31);
        assert_eq!(
            unsafe {
                crate::abi::copy_versioned::<MigoHostCallbacks>(
                    buffer.0.as_ptr() as *const VersionedHeader
                )
            }
            .err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    /// A bigger struct is a newer contract, and the caller needs to hear that
    /// rather than have its unknown fields silently dropped.
    #[test]
    fn a_client_from_a_newer_header_is_told_the_abi_is_unsupported() {
        let mut raw = raw_callbacks();
        raw.header.struct_size = size_of::<MigoHostCallbacks>() as u32 + 8;
        assert_eq!(
            unsafe {
                crate::abi::copy_versioned::<MigoHostCallbacks>(
                    &raw as *const MigoHostCallbacks as *const VersionedHeader,
                )
            }
            .err(),
            Some(crate::abi::MIGO_ERROR_UNSUPPORTED_ABI)
        );
    }

    /// A host that can open a keyboard but not close it strands it on screen
    /// with no event that corrects the state. Partial support is refused at
    /// install time, where the host can still fix it.
    #[test]
    fn keyboard_callbacks_install_all_or_none() {
        // None of the three: accepted, and the capability is not offered.
        let copied = unsafe { HostCallbacks::from_c(&raw_callbacks()) }.expect("none is valid");
        assert!(!copied.supplies_keyboard());

        // Each one alone, then two of three: all partial, all refused.
        let mut show_only = raw_callbacks();
        show_only.on_show_keyboard = Some(noop_show_keyboard);
        let mut hide_only = raw_callbacks();
        hide_only.on_hide_keyboard = Some(noop_hide_keyboard);
        let mut update_only = raw_callbacks();
        update_only.on_update_keyboard = Some(noop_update_keyboard);
        let mut two = raw_callbacks();
        two.on_show_keyboard = Some(noop_show_keyboard);
        two.on_hide_keyboard = Some(noop_hide_keyboard);

        for (name, partial) in [
            ("show only", &show_only),
            ("hide only", &hide_only),
            ("update only", &update_only),
            ("two of three", &two),
        ] {
            assert_eq!(
                unsafe { HostCallbacks::from_c(partial) }.err(),
                Some(MIGO_ERROR_INVALID_ARGUMENT),
                "{name} must be refused"
            );
        }

        // All three: accepted, and the capability is offered.
        let mut all = raw_callbacks();
        all.on_show_keyboard = Some(noop_show_keyboard);
        all.on_hide_keyboard = Some(noop_hide_keyboard);
        all.on_update_keyboard = Some(noop_update_keyboard);
        let copied = unsafe { HostCallbacks::from_c(&all) }.expect("all three is valid");
        assert!(copied.supplies_keyboard());
    }

    /// The verbs must report whether the dispatcher took the task. Reporting
    /// success for a refused dispatch would tell content its keyboard is
    /// opening when nothing will ever open it.
    #[test]
    fn a_refused_dispatch_is_reported_to_the_caller() {
        let accepting = keyboard_notifier(inline_dispatch, Some(noop_show_keyboard));
        assert!(accepting.show_keyboard(ShowOptions::default()));
        assert!(accepting.hide_keyboard());
        assert!(accepting.update_keyboard("x".to_string()));

        let refusing = keyboard_notifier(rejecting_dispatch, Some(noop_show_keyboard));
        assert!(!refusing.show_keyboard(ShowOptions::default()));
        assert!(!refusing.hide_keyboard());
        assert!(!refusing.update_keyboard("x".to_string()));
    }

    /// The options and their string are borrowed for the callback only, so the
    /// payload must still own them while the host reads them. A payload dropped
    /// before the call would hand the host a pointer into freed memory.
    #[test]
    fn show_options_reach_the_host_intact() {
        let _guard = COUNTER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        SHOWN_MAX_LENGTH.store(0, Ordering::SeqCst);
        SHOWN_DEFAULT_VALUE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();

        let notifier = keyboard_notifier(inline_dispatch, Some(recording_show_keyboard));
        assert!(notifier.show_keyboard(ShowOptions {
            flags: MIGO_KEYBOARD_FLAG_MULTIPLE,
            max_length: 140,
            confirm_type: MIGO_KEYBOARD_CONFIRM_SEND,
            keyboard_type: MIGO_KEYBOARD_TYPE_TEXT,
            default_value: "seed".to_string(),
        }));

        assert_eq!(SHOWN_MAX_LENGTH.load(Ordering::SeqCst), 140);
        assert_eq!(
            *SHOWN_DEFAULT_VALUE
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
            "seed"
        );
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
        let _guard = COUNTER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _guard = COUNTER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let mut raw = raw_callbacks();
        raw.dispatch = None;
        raw.on_ready = Some(on_ready);
        assert_eq!(
            unsafe { HostCallbacks::from_c(&raw) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }
}
