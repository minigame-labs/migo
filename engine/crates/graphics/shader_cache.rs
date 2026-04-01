//! Best-effort shader binary cache using `glGetProgramBinary` / `glProgramBinary`.
//!
//! Saves compiled GL program binaries to the app cache directory on first link,
//! and loads them on subsequent runs to skip runtime compilation (~50-200 ms saved
//! on cold start).
//!
//! **Best-effort only:** `glProgramBinary` is not portable across drivers or driver
//! versions.  Any load failure silently falls back to runtime compilation + re-cache.

use glow::HasContext;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use tracing::{debug, trace};

/// Manages a disk-backed shader binary cache.
pub struct ShaderCache {
    /// Base directory for cached binaries (e.g. `<app_cache>/migo_shader_cache/`).
    cache_dir: PathBuf,
    /// Fingerprint of the current GL driver (GL_RENDERER + GL_VERSION).
    /// When this changes (e.g. system update), all cached binaries are invalid.
    driver_key: String,
    /// `true` if `GL_NUM_PROGRAM_BINARY_FORMATS > 0`.
    supported: bool,
}

impl ShaderCache {
    /// Create a new cache.  `cache_root` is the app's cache directory.
    /// Probes GL capabilities; if program binary is not supported, all
    /// operations become no-ops.
    pub fn new(gl: &glow::Context, cache_root: &Path) -> Self {
        let renderer = unsafe { gl.get_parameter_string(glow::RENDERER) };
        let version = unsafe { gl.get_parameter_string(glow::VERSION) };
        let driver_key = format!("{renderer}||{version}");

        let num_formats = unsafe { gl.get_parameter_i32(glow::NUM_PROGRAM_BINARY_FORMATS) };
        let supported = num_formats > 0;

        let cache_dir = cache_root.join("migo_shader_cache");
        if supported {
            std::fs::create_dir_all(&cache_dir).ok();
            // Invalidate cache if driver changed.
            let key_path = cache_dir.join(".driver_key");
            let stale = std::fs::read_to_string(&key_path)
                .map(|s| s != driver_key)
                .unwrap_or(true);
            if stale {
                debug!("Shader cache: driver changed, clearing cache");
                clear_dir(&cache_dir);
                std::fs::write(&key_path, &driver_key).ok();
            }
        }

        debug!("ShaderCache: supported={supported}, driver_key={driver_key:.60}");

        Self {
            cache_dir,
            driver_key,
            supported,
        }
    }

    /// Returns true if program binary caching is supported on this driver.
    pub fn is_supported(&self) -> bool {
        self.supported
    }

    /// Try to load a cached binary for the given shader source pair.
    /// Returns `Some(binary_format, binary_data)` on cache hit.
    pub fn load(&self, vertex_src: &str, fragment_src: &str) -> Option<(u32, Vec<u8>)> {
        if !self.supported {
            return None;
        }
        let key = self.cache_key(vertex_src, fragment_src);
        let path = self.cache_dir.join(&key);

        let data = std::fs::read(&path).ok()?;
        // First 4 bytes: binary format (u32 LE).
        if data.len() < 4 {
            return None;
        }
        let format = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let binary = data[4..].to_vec();
        trace!("Shader cache hit: {key}");
        Some((format, binary))
    }

    /// Save a compiled program binary to disk.
    pub fn save(
        &self,
        gl: &glow::Context,
        program: glow::NativeProgram,
        vertex_src: &str,
        fragment_src: &str,
    ) {
        if !self.supported {
            return;
        }
        let (format, binary) = match get_program_binary(gl, program) {
            Some(v) => v,
            None => return,
        };
        let key = self.cache_key(vertex_src, fragment_src);
        let path = self.cache_dir.join(&key);

        // Prepend format as 4-byte LE header.
        let mut data = Vec::with_capacity(4 + binary.len());
        data.extend_from_slice(&format.to_le_bytes());
        data.extend_from_slice(&binary);

        if let Err(e) = std::fs::write(&path, &data) {
            debug!("Shader cache write failed: {e}");
        } else {
            trace!("Shader cache saved: {key} ({} bytes)", data.len());
        }
    }

    fn cache_key(&self, vertex_src: &str, fragment_src: &str) -> String {
        let mut hasher = DefaultHasher::new();
        vertex_src.hash(&mut hasher);
        fragment_src.hash(&mut hasher);
        format!("{:016x}.bin", hasher.finish())
    }
}

/// Get the binary representation of a linked program using glow's
/// `get_program_binary` wrapper.
fn get_program_binary(gl: &glow::Context, program: glow::NativeProgram) -> Option<(u32, Vec<u8>)> {
    unsafe {
        let length = gl.get_program_parameter_i32(program, glow::PROGRAM_BINARY_LENGTH);
        if length <= 0 {
            return None;
        }

        // glow wraps glGetProgramBinary → returns (format, Vec<u8>).
        match gl.get_program_binary(program) {
            Some(binary) => {
                if binary.buffer.is_empty() {
                    return None;
                }
                Some((binary.format, binary.buffer))
            }
            None => None,
        }
    }
}

fn clear_dir(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                std::fs::remove_file(&path).ok();
            }
        }
    }
}
