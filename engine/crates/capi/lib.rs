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

#[repr(C)]
struct MigoSurfaceDescriptor {
    header: VersionedHeader,
    generation: u64,
    platform_kind: u32,
    flags: u32,
    width_pixels: u32,
    height_pixels: u32,
    scale_factor: f32,
    color_space: u32,
    alpha_mode: u32,
    preferred_presentation_mode: u32,
    capability_flags: u64,
    platform_descriptor_size: u32,
    reserved0: u32,
    platform_descriptor: *const c_void,
}

#[repr(C)]
struct MigoX11WindowDescriptor {
    header: VersionedHeader,
    platform_kind: u32,
    flags: u32,
    display: *mut c_void,
    window: usize,
    screen: i32,
    reserved0: u32,
}

// ---- Handles ----------------------------------------------------------------

/// Process-level state: the storage roots the host granted, plus a live-session
/// count so `migo_engine_destroy` can enforce the header's rule that children
/// go first instead of leaving sessions pointing at freed configuration.
struct EngineInner {
    files_dir: PathBuf,
    cache_dir: PathBuf,
    code_cache_dir: PathBuf,
    live_sessions: Mutex<usize>,
}

pub struct MigoEngine {
    inner: Arc<EngineInner>,
}

#[derive(Default)]
struct SessionState {
    /// Set once a surface has been attached; the engine host thread owns the
    /// render loop from that point.
    host: Option<i32>,
    content_loaded: bool,
    attached: bool,
}

pub struct MigoSession {
    engine: Arc<EngineInner>,
    state: Mutex<SessionState>,
}

pub struct MigoSurfaceAttachment {
    session: NonNull<MigoSession>,
    generation: u64,
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

