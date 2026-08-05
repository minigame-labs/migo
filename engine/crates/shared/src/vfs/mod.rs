//! Virtual File System for game sandboxing — hardened against symlink escape.
//!
//! Provides path isolation and permission management for games.
//! Each game has its own virtual paths that map to real directories:
//!
//! - `/user` → User data (saves, preferences) - read/write
//! - `/cache` → Cache files - read/write
//! - `/code` → Game code - read only
//! - `/tmp` → Temporary files - read/write
//!
//! Each root is a directory of its own. In particular `/cache` is
//! [`GamePaths::sandbox_cache_dir`], not the per-game cache root: the root also
//! holds the subpackage install store and staging directories, and a mapping
//! that contained it would let a game rewrite the record deciding which package
//! bytes a later session mounts as `/code`.
//!
//! # Security model
//!
//! Every call to [`VirtualFS::resolve`] performs a **three-phase** check:
//!
//! 1. **Input sanitization** — reject null bytes, backslashes, control chars.
//! 2. **Textual normalization** — resolve `.` / `..` components purely in
//!    memory and verify the result `starts_with(base)`.  This is a fast
//!    reject that needs no syscalls.
//! 3. **Filesystem-level verification** — if the path (or its closest
//!    existing ancestor) lives on disk, call `canonicalize` to resolve all
//!    symlinks and re-verify containment.  An optional *symlink policy* can
//!    reject symlinks outright in read-only directories (`/code`).
//!
//! # Example
//!
//! ```rust,ignore
//! use shared::vfs::{GamePaths, VirtualFS, VfsPolicy};
//!
//! let paths = GamePaths::new("/data/files", "/data/cache", "my-game")?;
//! paths.ensure_directories()?;
//!
//! let vfs = VirtualFS::from_game_paths(&paths);
//! let real = vfs.resolve("/user/save.json", FileOp::Write)?;
//! ```

pub mod game_paths;
#[cfg(feature = "code-signing")]
pub mod integrity;
pub mod mount;
pub mod package;

pub use game_paths::{GamePathError, GamePathStrings, GamePaths, validate_game_id};
pub use mount::{DirSource, MountBackend, MountTable, ResolvedCode, StagingArea};
pub use package::{PackSource, PackageError, PackageIdentity, PackageReader, PackageWriter};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// FileOp & FilePermissions (unchanged)
// ---------------------------------------------------------------------------

/// File operation type for permission checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOp {
    Read,
    Write,
    Create,
    Delete,
}

/// File permissions for a virtual path.
#[derive(Debug, Clone, Copy)]
pub struct FilePermissions {
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub delete: bool,
}

impl FilePermissions {
    pub const READ_ONLY: Self = Self {
        read: true,
        write: false,
        create: false,
        delete: false,
    };

    pub const READ_WRITE: Self = Self {
        read: true,
        write: true,
        create: true,
        delete: true,
    };

    pub fn allows(&self, op: FileOp) -> bool {
        match op {
            FileOp::Read => self.read,
            FileOp::Write => self.write,
            FileOp::Create => self.create,
            FileOp::Delete => self.delete,
        }
    }
}

// ---------------------------------------------------------------------------
// PathMapping
// ---------------------------------------------------------------------------

/// Mapping from virtual path prefix to real path.
#[derive(Debug, Clone)]
pub struct PathMapping {
    pub virtual_prefix: &'static str,
    /// Pre-computed `"{virtual_prefix}/"` to avoid per-resolve allocation.
    prefix_with_slash: String,
    pub real_path: PathBuf,
    /// Pre-resolved canonical base (follows symlinks at construction time).
    /// Falls back to the textually-normalized `real_path` when the directory
    /// does not exist yet.
    canonical_base: PathBuf,
    pub permissions: FilePermissions,
}

// ---------------------------------------------------------------------------
// VfsPolicy
// ---------------------------------------------------------------------------

/// Security policy knobs for the virtual file system.
#[derive(Debug, Clone)]
pub struct VfsPolicy {
    /// When `true`, any path component under a **read-only** mapping
    /// (`/code`) that is a symlink will be rejected.  This prevents a
    /// malicious game package from shipping symlinks that escape the sandbox.
    ///
    /// Default: **true**.
    pub deny_symlinks_in_code_dir: bool,

    /// When `true`, symlinks are also rejected in **writable** directories
    /// (`/user`, `/cache`, `/tmp`).  Without this, a symlink planted in a
    /// writable directory could escape the sandbox via read/write operations.
    ///
    /// Default: **true**.
    pub deny_symlinks_in_writable_dirs: bool,
}

/// ## Symlink policy design
///
/// Both fields default to `true` (deny symlinks) because the VFS is a
/// security sandbox: game code runs inside virtual directory mappings and
/// must not escape to the host filesystem.
///
/// - **`deny_symlinks_in_code_dir = true`** prevents a malicious game package
///   from shipping symlinks that point outside `/code`, which would let it
///   read arbitrary host files at runtime.
///
/// - **`deny_symlinks_in_writable_dirs = true`** prevents a game from
///   creating symlinks in `/user`, `/cache`, or `/tmp` that point outside
///   the sandbox, which would let it read or write arbitrary host files.
///
/// Setting either to `false` weakens the sandbox and should only be done in
/// trusted/development environments where symlink-based escape is acceptable.
/// There is intentionally no single "allow all symlinks" toggle to force
/// callers to opt in per-directory-class.
impl Default for VfsPolicy {
    fn default() -> Self {
        Self {
            deny_symlinks_in_code_dir: true,
            deny_symlinks_in_writable_dirs: true,
        }
    }
}

