//! The `migo_*` C ABI implementation.
//!
//! Implements the entry points declared in `include/migo/`. The headers are the
//! specification; this crate is the only place that turns them into engine
//! calls, and every rule they state (versioned structs, borrowed-for-the-call
//! strings, handles consumed on success, no panic across the boundary) is
//! enforced in [`abi`].
//!
//! Scope of this slice: engine and session lifetime, X11 surface attach/detach,
//! and content loading — enough for a C host to put a game on screen. Callbacks
//! and the lifecycle/visibility/focus calls are declared in the headers but not
//! implemented yet; they are the next slice (see
//! `docs/superpowers/plans/2026-07-18-c-abi-runtime-plan.md`), and until then
//! `MIGO_C_ABI_HAS_RUNTIME` stays 0 so nothing advertises a complete runtime.

mod abi;
mod callbacks;
mod capabilities;
mod host_kit;
mod platform;
mod input;
mod keyboard;
mod layout;
mod surface;
#[cfg(test)]
mod test_support;

// The surface entry points and the descriptors they read live in their
// own module; re-exported so the crate's public surface is unchanged.
pub use input::{migo_session_send_touch, MigoTouchEvent, MigoTouchPoint};
pub use keyboard::{
    migo_session_send_key_event, migo_session_send_keyboard_event, MigoKeyEvent,
    MigoKeyboardEvent,
};
pub use surface::{
    migo_session_attach_surface, migo_surface_detach, migo_surface_update,
    MigoAndroidNativeWindowDescriptor, MigoSurfaceAttachment, MigoSurfaceDescriptor,
    MigoSurfaceMetrics,
    MigoX11WindowDescriptor,
};

use std::{
    ffi::c_void,
    os::raw::c_char,
    path::PathBuf,
    ptr::NonNull,
    sync::{Arc, Mutex},
};

use abi::{
    copy_utf8, guard, validate_header, MigoResult, VersionedHeader, MIGO_ERROR_INTERNAL,
    MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_INVALID_STATE, MIGO_ERROR_UNSUPPORTED_PLATFORM, MIGO_OK,
};
use core::{send_command_to_host, shutdown_host, spawn_host_thread, PlatformServices};
use shared::{config::InitOptions, protocol::host_cmd::HostCommand, surface::SurfaceRef};

/// `MIGO_PLATFORM_X11_WINDOW` from `include/migo/surface.h`.
const MIGO_PLATFORM_X11_WINDOW: u32 = 6;

/// `MIGO_PLATFORM_ANDROID_NATIVE_WINDOW` from `include/migo/surface.h`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const MIGO_PLATFORM_ANDROID_NATIVE_WINDOW: u32 = 1;

// ---- C struct mirrors -------------------------------------------------------
// Field-for-field mirrors of the headers. Any divergence is an ABI break, so
// they are checked against the headers by the layout tests below.

#[repr(C)]
struct MigoEngineConfig {
    header: VersionedHeader,
    flags: u64,
    reserved0: u32,
    files_dir_utf8: *const c_char,
    cache_dir_utf8: *const c_char,
    code_cache_dir_utf8: *const c_char,
}

#[repr(C)]
struct MigoSessionConfig {
    header: VersionedHeader,
    flags: u64,
}

#[repr(C)]
struct MigoContentDescriptor {
    header: VersionedHeader,
    flags: u32,
    reserved0: u32,
    content_id_utf8: *const c_char,
    entry_utf8: *const c_char,
}

// ---- Handles ----------------------------------------------------------------

/// Process-level state: the storage roots the host granted, plus a live-session
/// count so `migo_engine_destroy` can enforce the header's rule that children
/// go first instead of leaving sessions pointing at freed configuration.
struct EngineInner {
    files_dir: PathBuf,
    cache_dir: PathBuf,
    code_cache_dir: PathBuf,
    /// `MIGO_ENGINE_FLAG_ALLOW_UNSIGNED_CONTENT`: opt-in, never a default.
    allow_unsigned_content: bool,
    live_sessions: Mutex<usize>,
}