        let engine = Box::new(MigoEngine {
            inner: Arc::new(EngineInner {
                files_dir: PathBuf::from(files_dir),
                cache_dir: PathBuf::from(cache_dir),
                code_cache_dir: PathBuf::from(code_cache_dir),
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

// ---- Surface ----------------------------------------------------------------

/// # Safety
/// `session` must be live; `descriptor` a `MigoSurfaceDescriptor` whose
/// `platform_descriptor` points at the matching typed descriptor;
/// `out_attachment` writable storage for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_attach_surface(
    session: *mut MigoSession,
    descriptor: *const MigoSurfaceDescriptor,
    out_attachment: *mut *mut MigoSurfaceAttachment,
) -> MigoResult {
    guard("migo_session_attach_surface", || {
        let Some(session_ptr) = NonNull::new(session) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        let Some(out_attachment) = (unsafe { out_attachment.as_mut() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        if let Err(error) = unsafe {
            validate_header(
                descriptor as *const VersionedHeader,
                size_of::<MigoSurfaceDescriptor>(),
            )
        } {
            return error;
        }
        let descriptor = unsafe { &*descriptor };

        let (surface, graphics_platform) = match unsafe { build_target(descriptor) } {
            Ok(target) => target,
            Err(error) => return error,
        };

        let session_ref = unsafe { session_ptr.as_ref() };
        let Ok(mut state) = session_ref.state.lock() else {
            return MIGO_ERROR_INTERNAL;
        };
        if state.attached {
            return MIGO_ERROR_INVALID_STATE;
        }

        let host = match state.host {
            // Re-attach after a detach: the host thread is still alive, so the
            // surface joins it as a new generation rather than starting over.
            Some(host) => match core::lease_surface(host, surface) {
                Ok(_lease) => host,
                Err(error) => {
                    tracing::error!("migo_session_attach_surface: lease failed: {error}");
                    return MIGO_ERROR_INTERNAL;
                }
            },
            None => {
                let host_kit: Arc<dyn PlatformServices> =
                    Arc::new(platform::desktop::platform::DesktopPlatform::new());
                let options = InitOptions::new()
                    .with_files_dir(session_ref.engine.files_dir.clone())
                    .with_cache_dir(session_ref.engine.cache_dir.clone())
                    .with_code_cache_dir(session_ref.engine.code_cache_dir.clone())
                    .with_pixel_ratio(descriptor.scale_factor.max(1.0))
                    .with_target_fps(60);
                match spawn_host_thread(surface, graphics_platform, host_kit, options) {
                    Ok(host) => {
                        state.host = Some(host);
                        host
                    }
                    Err(error) => {
                        tracing::error!("migo_session_attach_surface: spawn failed: {error:?}");
                        return MIGO_ERROR_INTERNAL;
                    }
                }
            }
        };
        let _ = host;
        state.attached = true;

        let attachment = Box::new(MigoSurfaceAttachment {
            session: session_ptr,
            generation: descriptor.generation,
        });
        *out_attachment = Box::into_raw(attachment);
        MIGO_OK
    })
}

/// # Safety
/// `attachment` must come from [`migo_session_attach_surface`] and its session
/// must still be live. On `MIGO_OK` the handle is consumed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_surface_detach(
    attachment: *mut MigoSurfaceAttachment,
) -> MigoResult {
    guard("migo_surface_detach", || {
        if attachment.is_null() {
            return MIGO_ERROR_INVALID_ARGUMENT;
        }
        let attachment = unsafe { Box::from_raw(attachment) };
        let session = unsafe { attachment.session.as_ref() };
        let Ok(mut state) = session.state.lock() else {
            return MIGO_ERROR_INTERNAL;
        };
        if let Some(host) = state.host {
            // Retiring the generation is the synchronous completion boundary the
            // header promises: no later present may reference it.
            if let Err(error) = core::retire_surface(host) {
                tracing::warn!("migo_surface_detach: retire failed: {error}");
            }
        }
        state.attached = false;
        MIGO_OK
    })
}

/// Translate a validated descriptor into the engine's surface + graphics
/// platform pair.
///
/// # Safety
/// `descriptor` must have passed [`validate_header`].
unsafe fn build_target(
    descriptor: &MigoSurfaceDescriptor,
) -> Result<(SurfaceRef, graphics::egl_platform::GraphicsPlatform), MigoResult> {
    if descriptor.platform_kind != MIGO_PLATFORM_X11_WINDOW {
        return Err(MIGO_ERROR_UNSUPPORTED_PLATFORM);
    }
    // The envelope's size field and the payload's own struct_size are an
    // intentional cross-check; disagreeing means the caller mismatched them.
    if descriptor.platform_descriptor_size as usize != size_of::<MigoX11WindowDescriptor>() {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    unsafe {
        validate_header(
            descriptor.platform_descriptor as *const VersionedHeader,
            size_of::<MigoX11WindowDescriptor>(),
        )
    }?;
    let x11 = unsafe { &*(descriptor.platform_descriptor as *const MigoX11WindowDescriptor) };
    if x11.platform_kind != MIGO_PLATFORM_X11_WINDOW {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    let Some(display) = NonNull::new(x11.display) else {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    };
    if x11.window == 0 || descriptor.width_pixels == 0 || descriptor.height_pixels == 0 {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }

    let surface: SurfaceRef = Arc::new(platform::desktop::presenter::LinuxX11Surface::new(
        x11.window as std::ffi::c_ulong,
        descriptor.width_pixels,
        descriptor.height_pixels,
    ));
    let graphics_platform = platform::desktop::presenter::linux_x11_graphics_platform(display)
        .map_err(|error| {
            tracing::error!("migo_session_attach_surface: graphics platform: {error:?}");
            MIGO_ERROR_INTERNAL
        })?;
    Ok((surface, graphics_platform))
}

#[cfg(test)]
mod tests {
    use super::*;
    use abi::{MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_UNSUPPORTED_ABI};
    use std::ffi::CString;

    fn engine_config(
        dirs: &(CString, CString, CString),
        struct_size: u32,
        abi_version: u32,
    ) -> MigoEngineConfig {
        MigoEngineConfig {
            header: VersionedHeader {
                struct_size,
                abi_version,
            },
            flags: 0,
            reserved0: 0,
            files_dir_utf8: dirs.0.as_ptr(),
            cache_dir_utf8: dirs.1.as_ptr(),
            code_cache_dir_utf8: dirs.2.as_ptr(),
        }
    }

    fn scratch_dirs(tag: &str) -> (CString, CString, CString) {
        let root = std::env::temp_dir().join(format!("migo-capi-test-{tag}-{}", std::process::id()));
        let make = |name: &str| {
            CString::new(root.join(name).to_str().expect("utf-8 path")).expect("cstring")
        };
        (make("files"), make("cache"), make("code-cache"))
    }

    fn session_config() -> MigoSessionConfig {
        MigoSessionConfig {
            header: VersionedHeader {
                struct_size: size_of::<MigoSessionConfig>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            flags: 0,
        }
    }

    /// Create an engine, run `body`, then destroy it.
    fn with_engine(tag: &str, body: impl FnOnce(*mut MigoEngine)) {
        let dirs = scratch_dirs(tag);
        let config = engine_config(&dirs, size_of::<MigoEngineConfig>() as u32, MIGO_ABI_VERSION_CURRENT);
        let mut engine: *mut MigoEngine = std::ptr::null_mut();
        assert_eq!(unsafe { migo_engine_create(&config, &mut engine) }, MIGO_OK);
        assert!(!engine.is_null());
        body(engine);
        assert_eq!(unsafe { migo_engine_destroy(engine) }, MIGO_OK);
    }

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

    #[test]
    fn non_x11_platforms_are_reported_as_unsupported_not_invalid() {
        // A host on a platform this build does not implement should learn that,
        // rather than think its descriptor was malformed.
        let descriptor = MigoSurfaceDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoSurfaceDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            generation: 1,
            platform_kind: 2, // MIGO_PLATFORM_WIN32_HWND
            flags: 0,
            width_pixels: 640,
            height_pixels: 480,
            scale_factor: 1.0,
            color_space: 0,
            alpha_mode: 0,
            preferred_presentation_mode: 0,
            capability_flags: 0,
            platform_descriptor_size: size_of::<MigoX11WindowDescriptor>() as u32,
            reserved0: 0,
            platform_descriptor: std::ptr::null(),
        };
        let error = unsafe { build_target(&descriptor) }.err().expect("rejected");
        assert_eq!(error, MIGO_ERROR_UNSUPPORTED_PLATFORM);
    }

    #[test]
    fn x11_descriptor_size_mismatch_is_rejected() {
        // The envelope's platform_descriptor_size and the payload's struct_size
        // are a deliberate cross-check; disagreement means a mismatched build.
        let x11 = MigoX11WindowDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoX11WindowDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            platform_kind: MIGO_PLATFORM_X11_WINDOW,
            flags: 0,
            display: 0xdead_beef_usize as *mut c_void,
            window: 0x2a0_0001,
            screen: 0,
            reserved0: 0,
        };
        let descriptor = MigoSurfaceDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoSurfaceDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            generation: 1,
            platform_kind: MIGO_PLATFORM_X11_WINDOW,
            flags: 0,
            width_pixels: 640,
            height_pixels: 480,
            scale_factor: 1.0,
            color_space: 0,
            alpha_mode: 0,
            preferred_presentation_mode: 0,
            capability_flags: 0,
            platform_descriptor_size: 8, // wrong on purpose
            reserved0: 0,
            platform_descriptor: &x11 as *const _ as *const c_void,
        };
        let error = unsafe { build_target(&descriptor) }.err().expect("rejected");
        assert_eq!(error, MIGO_ERROR_INVALID_ARGUMENT);
    }

    #[test]
    fn x11_descriptor_requires_a_real_window_and_display() {
        let make = |display: *mut c_void, window: usize| MigoX11WindowDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoX11WindowDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            platform_kind: MIGO_PLATFORM_X11_WINDOW,
            flags: 0,
            display,
            window,
            screen: 0,
            reserved0: 0,
        };
        let describe = |x11: &MigoX11WindowDescriptor| MigoSurfaceDescriptor {
            header: VersionedHeader {
                struct_size: size_of::<MigoSurfaceDescriptor>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            generation: 1,
            platform_kind: MIGO_PLATFORM_X11_WINDOW,
            flags: 0,
            width_pixels: 640,
            height_pixels: 480,
            scale_factor: 1.0,
            color_space: 0,
            alpha_mode: 0,
            preferred_presentation_mode: 0,
            capability_flags: 0,
            platform_descriptor_size: size_of::<MigoX11WindowDescriptor>() as u32,
            reserved0: 0,
            platform_descriptor: x11 as *const _ as *const c_void,
        };

        let no_display = make(std::ptr::null_mut(), 0x2a0_0001);
        assert_eq!(
            unsafe { build_target(&describe(&no_display)) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );

        let no_window = make(0xdead_beef_usize as *mut c_void, 0);
        assert_eq!(
            unsafe { build_target(&describe(&no_window)) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }
}