// ---------------------------------------------------------------------------
// VfsError
// ---------------------------------------------------------------------------

/// Error types for VFS operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VfsError {
    /// Path is not within any allowed virtual directory.
    PathNotAllowed,
    /// Operation is not permitted for this path.
    PermissionDenied,
    /// Path traversal attack detected (e.g., `../../../etc/passwd`).
    PathTraversal,
    /// A symlink resolves to a location outside the sandbox.
    SymlinkEscape,
    /// Symlinks are not allowed in this directory (policy violation).
    SymlinkNotAllowed,
    /// Invalid path format (null bytes, control chars, etc.).
    InvalidPath,
}

impl std::fmt::Display for VfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VfsError::PathNotAllowed => {
                write!(f, "Path is not within any allowed virtual directory")
            }
            VfsError::PermissionDenied => write!(f, "Permission denied for this operation"),
            VfsError::PathTraversal => write!(f, "Path traversal detected"),
            VfsError::SymlinkEscape => write!(f, "Symlink resolves outside sandbox"),
            VfsError::SymlinkNotAllowed => write!(f, "Symlinks not allowed in this directory"),
            VfsError::InvalidPath => write!(f, "Invalid path format"),
        }
    }
}

impl std::error::Error for VfsError {}

// ---------------------------------------------------------------------------
// VirtualFS
// ---------------------------------------------------------------------------

/// Virtual file system for game sandboxing.
#[derive(Debug, Clone)]
pub struct VirtualFS {
    mappings: HashMap<&'static str, PathMapping>,
    policy: VfsPolicy,
}

impl VirtualFS {
    /// Create a VirtualFS from a [`GamePaths`] instance (default policy).
    pub fn from_game_paths(paths: &GamePaths) -> Self {
        Self::with_policy(
            paths.code_dir().to_path_buf(),
            paths.user_data_dir().to_path_buf(),
            paths.sandbox_cache_dir().to_path_buf(),
            paths.temp_dir().to_path_buf(),
            VfsPolicy::default(),
        )
    }

    /// Create a VirtualFS from a [`GamePaths`] instance with a custom policy.
    pub fn from_game_paths_with_policy(paths: &GamePaths, policy: VfsPolicy) -> Self {
        Self::with_policy(
            paths.code_dir().to_path_buf(),
            paths.user_data_dir().to_path_buf(),
            paths.sandbox_cache_dir().to_path_buf(),
            paths.temp_dir().to_path_buf(),
            policy,
        )
    }

    /// Convenience constructor (default policy).
    pub fn new(
        code_dir: PathBuf,
        user_data_dir: PathBuf,
        cache_dir: PathBuf,
        temp_dir: PathBuf,
    ) -> Self {
        Self::with_policy(
            code_dir,
            user_data_dir,
            cache_dir,
            temp_dir,
            VfsPolicy::default(),
        )
    }

    /// Full constructor with explicit policy.
    pub fn with_policy(
        code_dir: PathBuf,
        user_data_dir: PathBuf,
        cache_dir: PathBuf,
        temp_dir: PathBuf,
        policy: VfsPolicy,
    ) -> Self {
        let mut mappings = HashMap::new();

        mappings.insert(
            "/code",
            Self::make_mapping("/code", code_dir, FilePermissions::READ_ONLY),
        );
        mappings.insert(
            "/user",
            Self::make_mapping("/user", user_data_dir, FilePermissions::READ_WRITE),
        );
        mappings.insert(
            "/cache",
            Self::make_mapping("/cache", cache_dir, FilePermissions::READ_WRITE),
        );
        mappings.insert(
            "/tmp",
            Self::make_mapping("/tmp", temp_dir, FilePermissions::READ_WRITE),
        );

        Self { mappings, policy }
    }