/// `MIGO_ENGINE_FLAG_ALLOW_UNSIGNED_CONTENT` from `include/migo/session.h`.
const MIGO_ENGINE_FLAG_ALLOW_UNSIGNED_CONTENT: u64 = 1 << 0;

pub struct MigoEngine {
    inner: Arc<EngineInner>,
}

#[derive(Default)]
struct SessionState {
    /// Installed once, before the first attach, per the header's rule that a
    /// queued task must never see a replaced pointer.
    callbacks: Option<callbacks::HostCallbacks>,
    /// Set once a surface has been attached; the engine host thread owns the
    /// render loop from that point.
    host: Option<i32>,
    content_loaded: bool,
    attached: bool,
    /// Visibility the host set before a surface existed.
    ///
    /// Visibility is a property of the session, not of the surface, and every
    /// Android lifecycle delivers RESUME before the window arrives. Rejecting
    /// the call until something is attached would make correct hosts look
    /// wrong; the value is remembered and applied when the surface lands.
    pending_visible: Option<bool>,
}

pub struct MigoSession {
    engine: Arc<EngineInner>,
    state: Mutex<SessionState>,
    /// Cleared by destroy so queued callbacks cancel instead of handing the
    /// host a session pointer it has already released.
    alive: Arc<std::sync::atomic::AtomicBool>,
}

// ---- Engine -----------------------------------------------------------------

