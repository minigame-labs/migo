//! Virtual File System for game sandboxing.
//!
//! Provides path isolation and permission management for games.
//! Each game has its own virtual paths that map to real directories:
//!
//! - `/user` → User data (saves, preferences) - read/write
//! - `/cache` → Cache files - read/write  
//! - `/code` → Game code - read only
//! - `/tmp` → Temporary files - read/write
//!
//! # Example
//!
//! ```rust,ignore
//! use shared::vfs::{GamePaths, VirtualFS};
//!
//! // Create game paths from base directories
//! let paths = GamePaths::new("/data/files", "/data/cache", "my-game")?;
//! paths.ensure_directories()?;
//!
//! // Create VFS for the game
//! let vfs = VirtualFS::from_game_paths(&paths);
//!
//! // Resolve virtual path to real path
//! let real = vfs.resolve("/user/save.json", FileOp::Write)?;
//! ```

pub mod game_paths;

pub use game_paths::{GamePaths, GamePathStrings, GamePathError, validate_game_id};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// Mapping from virtual path prefix to real path.
#[derive(Debug, Clone)]
pub struct PathMapping {
    pub virtual_prefix: &'static str,
    pub real_path: PathBuf,
    pub permissions: FilePermissions,
}

/// Error types for VFS operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VfsError {
    /// Path is not within any allowed virtual directory
    PathNotAllowed,
    /// Operation is not permitted for this path
    PermissionDenied,
    /// Path traversal attack detected (e.g., ../../../etc/passwd)
    PathTraversal,
    /// Invalid path format
    InvalidPath,
}

impl std::fmt::Display for VfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VfsError::PathNotAllowed => write!(f, "Path is not within any allowed virtual directory"),
            VfsError::PermissionDenied => write!(f, "Permission denied for this operation"),
            VfsError::PathTraversal => write!(f, "Path traversal detected"),
            VfsError::InvalidPath => write!(f, "Invalid path format"),
        }
    }
}

impl std::error::Error for VfsError {}

/// Virtual file system for game sandboxing.
#[derive(Debug, Clone)]
pub struct VirtualFS {
    mappings: HashMap<&'static str, PathMapping>,
}

impl VirtualFS {
    /// Create a VirtualFS from a GamePaths instance.
    ///
    /// This is the recommended way to create a VirtualFS.
    pub fn from_game_paths(paths: &GamePaths) -> Self {
        Self::new(
            paths.code_dir().to_path_buf(),
            paths.user_data_dir().to_path_buf(),
            paths.cache_dir().to_path_buf(),
            paths.temp_dir().to_path_buf(),
        )
    }

    /// Create a new VirtualFS with the given paths.
    ///
    /// # Arguments
    /// * `code_dir` - Game code directory (read-only, maps to /code)
    /// * `user_data_dir` - User data directory (read-write, maps to /user)
    /// * `cache_dir` - Cache directory (read-write, maps to /cache)
    /// * `temp_dir` - Temporary directory (read-write, maps to /tmp)
    pub fn new(
        code_dir: PathBuf,
        user_data_dir: PathBuf,
        cache_dir: PathBuf,
        temp_dir: PathBuf,
    ) -> Self {
        let mut mappings = HashMap::new();

        mappings.insert(
            "/code",
            PathMapping {
                virtual_prefix: "/code",
                real_path: code_dir,
                permissions: FilePermissions::READ_ONLY,
            },
        );

        mappings.insert(
            "/user",
            PathMapping {
                virtual_prefix: "/user",
                real_path: user_data_dir,
                permissions: FilePermissions::READ_WRITE,
            },
        );

        mappings.insert(
            "/cache",
            PathMapping {
                virtual_prefix: "/cache",
                real_path: cache_dir,
                permissions: FilePermissions::READ_WRITE,
            },
        );

        mappings.insert(
            "/tmp",
            PathMapping {
                virtual_prefix: "/tmp",
                real_path: temp_dir,
                permissions: FilePermissions::READ_WRITE,
            },
        );

        Self { mappings }
    }

    /// Resolve a virtual path to a real path, checking permissions.
    ///
    /// # Arguments
    /// * `virtual_path` - Path starting with /code, /user, /cache, or /tmp
    /// * `op` - The file operation being performed
    ///
    /// # Returns
    /// The real filesystem path if allowed, or a VfsError.
    pub fn resolve(&self, virtual_path: &str, op: FileOp) -> Result<PathBuf, VfsError> {
        // Find matching mapping
        let mapping = self
            .mappings
            .iter()
            .find(|(prefix, _)| virtual_path.starts_with(*prefix))
            .map(|(_, m)| m)
            .ok_or(VfsError::PathNotAllowed)?;

        // Check permissions
        if !mapping.permissions.allows(op) {
            return Err(VfsError::PermissionDenied);
        }

        // Build real path
        let relative = virtual_path
            .strip_prefix(mapping.virtual_prefix)
            .unwrap_or("")
            .trim_start_matches('/');

        let real_path = if relative.is_empty() {
            mapping.real_path.clone()
        } else {
            mapping.real_path.join(relative)
        };

        // Path traversal protection
        self.check_path_traversal(&real_path, &mapping.real_path)?;

        Ok(real_path)
    }

    /// Check for path traversal attacks.
    fn check_path_traversal(&self, path: &Path, base: &Path) -> Result<(), VfsError> {
        // Normalize the path (resolve .. and .)
        let normalized = normalize_path(path);

        // Check if normalized path is still within base directory
        if !normalized.starts_with(base) {
            return Err(VfsError::PathTraversal);
        }

        Ok(())
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
    /// Used for directory existence checks.
    pub fn get_real_path(&self, virtual_prefix: &str) -> Option<&Path> {
        self.mappings.get(virtual_prefix).map(|m| m.real_path.as_path())
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

/// Normalize a path by resolving . and .. components.
fn normalize_path(path: &Path) -> PathBuf {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_vfs() -> VirtualFS {
        VirtualFS::new(
            PathBuf::from("/data/games/test/code"),
            PathBuf::from("/data/games/test/user"),
            PathBuf::from("/data/games/test/cache"),
            PathBuf::from("/data/games/test/tmp"),
        )
    }

    #[test]
    fn test_resolve_code() {
        let vfs = test_vfs();
        
        let result = vfs.resolve("/code/game.js", FileOp::Read);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("/data/games/test/code/game.js"));
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
        assert_eq!(result.unwrap(), PathBuf::from("/data/games/test/user/save.json"));
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
}