    /// Build a [`PathMapping`], pre-computing the canonical base and
    /// the slash-appended prefix to avoid per-resolve allocations.
    fn make_mapping(
        prefix: &'static str,
        real_path: PathBuf,
        permissions: FilePermissions,
    ) -> PathMapping {
        let canonical_base =
            std::fs::canonicalize(&real_path).unwrap_or_else(|_| normalize_path(&real_path));
        PathMapping {
            virtual_prefix: prefix,
            prefix_with_slash: format!("{}/", prefix),
            real_path,
            canonical_base,
            permissions,
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Resolve a virtual path to a real filesystem path, checking permissions
    /// and enforcing sandbox containment (including symlink verification).
    ///
    /// # Arguments
    /// * `virtual_path` — Path starting with `/code`, `/user`, `/cache`, or `/tmp`.
    /// * `op` — The file operation being performed.
    ///
    /// # Returns
    /// The real filesystem path if allowed, or a [`VfsError`].
    pub fn resolve(&self, virtual_path: &str, op: FileOp) -> Result<PathBuf, VfsError> {
        // ---- Phase 0: input sanitization ----
        Self::validate_input(virtual_path)?;

        // ---- Phase 1: find mapping & check permissions ----
        let mapping = self
            .mappings
            .iter()
            .find(|(prefix, m)| {
                virtual_path == **prefix || virtual_path.starts_with(&m.prefix_with_slash)
            })
            .map(|(_, m)| m)
            .ok_or(VfsError::PathNotAllowed)?;

        if !mapping.permissions.allows(op) {
            return Err(VfsError::PermissionDenied);
        }

        // ---- Phase 2: build real path & textual normalization ----
        let relative = virtual_path
            .strip_prefix(mapping.virtual_prefix)
            .unwrap_or("")
            .trim_start_matches('/');

        let real_path = if relative.is_empty() {
            mapping.real_path.clone()
        } else {
            mapping.real_path.join(relative)
        };

        let normalized = normalize_path(&real_path);

        // Fast reject: textual containment check.
        if !normalized.starts_with(&mapping.canonical_base) {
            // The base might have been normalized (not canonical) at creation
            // time because the directory didn't exist yet.  Re-check against
            // the normalized real_path.
            let norm_base = normalize_path(&mapping.real_path);
            if !normalized.starts_with(&norm_base) {
                return Err(VfsError::PathTraversal);
            }
        }

        // ---- Phase 3: filesystem-level symlink verification ----
        self.check_real_path(&normalized, mapping)?;

        Ok(normalized)
    }

    /// Check if a path is within the VFS.
    pub fn is_virtual_path(&self, path: &str) -> bool {
        self.mappings.keys().any(|prefix| path.starts_with(prefix))
    }

    /// Get the virtual path prefixes (for env.js).
    pub fn get_virtual_paths(&self) -> VirtualPaths {
        VirtualPaths {
            user: "/user".to_string(),
            cache: "/cache".to_string(),
            code: "/code".to_string(),
            tmp: "/tmp".to_string(),
        }
    }

    /// Get real path for a virtual directory (without permission check).
    pub fn get_real_path(&self, virtual_prefix: &str) -> Option<&Path> {
        self.mappings
            .get(virtual_prefix)
            .map(|m| m.real_path.as_path())
    }

    /// Get the current policy.
    pub fn policy(&self) -> &VfsPolicy {
        &self.policy
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Reject paths that contain dangerous byte sequences.
    fn validate_input(virtual_path: &str) -> Result<(), VfsError> {
        // Must start with '/'
        if !virtual_path.starts_with('/') {
            return Err(VfsError::InvalidPath);
        }

        for b in virtual_path.bytes() {
            // Reject control characters (including null byte 0x00)
            if b < 0x20 {
                return Err(VfsError::InvalidPath);
            }
            // Reject backslashes — not valid in virtual paths and commonly
            // used in cross-platform traversal attacks.
            if b == b'\\' {
                return Err(VfsError::InvalidPath);
            }
        }

        Ok(())
    }

    /// Filesystem-level path verification.
    ///
    /// If the path (or a prefix of it) exists on disk, we `canonicalize` to
    /// resolve symlinks and re-verify that the result is still inside the
    /// sandbox base.
    fn check_real_path(&self, normalized: &Path, mapping: &PathMapping) -> Result<(), VfsError> {
        if normalized.exists() {
            // --- Path exists: full canonicalize ---
            self.verify_existing_path(normalized, mapping)?;
        } else if normalized != mapping.real_path && normalized != mapping.canonical_base {
            // --- Path does not exist yet (write / create) ---
            // Walk up to the closest existing ancestor and verify it.
            self.verify_nonexistent_path(normalized, mapping)?;
        }
        // If the path *is* the base itself, the textual check was sufficient.
        Ok(())
    }

    /// Canonicalize an existing path and verify containment.
    fn verify_existing_path(&self, path: &Path, mapping: &PathMapping) -> Result<(), VfsError> {
        let canonical = std::fs::canonicalize(path).map_err(|_| VfsError::InvalidPath)?;
        let base_canonical = self.canonical_base_fresh(mapping);

        if !canonical.starts_with(&base_canonical) {
            return Err(VfsError::SymlinkEscape);
        }

        // Policy: deny symlinks in read-only dirs (e.g. /code)
        if self.should_deny_symlinks(mapping) {
            self.check_no_symlinks_in_chain(path, &mapping.real_path)?;
        }

        Ok(())
    }

    /// For a path that doesn't exist, find the deepest existing ancestor,
    /// canonicalize it, and verify that it (and the projected full path)
    /// remain inside the sandbox.
    ///
    /// Three cases:
    ///
    /// 1. **Ancestor is within (or equal to) the canonical base** — the base
    ///    directory exists and the ancestor is deeper.  We canonicalize the
    ///    ancestor and verify the projected path stays inside the base.
    /// 2. **Ancestor is *above* the base** — the base directory doesn't exist
    ///    yet (common during first launch before `ensure_directories`).  The
    ///    textual phase-2 check already verified containment and there can be
    ///    no in-sandbox symlink to exploit, so we accept.
    /// 3. **Ancestor is neither within nor above the base** — a symlink
    ///    somewhere in the chain redirects outside the expected tree.
    fn verify_nonexistent_path(&self, path: &Path, mapping: &PathMapping) -> Result<(), VfsError> {
        let mut current = path.to_path_buf();
        let mut missing_tail: Vec<std::ffi::OsString> = Vec::new();

        // Walk upward until we find an ancestor that exists on disk.
        loop {
            if current.exists() {
                break;
            }
            match current.file_name() {
                Some(name) => {
                    missing_tail.push(name.to_os_string());
                    current = match current.parent() {
                        Some(p) => p.to_path_buf(),
                        // Reached filesystem root without hitting an existing
                        // dir — the textual check suffices.
                        None => return Ok(()),
                    };
                }
                None => return Ok(()),
            }
        }

        // Canonicalize the existing ancestor.
        let canonical_ancestor =
            std::fs::canonicalize(&current).map_err(|_| VfsError::InvalidPath)?;
        let base_canonical = self.canonical_base_fresh(mapping);

        // Case 1: ancestor is within (or equal to) the base.
        if canonical_ancestor.starts_with(&base_canonical) {
            // Reconstruct the intended path with the canonical prefix and
            // normalize to catch remaining `..` (defense in depth).
            let mut projected = canonical_ancestor;
            for component in missing_tail.iter().rev() {
                projected.push(component);
            }
            let projected_normalized = normalize_path(&projected);
            if !projected_normalized.starts_with(&base_canonical) {
                return Err(VfsError::PathTraversal);
            }

            // Policy check on the existing ancestor chain.
            if self.should_deny_symlinks(mapping) {
                self.check_no_symlinks_in_chain(&current, &mapping.real_path)?;
            }

            return Ok(());
        }

        // Case 2: the walk went *past* the base directory itself.
        // This happens when the base doesn't exist yet (first launch before
        // `ensure_directories`).  We verify this by checking that `current`
        // (the actual directory we found on disk, **before** canonicalize) is
        // a textual prefix of the base path — not just the canonical result.
        // This prevents a symlink whose canonical target happens to be an
        // ancestor of the base from being accepted.
        let norm_base = normalize_path(&mapping.real_path);
        if norm_base.starts_with(&current) {
            return Ok(());
        }

        // Case 3: ancestor is in an unrelated subtree — symlink escape.
        Err(VfsError::SymlinkEscape)
    }

    /// Get a fresh canonical base for a mapping.
    ///
    /// If `mapping.real_path` exists we re-canonicalize it (one `realpath`
    /// syscall) so that we stay correct even if the base was created after
    /// VFS construction.  Otherwise we fall back to the cached value.
    #[inline]
    fn canonical_base_fresh(&self, mapping: &PathMapping) -> PathBuf {
        if mapping.real_path.exists() {
            std::fs::canonicalize(&mapping.real_path)
                .unwrap_or_else(|_| mapping.canonical_base.clone())
        } else {
            mapping.canonical_base.clone()
        }
    }

    /// Whether symlinks should be rejected for this mapping.
    ///
    /// Symlinks are denied in read-only directories (controlled by
    /// `deny_symlinks_in_code_dir`) **and** in writable directories
    /// (controlled by `deny_symlinks_in_writable_dirs`).  Both default
    /// to `true`, closing the sandbox escape via writable-dir symlinks.
    #[inline]
    fn should_deny_symlinks(&self, mapping: &PathMapping) -> bool {
        if mapping.permissions.write {
            self.policy.deny_symlinks_in_writable_dirs
        } else {
            self.policy.deny_symlinks_in_code_dir
        }
    }

    /// Walk every path component between `base` and `full_path` and reject
    /// if any component is a symbolic link.
    ///
    /// Uses `symlink_metadata` (lstat) which does **not** follow symlinks,
    /// so we can detect them.
    fn check_no_symlinks_in_chain(&self, full_path: &Path, base: &Path) -> Result<(), VfsError> {
        let relative = full_path
            .strip_prefix(base)
            .map_err(|_| VfsError::PathTraversal)?;

        let mut current = base.to_path_buf();
        for component in relative.components() {
            current.push(component);
            if current.exists() {
                let meta =
                    std::fs::symlink_metadata(&current).map_err(|_| VfsError::InvalidPath)?;
                if meta.file_type().is_symlink() {
                    return Err(VfsError::SymlinkNotAllowed);
                }
            }
        }
        Ok(())
    }
}

/// Virtual path constants exposed to JavaScript.
#[derive(Debug, Clone)]
pub struct VirtualPaths {
    pub user: String,
    pub cache: String,
    pub code: String,
    pub tmp: String,
}

// ---------------------------------------------------------------------------
// normalize_path (pure textual, no syscalls)
// ---------------------------------------------------------------------------

/// Normalize a path by resolving `.` and `..` components **textually**.
///
/// This intentionally does **not** follow symlinks.  It is used as a fast
/// first-pass check before the more expensive `canonicalize` call.
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();

    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            c => components.push(c),
        }
    }

