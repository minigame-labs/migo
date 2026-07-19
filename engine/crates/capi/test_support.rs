//! Fixtures shared by the crate's test modules.
//!
//! They live here rather than in one module's `mod tests` so the surface tests
//! and the lifecycle tests build their engines and sessions the same way. Two
//! copies of a fixture drift, and a drifted fixture makes two tests that look
//! identical prove different things.

use std::ffi::CString;

use crate::{
    abi::{VersionedHeader, MIGO_ABI_VERSION_CURRENT, MIGO_OK},
    migo_engine_create, migo_engine_destroy, migo_session_create, migo_session_destroy,
    MigoEngine, MigoEngineConfig, MigoSession, MigoSessionConfig,
};

pub(crate) fn engine_config(
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

/// Storage roots under the process's temp dir, keyed by `tag` so concurrently
/// running tests do not share state.
pub(crate) fn scratch_dirs(tag: &str) -> (CString, CString, CString) {
    let root = std::env::temp_dir().join(format!("migo-capi-test-{tag}-{}", std::process::id()));
    let make =
        |name: &str| CString::new(root.join(name).to_str().expect("utf-8 path")).expect("cstring");
    (make("files"), make("cache"), make("code-cache"))
}

pub(crate) fn session_config() -> MigoSessionConfig {
    MigoSessionConfig {
        header: VersionedHeader {
            struct_size: size_of::<MigoSessionConfig>() as u32,
            abi_version: MIGO_ABI_VERSION_CURRENT,
        },
        flags: 0,
    }
}

/// Create an engine, run `body`, then destroy it.
pub(crate) fn with_engine(tag: &str, body: impl FnOnce(*mut MigoEngine)) {
    let dirs = scratch_dirs(tag);
    let config = engine_config(
        &dirs,
        size_of::<MigoEngineConfig>() as u32,
        MIGO_ABI_VERSION_CURRENT,
    );
    let mut engine: *mut MigoEngine = std::ptr::null_mut();
    assert_eq!(unsafe { migo_engine_create(&config, &mut engine) }, MIGO_OK);
    assert!(!engine.is_null());
    body(engine);
    assert_eq!(unsafe { migo_engine_destroy(engine) }, MIGO_OK);
}

/// Create an engine and one session, run `body`, then tear both down in order.
///
/// The session is real rather than a stand-in: the surface entry points read
/// engine configuration through it, so a hand-built session would not exercise
/// the same field accesses.
pub(crate) fn with_session(tag: &str, body: impl FnOnce(*mut MigoSession)) {
    with_engine(tag, |engine| {
        let config = session_config();
        let mut session: *mut MigoSession = std::ptr::null_mut();
        assert_eq!(
            unsafe { migo_session_create(engine, &config, &mut session) },
            MIGO_OK
        );
        assert!(!session.is_null());
        body(session);
        assert_eq!(unsafe { migo_session_destroy(session) }, MIGO_OK);
    });
}