/// # Safety
/// `config` must point to a `MigoEngineConfig`, `out_engine` to writable
/// storage for one pointer. Both are borrowed for the call only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_engine_create(
    config: *const MigoEngineConfig,
    out_engine: *mut *mut MigoEngine,
) -> MigoResult {
    guard("migo_engine_create", || {
        let Some(out_engine) = (unsafe { out_engine.as_mut() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        if let Err(error) = unsafe {
            validate_header(
                config as *const VersionedHeader,
                size_of::<MigoEngineConfig>(),
            )
        } {
            return error;
        }
        let config = unsafe { &*config };

        let (files_dir, cache_dir, code_cache_dir) = match (
            unsafe { copy_utf8(config.files_dir_utf8) },
            unsafe { copy_utf8(config.cache_dir_utf8) },
            unsafe { copy_utf8(config.code_cache_dir_utf8) },
        ) {
            (Ok(files), Ok(cache), Ok(code_cache)) => (files, cache, code_cache),
            _ => return MIGO_ERROR_INVALID_ARGUMENT,
        };

        // The host owns the layout; we only make sure the roots it named exist
        // before the engine starts writing under them.
        for dir in [&files_dir, &cache_dir, &code_cache_dir] {
            if std::fs::create_dir_all(dir).is_err() {
                return MIGO_ERROR_INVALID_ARGUMENT;
            }
        }

        init_dev_logging();

        let engine = Box::new(MigoEngine {
            inner: Arc::new(EngineInner {
                files_dir: PathBuf::from(files_dir),
                cache_dir: PathBuf::from(cache_dir),
                code_cache_dir: PathBuf::from(code_cache_dir),
                allow_unsigned_content: config.flags & MIGO_ENGINE_FLAG_ALLOW_UNSIGNED_CONTENT != 0,
                live_sessions: Mutex::new(0),
            }),
        });
        *out_engine = Box::into_raw(engine);
        MIGO_OK
    })
}

/// # Safety
/// `engine` must be a handle from [`migo_engine_create`] that has not been
/// destroyed. On `MIGO_OK` the pointer is consumed and invalid afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_engine_destroy(engine: *mut MigoEngine) -> MigoResult {
    guard("migo_engine_destroy", || {
        let Some(engine_ref) = (unsafe { engine.as_ref() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        // Refuse rather than free configuration that live sessions still read.
        match engine_ref.inner.live_sessions.lock() {
            Ok(live) if *live > 0 => return MIGO_ERROR_INVALID_STATE,
            Ok(_) => {}
            Err(_) => return MIGO_ERROR_INTERNAL,
        }
        drop(unsafe { Box::from_raw(engine) });
        MIGO_OK
    })
}

// ---- Session ----------------------------------------------------------------

/// # Safety
/// `engine` must be a live engine handle; `config` a `MigoSessionConfig`;
/// `out_session` writable storage for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_create(
    engine: *mut MigoEngine,
    config: *const MigoSessionConfig,
    out_session: *mut *mut MigoSession,
) -> MigoResult {
    guard("migo_session_create", || {
        let Some(engine) = (unsafe { engine.as_ref() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        let Some(out_session) = (unsafe { out_session.as_mut() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        if let Err(error) = unsafe {
            validate_header(
                config as *const VersionedHeader,
                size_of::<MigoSessionConfig>(),
            )
        } {
            return error;
        }

        match engine.inner.live_sessions.lock() {
            Ok(mut live) => *live += 1,
            Err(_) => return MIGO_ERROR_INTERNAL,
        }
        let session = Box::new(MigoSession {
            engine: Arc::clone(&engine.inner),
            state: Mutex::new(SessionState::default()),
            alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        });
        *out_session = Box::into_raw(session);
        MIGO_OK
    })
}

/// # Safety
/// `session` must be a live session handle. On `MIGO_OK` it is consumed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_destroy(session: *mut MigoSession) -> MigoResult {
    guard("migo_session_destroy", || {
        if session.is_null() {
            return MIGO_ERROR_INVALID_ARGUMENT;
        }
        let session = unsafe { Box::from_raw(session) };
        // Cancel queued callbacks before anything else: from here on the
        // session pointer must never reach host code again.
        session
            .alive
            .store(false, std::sync::atomic::Ordering::Release);
        // Stopping the host also retires whatever surface is still attached,
        // which is why the header documents destroy as consuming every live
        // attachment.
        let host = session.state.lock().ok().and_then(|state| state.host);
        if let Some(host) = host {
            let _ = shutdown_host(host);
        }
        if let Ok(mut live) = session.engine.live_sessions.lock() {
            *live = live.saturating_sub(1);
        }
        MIGO_OK
    })
}

/// # Safety
/// `session` must be live; `content` a `MigoContentDescriptor` borrowed for the
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_load_content(
    session: *mut MigoSession,
    content: *const MigoContentDescriptor,
) -> MigoResult {
    guard("migo_session_load_content", || {
        let Some(session) = (unsafe { session.as_ref() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        if let Err(error) = unsafe {
            validate_header(
                content as *const VersionedHeader,
                size_of::<MigoContentDescriptor>(),
            )
        } {
            return error;
        }
        let content = unsafe { &*content };
        let (Ok(content_id), Ok(entry)) = (unsafe { copy_utf8(content.content_id_utf8) }, unsafe {
            copy_utf8(content.entry_utf8)
        }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };

        let Ok(mut state) = session.state.lock() else {
            return MIGO_ERROR_INTERNAL;
        };
        // Content needs a render target: without one there is no host thread to
        // evaluate it on.
        let Some(host) = state.host else {
            return MIGO_ERROR_INVALID_STATE;
        };
        if state.content_loaded {
            return MIGO_ERROR_INVALID_STATE;
        }

        match send_command_to_host(
            host,
            HostCommand::EvaluateModule {
                game_id: content_id,
                entry,
            },
        ) {
            Ok(()) => {
                state.content_loaded = true;
                MIGO_OK
            }
            Err(error) => {
                tracing::error!("migo_session_load_content: {error}");
                MIGO_ERROR_INTERNAL
            }
        }
    })
}

/// # Safety
/// `session` must be live; `callbacks` a `MigoHostCallbacks` borrowed for the
/// call. Only the fields it declares are copied.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_set_host_callbacks(
    session: *mut MigoSession,
    callbacks: *const callbacks::MigoHostCallbacks,
) -> MigoResult {
    guard("migo_session_set_host_callbacks", || {
        let Some(session) = (unsafe { session.as_ref() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        // Copied rather than reinterpreted: a host compiled against an earlier
        // header wrote fewer bytes than this struct now holds, and reading the
        // appended fields straight from its pointer would read past what it
        // allocated. `copy_versioned` reads only what the caller announced and
        // leaves the rest null, which is what "that host never had this
        // callback" already means.
        let raw = match unsafe {
            abi::copy_versioned::<callbacks::MigoHostCallbacks>(
                callbacks as *const VersionedHeader,
            )
        } {
            Ok(raw) => raw,
            Err(error) => return error,
        };
        let copied = match unsafe { callbacks::HostCallbacks::from_c(&raw) } {
            Ok(copied) => copied,
            Err(error) => return error,
        };

        let Ok(mut state) = session.state.lock() else {
            return MIGO_ERROR_INTERNAL;
        };
        // Install-once, before the first attach: a second set would let tasks
        // already queued against the old pointers run against the new ones.
        if state.callbacks.is_some() || state.host.is_some() {
            return MIGO_ERROR_INVALID_STATE;
        }
        state.callbacks = Some(copied);
        MIGO_OK
    })
}

/// Lifecycle states from `include/migo/session.h`.
const MIGO_LIFECYCLE_CREATED: u32 = 0;
const MIGO_LIFECYCLE_RUNNING: u32 = 1;
const MIGO_LIFECYCLE_PAUSED: u32 = 2;

/// Drive the engine's show/hide channel — the same one Android's `onShow` /
/// `onHide` use, so a desktop host produces the lifecycle the content already
/// expects instead of a second, divergent notion of "paused".
///
/// `send_critical_command_to_host` matches Android: lifecycle must not be
/// dropped when the command queue is saturated.
/// Report that a frame boundary arrived, in response to `on_request_frame`.
///
/// # Safety
/// `session` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_notify_vsync(
    session: *mut MigoSession,
    frame_time_nanos: i64,
) -> MigoResult {
    guard("migo_session_notify_vsync", || {
        let Some(session) = (unsafe { session.as_ref() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        let Ok(state) = session.state.lock() else {
            return MIGO_ERROR_INTERNAL;
        };
        // Nothing is rendering yet, so there is no frame to pace. Reporting
        // success would tell the host its tick was consumed.
        let Some(host) = state.host else {
            return MIGO_ERROR_INVALID_STATE;
        };
        drop(state);
        // The engine measures frame time in milliseconds; the host reports the
        // platform's nanosecond timestamp, so the conversion belongs here
        // rather than in every host.
        core::send_vsync(host, frame_time_nanos as f64 / 1_000_000.0);
        MIGO_OK
    })
}

fn drive_visibility(session: &MigoSession, visible: bool) -> MigoResult {
    let Ok(mut state) = session.state.lock() else {
        return MIGO_ERROR_INTERNAL;
    };
    // Before a surface exists there is nothing to show or hide yet, but the
    // host is not wrong to have said so: remember it for attach.
    let Some(host) = state.host else {
        state.pending_visible = Some(visible);
        return MIGO_OK;
    };
    state.pending_visible = None;
    let command = if visible {
        HostCommand::OnShow {
            options_json: None,
        }
    } else {
        HostCommand::OnHide
    };
    match core::send_critical_command_to_host(host, command) {
        Ok(()) => MIGO_OK,
        Err(error) => {
            tracing::error!("lifecycle command failed: {error}");
            MIGO_ERROR_INTERNAL
        }
    }
}

/// # Safety
/// `session` must be a live session handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_set_lifecycle(
    session: *mut MigoSession,
    state: u32,
) -> MigoResult {
    guard("migo_session_set_lifecycle", || {
        let Some(session) = (unsafe { session.as_ref() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        match state {
            MIGO_LIFECYCLE_RUNNING => drive_visibility(session, true),
            MIGO_LIFECYCLE_PAUSED => drive_visibility(session, false),
            // CREATED is where a session starts; asking to go back to it would
            // mean unwinding a running engine, which the ABI does not define.
            MIGO_LIFECYCLE_CREATED => MIGO_ERROR_INVALID_STATE,
            _ => MIGO_ERROR_INVALID_ARGUMENT,
        }
    })
}

/// # Safety
/// `session` must be a live session handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_set_visibility(
    session: *mut MigoSession,
    visible: u8,
) -> MigoResult {
    guard("migo_session_set_visibility", || {
        let Some(session) = (unsafe { session.as_ref() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        drive_visibility(session, visible != 0)
    })
}

/// # Safety
/// `session` must be a live session handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_set_focus(
    session: *mut MigoSession,
    _focused: u8,
) -> MigoResult {
    guard("migo_session_set_focus", || {
        let Some(session) = (unsafe { session.as_ref() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        // No surface requirement: focus is a property of the session, and every
        // Android lifecycle reports it before the window exists. Rejecting it
        // there would make a correct host look wrong.
        let Ok(_state) = session.state.lock() else {
            return MIGO_ERROR_INTERNAL;
        };
        // Focus is validated and accepted, but the engine has no separate focus
        // channel today: wx-style content observes show/hide, not focus. Wiring
        // it to visibility would pause a game that merely lost keyboard focus
        // while still on screen, so it stays a no-op until content needs it.
        MIGO_OK
    })
}

// ---- Surface ----------------------------------------------------------------

/// # Safety

/// Install a log subscriber when `MIGO_CAPI_LOG` is set.
///
/// A library has no business hijacking the process's global logger, so this is
/// opt-in and off by default. It exists because a C host currently has no other
/// way to see engine diagnostics: `on_error` is declared in the headers but not
/// implemented yet, which makes a failed content load silent. Once callbacks
/// land this becomes a convenience rather than the only channel.
fn init_dev_logging() {
    use shared::config::LogLevel;
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(level) = std::env::var_os("MIGO_CAPI_LOG") else {
            return;
        };
        let level = level.to_string_lossy().to_lowercase();
        let level = match level.as_str() {
            "trace" => LogLevel::Trace,
            "debug" => LogLevel::Debug,
            "warn" => LogLevel::Warn,
            "error" => LogLevel::Error,
            _ => LogLevel::Info,
        };
        crate::platform::install_dev_logging(level);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use abi::{MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_UNSUPPORTED_ABI};
    use crate::test_support::{engine_config, scratch_dirs, session_config, with_engine};
    use std::ffi::CString;





    #[test]
    fn engine_rejects_a_config_from_a_different_abi() {
        let dirs = scratch_dirs("abi");
        let config = engine_config(
            &dirs,
            size_of::<MigoEngineConfig>() as u32,
            MIGO_ABI_VERSION_CURRENT + 1,
        );
        let mut engine: *mut MigoEngine = std::ptr::null_mut();
        assert_eq!(
            unsafe { migo_engine_create(&config, &mut engine) },
            MIGO_ERROR_UNSUPPORTED_ABI
        );
        assert!(engine.is_null(), "no handle may escape a rejected call");
    }

    #[test]
    fn unsigned_content_is_refused_unless_the_host_opts_in() {
        // The signing check is the default; a host that wants unsigned content
        // has to say so, because silently accepting it defeats the check.
        let dirs = scratch_dirs("signing");
        let mut config =
            engine_config(&dirs, size_of::<MigoEngineConfig>() as u32, MIGO_ABI_VERSION_CURRENT);
        let mut engine: *mut MigoEngine = std::ptr::null_mut();
        assert_eq!(unsafe { migo_engine_create(&config, &mut engine) }, MIGO_OK);
        assert!(
            !unsafe { &*engine }.inner.allow_unsigned_content,
            "MIGO_ENGINE_FLAG_NONE must keep signing enforced"
        );
        assert_eq!(unsafe { migo_engine_destroy(engine) }, MIGO_OK);

        config.flags = MIGO_ENGINE_FLAG_ALLOW_UNSIGNED_CONTENT;
        let mut engine: *mut MigoEngine = std::ptr::null_mut();
        assert_eq!(unsafe { migo_engine_create(&config, &mut engine) }, MIGO_OK);
        assert!(unsafe { &*engine }.inner.allow_unsigned_content);
        assert_eq!(unsafe { migo_engine_destroy(engine) }, MIGO_OK);
    }

    #[test]
    fn engine_requires_storage_roots() {
        let dirs = scratch_dirs("roots");
        let mut config = engine_config(&dirs, size_of::<MigoEngineConfig>() as u32, MIGO_ABI_VERSION_CURRENT);
        config.files_dir_utf8 = std::ptr::null();
        let mut engine: *mut MigoEngine = std::ptr::null_mut();
        assert_eq!(
            unsafe { migo_engine_create(&config, &mut engine) },
            MIGO_ERROR_INVALID_ARGUMENT
        );
    }

    #[test]
    fn engine_creates_its_storage_roots() {
        let dirs = scratch_dirs("mkdir");
        let files = PathBuf::from(dirs.0.to_str().expect("utf-8"));
        let _ = std::fs::remove_dir_all(&files);
        with_engine("mkdir", |_| {});
        assert!(files.is_dir(), "engine must create the roots the host named");
    }

    #[test]
    fn engine_refuses_to_die_while_sessions_are_live() {
        // Otherwise the session would keep reading configuration that just got
        // freed underneath it.
        let dirs = scratch_dirs("children");
        let config = engine_config(&dirs, size_of::<MigoEngineConfig>() as u32, MIGO_ABI_VERSION_CURRENT);
        let mut engine: *mut MigoEngine = std::ptr::null_mut();
        assert_eq!(unsafe { migo_engine_create(&config, &mut engine) }, MIGO_OK);

        let session_config = session_config();
        let mut session: *mut MigoSession = std::ptr::null_mut();
        assert_eq!(
            unsafe { migo_session_create(engine, &session_config, &mut session) },
            MIGO_OK
        );
        assert_eq!(
            unsafe { migo_engine_destroy(engine) },
            MIGO_ERROR_INVALID_STATE
        );

        assert_eq!(unsafe { migo_session_destroy(session) }, MIGO_OK);
        assert_eq!(unsafe { migo_engine_destroy(engine) }, MIGO_OK);
    }

    #[test]
    fn loading_content_before_a_surface_is_a_state_error() {
        // No surface means no host thread, so there is nothing to evaluate on.
        with_engine("content", |engine| {
            let session_config = session_config();
            let mut session: *mut MigoSession = std::ptr::null_mut();
            assert_eq!(
                unsafe { migo_session_create(engine, &session_config, &mut session) },
                MIGO_OK
            );

            let content_id = CString::new("demo").expect("cstring");
            let entry = CString::new("game.js").expect("cstring");
            let content = MigoContentDescriptor {
                header: VersionedHeader {
                    struct_size: size_of::<MigoContentDescriptor>() as u32,
                    abi_version: MIGO_ABI_VERSION_CURRENT,
                },
                flags: 0,
                reserved0: 0,
                content_id_utf8: content_id.as_ptr(),
                entry_utf8: entry.as_ptr(),
            };
            assert_eq!(
                unsafe { migo_session_load_content(session, &content) },
                MIGO_ERROR_INVALID_STATE
            );
            assert_eq!(unsafe { migo_session_destroy(session) }, MIGO_OK);
        });
    }

    #[test]
    fn lifecycle_calls_are_accepted_before_a_surface_exists() {
        // These describe the session, not the surface, and every Android
        // lifecycle delivers resume and focus before the window arrives.
        // Rejecting them there would make a correct host look wrong; the
        // visibility is remembered and applied when the surface attaches.
        with_engine("lifecycle", |engine| {
            let session_config = session_config();
            let mut session: *mut MigoSession = std::ptr::null_mut();
            assert_eq!(
                unsafe { migo_session_create(engine, &session_config, &mut session) },
                MIGO_OK
            );
            assert_eq!(
                unsafe { migo_session_set_lifecycle(session, MIGO_LIFECYCLE_RUNNING) },
                MIGO_OK
            );
            assert_eq!(unsafe { migo_session_set_visibility(session, 1) }, MIGO_OK);
            assert_eq!(unsafe { migo_session_set_focus(session, 1) }, MIGO_OK);
            assert_eq!(unsafe { migo_session_destroy(session) }, MIGO_OK);
        });
    }

    #[test]
    fn returning_to_the_created_state_is_still_rejected() {
        // Unlike the ordering cases above, this is a genuinely undefined
        // transition: unwinding a running engine is not something the ABI
        // describes, so it stays an error rather than becoming a deferral.
        with_engine("lifecycle-created", |engine| {
            let session_config = session_config();
            let mut session: *mut MigoSession = std::ptr::null_mut();
            assert_eq!(
                unsafe { migo_session_create(engine, &session_config, &mut session) },
                MIGO_OK
            );
            assert_eq!(
                unsafe { migo_session_set_lifecycle(session, MIGO_LIFECYCLE_CREATED) },
                MIGO_ERROR_INVALID_STATE
            );
            assert_eq!(unsafe { migo_session_destroy(session) }, MIGO_OK);
        });
    }

    #[test]
    fn a_vsync_without_a_surface_is_a_state_error() {
        // Unlike the session-level calls, a frame tick has nothing to pace
        // until something is rendering, and reporting success would tell the
        // host its tick was consumed.
        with_engine("vsync-detached", |engine| {
            let session_config = session_config();
            let mut session: *mut MigoSession = std::ptr::null_mut();
            assert_eq!(
                unsafe { migo_session_create(engine, &session_config, &mut session) },
                MIGO_OK
            );
            assert_eq!(
                unsafe { migo_session_notify_vsync(session, 1_000_000) },
                MIGO_ERROR_INVALID_STATE
            );
            assert_eq!(unsafe { migo_session_destroy(session) }, MIGO_OK);
        });
    }

    #[test]
    fn unknown_lifecycle_states_are_told_apart_from_unsupported_transitions() {
        // A value outside the enum is a bad argument; going back to CREATED is
        // a defined value the ABI simply does not support.
        with_engine("lifecycle-values", |engine| {
            let session_config = session_config();
            let mut session: *mut MigoSession = std::ptr::null_mut();
            assert_eq!(
                unsafe { migo_session_create(engine, &session_config, &mut session) },
                MIGO_OK
            );
            assert_eq!(
                unsafe { migo_session_set_lifecycle(session, 99) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
            assert_eq!(
                unsafe { migo_session_set_lifecycle(session, MIGO_LIFECYCLE_CREATED) },
                MIGO_ERROR_INVALID_STATE
            );
            assert_eq!(unsafe { migo_session_destroy(session) }, MIGO_OK);
        });
    }

    #[test]
    fn callbacks_install_only_once() {
        // A second install would let tasks queued against the old pointers run
        // against the new ones.
        with_engine("callbacks-once", |engine| {
            let session_config = session_config();
            let mut session: *mut MigoSession = std::ptr::null_mut();
            assert_eq!(
                unsafe { migo_session_create(engine, &session_config, &mut session) },
                MIGO_OK
            );

            unsafe extern "C" fn dispatch(
                _dispatcher: *mut c_void,
                task: callbacks::MigoTaskFn,
                context: *mut c_void,
            ) -> MigoResult {
                unsafe { task(context) };
                MIGO_OK
            }
            unsafe extern "C" fn on_ready(_user: *mut c_void, _session: *mut c_void) {}

            let host_callbacks = callbacks::MigoHostCallbacks {
                header: VersionedHeader {
                    struct_size: size_of::<callbacks::MigoHostCallbacks>() as u32,
                    abi_version: MIGO_ABI_VERSION_CURRENT,
                },
                user_data: std::ptr::null_mut(),
                dispatcher_data: std::ptr::null_mut(),
                dispatch: Some(dispatch),
                on_ready: Some(on_ready),
                on_error: None,
                on_exit_requested: None,
                on_surface_lost: None,
                on_request_frame: None,
                on_show_keyboard: None,
                on_hide_keyboard: None,
                on_update_keyboard: None,
            };
            assert_eq!(
                unsafe { migo_session_set_host_callbacks(session, &host_callbacks) },
                MIGO_OK
            );
            assert_eq!(
                unsafe { migo_session_set_host_callbacks(session, &host_callbacks) },
                MIGO_ERROR_INVALID_STATE
            );
            assert_eq!(unsafe { migo_session_destroy(session) }, MIGO_OK);
        });
    }

    #[test]
    fn null_handles_are_argument_errors_not_crashes() {
        assert_eq!(
            unsafe { migo_engine_destroy(std::ptr::null_mut()) },
            MIGO_ERROR_INVALID_ARGUMENT
        );
        assert_eq!(
            unsafe { migo_session_destroy(std::ptr::null_mut()) },
            MIGO_ERROR_INVALID_ARGUMENT
        );
        assert_eq!(
            unsafe { migo_surface_detach(std::ptr::null_mut()) },
            MIGO_ERROR_INVALID_ARGUMENT
        );
    }



}