    components.iter().collect()
}

// ---------------------------------------------------------------------------
// Shared security helpers (used by both VirtualFS and MountTable)
// ---------------------------------------------------------------------------

/// Verify that a resolved real path is contained within `base_dir` by
/// canonicalizing (resolving symlinks) and re-checking `starts_with`.
///
/// If `deny_symlinks` is true, every component between `base_dir` and the
/// resolved path is checked via `symlink_metadata` (lstat) and rejected if
/// it is a symbolic link.
///
/// For paths that don't exist yet, the closest existing ancestor is
/// canonicalized and the remaining tail is projected, matching the
/// [`VirtualFS`] non-existent-path verification logic.
pub(crate) fn verify_path_containment(
    real_path: &Path,
    base_dir: &Path,
    deny_symlinks: bool,
) -> Result<(), VfsError> {
    let normalized = normalize_path(real_path);
    let norm_base = normalize_path(base_dir);

    // Fast textual reject.
    if !normalized.starts_with(&norm_base) {
        return Err(VfsError::PathTraversal);
    }

    // If the path *is* the base itself, textual check suffices.
    if normalized == norm_base {
        return Ok(());
    }

    if normalized.exists() {
        // ---- Path exists: full canonicalize ----
        let canonical = std::fs::canonicalize(&normalized).map_err(|_| VfsError::InvalidPath)?;
        let canonical_base = canonical_base_for(base_dir);
        if !canonical.starts_with(&canonical_base) {
            return Err(VfsError::SymlinkEscape);
        }
        if deny_symlinks {
            check_no_symlinks_in_chain(&normalized, base_dir)?;
        }
    } else {
        // ---- Path doesn't exist yet: walk to closest ancestor ----
        verify_nonexistent_containment(&normalized, base_dir, deny_symlinks)?;
    }

    Ok(())
}

/// Canonicalize `base_dir` if it exists, else normalize textually.
fn canonical_base_for(base_dir: &Path) -> PathBuf {
    if base_dir.exists() {
        std::fs::canonicalize(base_dir).unwrap_or_else(|_| normalize_path(base_dir))
    } else {
        normalize_path(base_dir)
    }
}

/// Verify a non-existent path by walking up to the closest existing
/// ancestor, canonicalizing it, and projecting the remaining tail.
fn verify_nonexistent_containment(
    path: &Path,
    base_dir: &Path,
    deny_symlinks: bool,
) -> Result<(), VfsError> {
    let mut current = path.to_path_buf();
    let mut missing_tail: Vec<std::ffi::OsString> = Vec::new();

    loop {
        if current.exists() {
            break;
        }
        match current.file_name() {
            Some(name) => {
                missing_tail.push(name.to_os_string());
                current = match current.parent() {
                    Some(p) => p.to_path_buf(),
                    None => return Ok(()),
                };
            }
            None => return Ok(()),
        }
    }

    let canonical_ancestor = std::fs::canonicalize(&current).map_err(|_| VfsError::InvalidPath)?;
    let canonical_base = canonical_base_for(base_dir);

    if canonical_ancestor.starts_with(&canonical_base) {
        let mut projected = canonical_ancestor;
        for component in missing_tail.iter().rev() {
            projected.push(component);
        }
        let projected_normalized = normalize_path(&projected);
        if !projected_normalized.starts_with(&canonical_base) {
            return Err(VfsError::PathTraversal);
        }
        if deny_symlinks {
            check_no_symlinks_in_chain(&current, base_dir)?;
        }
        return Ok(());
    }

    // Walk went past base — base doesn't exist yet.
    let norm_base = normalize_path(base_dir);
    if norm_base.starts_with(&current) {
        return Ok(());
    }

    Err(VfsError::SymlinkEscape)
}

/// Walk every component between `base` and `full_path`, reject if any is
/// a symbolic link.  Uses `symlink_metadata` (lstat).
pub(crate) fn check_no_symlinks_in_chain(full_path: &Path, base: &Path) -> Result<(), VfsError> {
    let relative = full_path
        .strip_prefix(base)
        .map_err(|_| VfsError::PathTraversal)?;

    let mut current = base.to_path_buf();
    for component in relative.components() {
        current.push(component);
        if current.exists() {
            let meta = std::fs::symlink_metadata(&current).map_err(|_| VfsError::InvalidPath)?;
            if meta.file_type().is_symlink() {
                return Err(VfsError::SymlinkNotAllowed);
            }
        }
    }
    Ok(())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn test_vfs() -> VirtualFS {
        VirtualFS::new(
            PathBuf::from("/data/games/test/code"),
            PathBuf::from("/data/games/test/user"),
            PathBuf::from("/data/games/test/cache"),
            PathBuf::from("/data/games/test/tmp"),
        )
    }

    /// The per-game cache root holds runtime state, not game data: the install
    /// record and `.mpkg` files a later session mounts as `/code`, and the
    /// staging directories an in-flight install renames from. A mapping that
    /// contains the root hands the game write access to all of it.
    #[test]
    fn no_vfs_mapping_contains_the_runtime_cache_root() {
        let base = std::env::temp_dir().join("migo_vfs_runtime_state_containment");
        let paths = GamePaths::new(base.join("files"), base.join("cache"), "game-a").unwrap();
        let vfs = VirtualFS::from_game_paths(&paths);

        let runtime_root = paths.cache_dir();
        for mapping in vfs.mappings.values() {
            assert!(
                !runtime_root.starts_with(&mapping.real_path),
                "the cache root {} is writable through {}, which exposes the install \
                 record at {}",
                runtime_root.display(),
                mapping.virtual_prefix,
                crate::vfs::mount::package_store_dir(runtime_root).display(),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Original tests (kept for regression)
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_code() {
        let vfs = test_vfs();

        let result = vfs.resolve("/code/game.js", FileOp::Read);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            PathBuf::from("/data/games/test/code/game.js")
        );
    }

    #[test]
    fn test_code_write_denied() {
        let vfs = test_vfs();

        let result = vfs.resolve("/code/game.js", FileOp::Write);
        assert_eq!(result.unwrap_err(), VfsError::PermissionDenied);
    }

    #[test]
    fn test_resolve_user() {
        let vfs = test_vfs();

        let result = vfs.resolve("/user/save.json", FileOp::Write);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            PathBuf::from("/data/games/test/user/save.json")
        );
    }

    #[test]
    fn test_path_traversal() {
        let vfs = test_vfs();

        let result = vfs.resolve("/user/../../../etc/passwd", FileOp::Read);
        assert_eq!(result.unwrap_err(), VfsError::PathTraversal);
    }

    #[test]
    fn test_invalid_prefix() {
        let vfs = test_vfs();

        let result = vfs.resolve("/invalid/path", FileOp::Read);
        assert_eq!(result.unwrap_err(), VfsError::PathNotAllowed);
    }

    // -----------------------------------------------------------------------
    // Input sanitization
    // -----------------------------------------------------------------------

    #[test]
    fn test_reject_null_byte() {
        let vfs = test_vfs();
        let result = vfs.resolve("/user/save\0.json", FileOp::Read);
        assert_eq!(result.unwrap_err(), VfsError::InvalidPath);
    }

    #[test]
    fn test_reject_backslash() {
        let vfs = test_vfs();
        let result = vfs.resolve("/user\\..\\..\\etc\\passwd", FileOp::Read);
        assert_eq!(result.unwrap_err(), VfsError::InvalidPath);
    }

    #[test]
    fn test_reject_control_chars() {
        let vfs = test_vfs();
        // Tab character
        let result = vfs.resolve("/user/save\t.json", FileOp::Read);
        assert_eq!(result.unwrap_err(), VfsError::InvalidPath);

        // Newline
        let result = vfs.resolve("/user/save\n.json", FileOp::Read);
        assert_eq!(result.unwrap_err(), VfsError::InvalidPath);
    }

    #[test]
    fn test_reject_no_leading_slash() {
        let vfs = test_vfs();
        let result = vfs.resolve("user/save.json", FileOp::Read);
        assert_eq!(result.unwrap_err(), VfsError::InvalidPath);
    }

    // -----------------------------------------------------------------------
    // Path traversal variants
    // -----------------------------------------------------------------------

    #[test]
    fn test_traversal_dot_dot_slash() {
        let vfs = test_vfs();

        // Various encodings of ../
        assert_eq!(
            vfs.resolve("/user/../../../etc/passwd", FileOp::Read)
                .unwrap_err(),
            VfsError::PathTraversal
        );
        assert_eq!(
            vfs.resolve("/cache/../../secret", FileOp::Read)
                .unwrap_err(),
            VfsError::PathTraversal
        );
        assert_eq!(
            vfs.resolve("/tmp/./../../../etc/shadow", FileOp::Read)
                .unwrap_err(),
            VfsError::PathTraversal
        );
    }

    #[test]
    fn test_traversal_deep_nested() {
        let vfs = test_vfs();
        // Try to escape by going deep then backing out
        let result = vfs.resolve("/user/a/b/c/d/../../../../../../etc/passwd", FileOp::Read);
        assert_eq!(result.unwrap_err(), VfsError::PathTraversal);
    }

    #[test]
    fn test_traversal_dot_dot_at_boundary() {
        let vfs = test_vfs();
        // Exactly one level up from /user → should still be inside test/ but not in /user
        let result = vfs.resolve("/user/../code/game.js", FileOp::Read);
        assert_eq!(result.unwrap_err(), VfsError::PathTraversal);
    }

    #[test]
    fn test_resolve_with_dot_current_dir() {
        let vfs = test_vfs();
        // /user/./save.json should be fine
        let result = vfs.resolve("/user/./save.json", FileOp::Read);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            PathBuf::from("/data/games/test/user/save.json")
        );
    }

    #[test]
    fn test_resolve_bare_prefix() {
        let vfs = test_vfs();

        // Bare /code without trailing path
        let result = vfs.resolve("/code", FileOp::Read);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("/data/games/test/code"));
    }

    // -----------------------------------------------------------------------
    // Policy: symlinks denied in code dir
    // -----------------------------------------------------------------------

    #[test]
    fn test_policy_default() {
        let vfs = test_vfs();
        assert!(vfs.policy().deny_symlinks_in_code_dir);
    }

    #[test]
    fn test_policy_custom_allows_symlinks() {
        let vfs = VirtualFS::with_policy(
            PathBuf::from("/data/games/test/code"),
            PathBuf::from("/data/games/test/user"),
            PathBuf::from("/data/games/test/cache"),
            PathBuf::from("/data/games/test/tmp"),
            VfsPolicy {
                deny_symlinks_in_code_dir: false,
                deny_symlinks_in_writable_dirs: false,
            },
        );
        assert!(!vfs.policy().deny_symlinks_in_code_dir);
    }

    // -----------------------------------------------------------------------
    // VfsError display
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_display() {
        assert_eq!(
            VfsError::SymlinkEscape.to_string(),
            "Symlink resolves outside sandbox"
        );
        assert_eq!(
            VfsError::SymlinkNotAllowed.to_string(),
            "Symlinks not allowed in this directory"
        );
    }

    // -----------------------------------------------------------------------
    // Filesystem-level tests (real temp dirs, real symlinks)
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    mod fs_tests {
        use super::*;
        use std::fs;

        /// Create a unique temp directory for a test.
        fn make_test_dir(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!("migo_vfs_test_{}", name));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            dir
        }

        /// Create a VFS rooted in a real temp directory.
        fn make_real_vfs(base: &Path) -> VirtualFS {
            let code = base.join("code");
            let user = base.join("user");
            let cache = base.join("cache");
            let tmp = base.join("tmp");

            fs::create_dir_all(&code).unwrap();
            fs::create_dir_all(&user).unwrap();
            fs::create_dir_all(&cache).unwrap();
            fs::create_dir_all(&tmp).unwrap();

            VirtualFS::new(code, user, cache, tmp)
        }

        /// Create a VFS with a custom policy.
        fn make_real_vfs_with_policy(base: &Path, policy: VfsPolicy) -> VirtualFS {
            let code = base.join("code");
            let user = base.join("user");
            let cache = base.join("cache");
            let tmp = base.join("tmp");

            fs::create_dir_all(&code).unwrap();
            fs::create_dir_all(&user).unwrap();
            fs::create_dir_all(&cache).unwrap();
            fs::create_dir_all(&tmp).unwrap();

            VirtualFS::with_policy(code, user, cache, tmp, policy)
        }

        // -- Symlink escape: read --

        #[test]
        fn test_symlink_escape_read_user() {
            let base = make_test_dir("symlink_escape_read_user");
            let vfs = make_real_vfs(&base);

            // Create a symlink inside /user pointing outside the sandbox.
            let target = std::env::temp_dir(); // outside sandbox
            let link = base.join("user").join("evil_link");
            std::os::unix::fs::symlink(&target, &link).unwrap();

            let result = vfs.resolve("/user/evil_link", FileOp::Read);
            assert_eq!(result.unwrap_err(), VfsError::SymlinkEscape);

            // Cleanup
            let _ = fs::remove_dir_all(&base);
        }

        #[test]
        fn test_symlink_escape_read_file_through_link() {
            let base = make_test_dir("symlink_escape_read_through");
            let vfs = make_real_vfs(&base);

            // /user/link -> /tmp (system /tmp, outside sandbox)
            let link = base.join("user").join("escape");
            std::os::unix::fs::symlink("/tmp", &link).unwrap();

            // Try to read a file through the symlinked directory
            let result = vfs.resolve("/user/escape/some_file", FileOp::Read);
            // The ancestor /user/escape exists and canonicalizes to /tmp,
            // which is outside the sandbox → SymlinkEscape
            assert_eq!(result.unwrap_err(), VfsError::SymlinkEscape);

            let _ = fs::remove_dir_all(&base);
        }

        // -- Symlink escape: write --

        #[test]
        fn test_symlink_escape_write() {
            let base = make_test_dir("symlink_escape_write");
            let vfs = make_real_vfs(&base);

            // /user/escape -> /tmp (system)
            let link = base.join("user").join("escape");
            std::os::unix::fs::symlink("/tmp", &link).unwrap();

            let result = vfs.resolve("/user/escape/pwned.txt", FileOp::Write);
            assert_eq!(result.unwrap_err(), VfsError::SymlinkEscape);

            let _ = fs::remove_dir_all(&base);
        }

        // -- Symlink within writable dir (denied by default policy) --
        //
        // Even if the symlink target is inside the sandbox, symlinks in
        // writable directories are denied by default to prevent TOCTOU
        // and sandbox-escape attacks.

        #[test]
        fn test_symlink_within_writable_dir_denied_by_default() {
            let base = make_test_dir("symlink_within_sandbox");
            let vfs = make_real_vfs(&base);

            // Create a real file and a symlink to it *within* /user
            let real_file = base.join("user").join("real.txt");
            fs::write(&real_file, "hello").unwrap();

            let link = base.join("user").join("alias.txt");
            std::os::unix::fs::symlink(&real_file, &link).unwrap();

            // Default policy now denies symlinks in writable dirs too
            let result = vfs.resolve("/user/alias.txt", FileOp::Read);
            assert_eq!(result.unwrap_err(), VfsError::SymlinkNotAllowed);

            let _ = fs::remove_dir_all(&base);
        }

        // -- Symlink within writable dir allowed when policy is relaxed --

        #[test]
        fn test_symlink_within_writable_dir_allowed_when_policy_off() {
            let base = make_test_dir("symlink_writable_allowed");
            let policy = VfsPolicy {
                deny_symlinks_in_code_dir: true,
                deny_symlinks_in_writable_dirs: false,
            };
            let vfs = make_real_vfs_with_policy(&base, policy);

            let real_file = base.join("user").join("real.txt");
            fs::write(&real_file, "hello").unwrap();

            let link = base.join("user").join("alias.txt");
            std::os::unix::fs::symlink(&real_file, &link).unwrap();

            // With writable-dir symlinks allowed, this should pass
            let result = vfs.resolve("/user/alias.txt", FileOp::Read);
            assert!(result.is_ok());

            let _ = fs::remove_dir_all(&base);
        }

        // -- Symlink in code_dir (denied by default policy) --

        #[test]
        fn test_symlink_in_code_dir_denied_by_policy() {
            let base = make_test_dir("symlink_code_dir_denied");
            let vfs = make_real_vfs(&base);

            // Create a real file within code and a symlink to it
            let real_file = base.join("code").join("real.js");
            fs::write(&real_file, "// real").unwrap();

            let link = base.join("code").join("link.js");
            std::os::unix::fs::symlink(&real_file, &link).unwrap();

            // Default policy denies symlinks in code_dir, even if the
            // target is within the sandbox.
            let result = vfs.resolve("/code/link.js", FileOp::Read);
            assert_eq!(result.unwrap_err(), VfsError::SymlinkNotAllowed);

            let _ = fs::remove_dir_all(&base);
        }

        #[test]
        fn test_symlink_in_code_dir_allowed_when_policy_off() {
            let base = make_test_dir("symlink_code_dir_allowed");
            let policy = VfsPolicy {
                deny_symlinks_in_code_dir: false,
                deny_symlinks_in_writable_dirs: false,
            };
            let vfs = make_real_vfs_with_policy(&base, policy);

            let real_file = base.join("code").join("real.js");
            fs::write(&real_file, "// real").unwrap();

            let link = base.join("code").join("link.js");
            std::os::unix::fs::symlink(&real_file, &link).unwrap();

            // Policy is off — should be allowed (target is within sandbox)
            let result = vfs.resolve("/code/link.js", FileOp::Read);
            assert!(result.is_ok());

            let _ = fs::remove_dir_all(&base);
        }

        #[test]
        fn test_code_dir_symlink_escaping_sandbox() {
            let base = make_test_dir("symlink_code_escape");
            let vfs = make_real_vfs(&base);

            // Symlink in code_dir pointing outside sandbox
            let link = base.join("code").join("evil.js");
            std::os::unix::fs::symlink("/etc/passwd", &link).unwrap();

            let result = vfs.resolve("/code/evil.js", FileOp::Read);
            // Could be SymlinkEscape or SymlinkNotAllowed — both are rejections.
            // With default policy, the symlink chain check fires after the
            // canonicalize containment check.
            let err = result.unwrap_err();
            assert!(
                err == VfsError::SymlinkNotAllowed || err == VfsError::SymlinkEscape,
                "expected SymlinkNotAllowed or SymlinkEscape, got {:?}",
                err
            );

            let _ = fs::remove_dir_all(&base);
        }

        // -- Non-existent path with symlinked ancestor --

        #[test]
        fn test_nonexistent_path_with_symlinked_ancestor() {
            let base = make_test_dir("nonexist_symlink_ancestor");
            let vfs = make_real_vfs(&base);

            // /user/escape -> /tmp (system)
            let link = base.join("user").join("escape");
            std::os::unix::fs::symlink("/tmp", &link).unwrap();

            // /user/escape/subdir/file.txt — file doesn't exist, but
            // ancestor /user/escape is a symlink to /tmp
            let result = vfs.resolve("/user/escape/subdir/file.txt", FileOp::Create);
            assert_eq!(result.unwrap_err(), VfsError::SymlinkEscape);

            let _ = fs::remove_dir_all(&base);
        }

        // -- Normal operations on real dirs --

        #[test]
        fn test_real_dir_read_write_ok() {
            let base = make_test_dir("real_dir_ok");
            let vfs = make_real_vfs(&base);

            // Write
            let result = vfs.resolve("/user/data.json", FileOp::Write);
            assert!(result.is_ok());

            // Read existing file
            let file = base.join("user").join("readme.txt");
            fs::write(&file, "hi").unwrap();
            let result = vfs.resolve("/user/readme.txt", FileOp::Read);
            assert!(result.is_ok());

            // Create nested dir
            let result = vfs.resolve("/cache/a/b/c.txt", FileOp::Create);
            assert!(result.is_ok());

            let _ = fs::remove_dir_all(&base);
        }

        // -- Deeply nested symlink in a chain --

        #[test]
        fn test_deep_nested_symlink_escape() {
            let base = make_test_dir("deep_symlink");
            let vfs = make_real_vfs(&base);

            // /user/a/b/c where b -> /
            let a_dir = base.join("user").join("a");
            fs::create_dir_all(&a_dir).unwrap();
            let b_link = a_dir.join("b");
            std::os::unix::fs::symlink("/", &b_link).unwrap();

            let result = vfs.resolve("/user/a/b/etc/passwd", FileOp::Read);
            assert_eq!(result.unwrap_err(), VfsError::SymlinkEscape);

            let _ = fs::remove_dir_all(&base);
        }

        // -- P3-2: Writable-dir symlink tests (validates P0-2 fix) --

        #[test]
        fn test_writable_dir_symlink_escape_outside() {
            let base = make_test_dir("writable_symlink_outside");
            let vfs = make_real_vfs(&base);

            let outside = base.join("outside_target");
            fs::create_dir_all(&outside).unwrap();
            let link = base.join("user").join("escape_link");
            std::os::unix::fs::symlink(&outside, &link).unwrap();

            let result = vfs.resolve("/user/escape_link/file.txt", FileOp::Write);
            assert!(
                result.is_err(),
                "Symlink in writable dir to outside must be blocked"
            );

            let _ = fs::remove_dir_all(&base);
        }

        #[test]
        fn test_writable_dir_symlink_to_system() {
            let base = make_test_dir("writable_symlink_system");
            let vfs = make_real_vfs(&base);

            let link = base.join("cache").join("etc_link");
            std::os::unix::fs::symlink("/etc", &link).unwrap();

            let result = vfs.resolve("/cache/etc_link/passwd", FileOp::Read);
            let err = result.unwrap_err();
            assert!(
                err == VfsError::SymlinkNotAllowed || err == VfsError::SymlinkEscape,
                "expected SymlinkNotAllowed or SymlinkEscape, got {:?}",
                err
            );

            let _ = fs::remove_dir_all(&base);
        }

        #[test]
        fn test_tmp_dir_symlink_blocked() {
            let base = make_test_dir("tmp_symlink");
            let vfs = make_real_vfs(&base);

            let link = base.join("tmp").join("sneaky");
            std::os::unix::fs::symlink("/", &link).unwrap();

            let result = vfs.resolve("/tmp/sneaky/etc/shadow", FileOp::Read);
            let err = result.unwrap_err();
            assert!(
                err == VfsError::SymlinkNotAllowed || err == VfsError::SymlinkEscape,
                "expected rejection, got {:?}",
                err
            );

            let _ = fs::remove_dir_all(&base);
        }

        #[test]
        fn test_canonicalize_containment_normal_file() {
            let base = make_test_dir("canon_normal");
            let vfs = make_real_vfs(&base);

            let file = base.join("user").join("normal.txt");
            fs::write(&file, "safe content").unwrap();

            let result = vfs.resolve("/user/normal.txt", FileOp::Read);
            assert!(result.is_ok());

            let _ = fs::remove_dir_all(&base);
        }

        #[test]
        fn test_all_four_dirs_reject_symlinks_by_default() {
            let base = make_test_dir("all_dirs_symlink");
            let vfs = make_real_vfs(&base);

            let outside = base.join("outside_all");
            fs::create_dir_all(&outside).unwrap();

            for dir_name in &["code", "user", "cache", "tmp"] {
                let link = base.join(dir_name).join("link_out");
                let _ = std::fs::remove_file(&link);
                std::os::unix::fs::symlink(&outside, &link).unwrap();

                let vpath = format!("/{}/link_out/x.txt", dir_name);
                let op = if *dir_name == "code" {
                    FileOp::Read
                } else {
                    FileOp::Write
                };
                let result = vfs.resolve(&vpath, op);
                assert!(
                    result.is_err(),
                    "Symlink escape in /{} should be blocked, but got Ok",
                    dir_name
                );
            }

            let _ = fs::remove_dir_all(&base);
        }
    }
}
