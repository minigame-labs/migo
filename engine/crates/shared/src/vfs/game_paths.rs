//! Game paths generation and validation.
//!
//! This module provides cross-platform game path management.
//! All path generation logic is centralized here to ensure consistency
//! across different platforms (Android, iOS, Desktop).

use std::path::{Path, PathBuf};

/// Error types for game path operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GamePathError {
    /// Game ID is invalid (wrong format or length)
    InvalidGameId(String),
    /// Base directory is invalid
    InvalidBaseDir,
}

impl std::fmt::Display for GamePathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GamePathError::InvalidGameId(id) => {
                write!(
                    f,
                    "Invalid game ID '{}': must be 1-64 alphanumeric, underscore or hyphen",
                    id
                )
            }
            GamePathError::InvalidBaseDir => write!(f, "Invalid base directory"),
        }
    }
}

impl std::error::Error for GamePathError {}

/// Validate a game ID.
///
/// # Rules
/// - Length: 1-64 characters
/// - Allowed characters: a-z, A-Z, 0-9, _, -
///
/// # Examples
/// ```
/// use shared::vfs::game_paths::validate_game_id;
///
/// assert!(validate_game_id("my-game").is_ok());
/// assert!(validate_game_id("game_123").is_ok());
/// assert!(validate_game_id("").is_err());
/// assert!(validate_game_id("../hack").is_err());
/// ```
pub fn validate_game_id(game_id: &str) -> Result<(), GamePathError> {
    // Hand-rolled validation replaces regex crate dependency for this single use.
    // Allowed: a-z, A-Z, 0-9, underscore, hyphen. Length: 1-64.
    if game_id.is_empty() || game_id.len() > 64 {
        return Err(GamePathError::InvalidGameId(game_id.to_string()));
    }
    for b in game_id.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' => {}
            _ => return Err(GamePathError::InvalidGameId(game_id.to_string())),
        }
    }
    Ok(())
}

/// Paths for a specific game instance.
///
/// All paths are absolute and isolated per game.
/// This struct is created by the runtime and can be serialized
/// back to the platform layer for cleanup operations.
#[derive(Debug, Clone)]
pub struct GamePaths {
    /// Game identifier
    game_id: String,
    /// Game code directory (read-only for the game)
    code_dir: PathBuf,
    /// User data directory (persistent, survives app updates)
    user_data_dir: PathBuf,
    /// Cache directory (may be cleared by system)
    cache_dir: PathBuf,
    /// Game-writable subtree of the cache directory (mapped to `/cache`)
    sandbox_cache_dir: PathBuf,
    /// Temporary directory (cleared when session ends)
    temp_dir: PathBuf,
}

impl GamePaths {
    /// Base subdirectory for all games.
    const GAMES_SUBDIR: &'static str = "migo/games";

    /// Create game paths from base directories and game ID.
    ///
    /// # Arguments
    /// * `files_dir` - Platform's persistent files directory
    ///   - Android: `Context.getFilesDir()`
    ///   - iOS: `NSDocumentDirectory`
    ///   - Desktop: `~/.local/share/app/` or `%APPDATA%/app/`
    /// * `cache_dir` - Platform's cache directory
    ///   - Android: `Context.getCacheDir()`
    ///   - iOS: `NSCachesDirectory`
    ///   - Desktop: `~/.cache/app/` or `%TEMP%/app/`
    /// * `game_id` - Unique game identifier
    ///
    /// # Returns
    /// `GamePaths` on success, `GamePathError` if game_id is invalid.
    pub fn new(
        files_dir: impl AsRef<Path>,
        cache_dir: impl AsRef<Path>,
        game_id: &str,
    ) -> Result<Self, GamePathError> {
        validate_game_id(game_id)?;

        let files_base = files_dir.as_ref().join(Self::GAMES_SUBDIR).join(game_id);
        let cache_base = cache_dir.as_ref().join(Self::GAMES_SUBDIR).join(game_id);

        Ok(Self {
            game_id: game_id.to_string(),
            code_dir: files_base.join("code"),
            user_data_dir: files_base.join("user_data"),
            cache_dir: cache_base.clone(),
            sandbox_cache_dir: cache_base.join("sandbox"),
            temp_dir: cache_base.join("tmp"),
        })
    }

    /// Get the game ID.
    pub fn game_id(&self) -> &str {
        &self.game_id
    }

    /// Get the code directory path (read-only for game).
    pub fn code_dir(&self) -> &Path {
        &self.code_dir
    }

    /// Persistent receipt for the sealed code tree. It is deliberately a
    /// sibling of `/code`: game VFS access cannot reach it and Android cache
    /// eviction does not discard the verification result.
    pub fn integrity_receipt_path(&self) -> PathBuf {
        self.code_dir
            .with_file_name("code.integrity-receipt-v1.json")
    }

    /// Advisory process lock serializing receipt promotion for this game.
    pub fn integrity_lock_path(&self) -> PathBuf {
        self.integrity_receipt_path().with_extension("lock")
    }

    /// Get the user data directory path (persistent).
    pub fn user_data_dir(&self) -> &Path {
        &self.user_data_dir
    }

    /// Get the cache directory path.
    ///
    /// This is the per-game cache **root**, and it is runtime state rather than
    /// game data: the package install store, install staging directories and the
    /// derived-asset cache all live directly under it. It must never be the
    /// target of a VFS mapping — the game-writable cache is
    /// [`sandbox_cache_dir`](Self::sandbox_cache_dir), a child of it.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// The subtree of the cache root the game may write, mapped to `/cache`.
    ///
    /// Separate from [`cache_dir`](Self::cache_dir) because the root also holds
    /// the runtime's own records. A game that could write those would decide
    /// which package bytes a later session mounts as `/code`, what identity a
    /// shared cache keys them under, and could swap a staged package between the
    /// moment an install validates it and the moment it is renamed into place.
    pub fn sandbox_cache_dir(&self) -> &Path {
        &self.sandbox_cache_dir
    }

    /// Get the temporary directory path.
    pub fn temp_dir(&self) -> &Path {
        &self.temp_dir
    }

    /// Ensure all directories exist.
    ///
    /// Creates the directories if they don't exist.
    /// Note: `code_dir` is created by the game deployment process,
    /// not by this method.
    pub fn ensure_directories(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.user_data_dir)?;
        std::fs::create_dir_all(&self.sandbox_cache_dir)?;
        std::fs::create_dir_all(&self.temp_dir)?;
        Ok(())
    }

    /// Clean the temporary directory.
    ///
    /// Removes all files in the temp directory but keeps the directory itself.
    pub fn clean_temp(&self) -> std::io::Result<()> {
        if self.temp_dir.exists() {
            std::fs::remove_dir_all(&self.temp_dir)?;
        }
        std::fs::create_dir_all(&self.temp_dir)?;
        Ok(())
    }

    /// Clean the cache directory (including temp).
    ///
    /// Removes all cached files. User data is preserved.
    pub fn clean_cache(&self) -> std::io::Result<()> {
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)?;
        }
        std::fs::create_dir_all(&self.sandbox_cache_dir)?;
        std::fs::create_dir_all(&self.temp_dir)?;
        Ok(())
    }

    /// Delete all game data (uninstall).
    ///
    /// Removes both persistent and cache data.
    /// Call this when the user uninstalls a game.
    pub fn delete_all(&self) -> std::io::Result<()> {
        // Delete files dir (user_data + code)
        let files_game_dir = self.user_data_dir.parent().unwrap_or(&self.user_data_dir);
        if files_game_dir.exists() {
            prepare_code_tree_for_removal(&self.code_dir)?;
            std::fs::remove_dir_all(files_game_dir)?;
        }

        // Delete cache dir
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)?;
        }

        Ok(())
    }

    /// Get the total size of user data in bytes.
    pub fn user_data_size(&self) -> u64 {
        dir_size(&self.user_data_dir)
    }

    /// Get the total size of cache in bytes.
    pub fn cache_size(&self) -> u64 {
        dir_size(&self.cache_dir)
    }

    /// Serialize paths to a format suitable for FFI/JNI.
    ///
    /// Returns paths as strings for cross-language communication.
    pub fn to_strings(&self) -> GamePathStrings {
        GamePathStrings {
            game_id: self.game_id.clone(),
            code_dir: self.code_dir.to_string_lossy().into_owned(),
            user_data_dir: self.user_data_dir.to_string_lossy().into_owned(),
            cache_dir: self.cache_dir.to_string_lossy().into_owned(),
            temp_dir: self.temp_dir.to_string_lossy().into_owned(),
        }
    }
}

/// String representation of game paths for FFI.
#[derive(Debug, Clone)]
pub struct GamePathStrings {
    pub game_id: String,
    pub code_dir: String,
    pub user_data_dir: String,
    pub cache_dir: String,
    pub temp_dir: String,
}

#[cfg(unix)]
fn prepare_code_tree_for_removal(code_dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = match std::fs::symlink_metadata(code_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(());
    }

    let mut directories = vec![code_dir.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let metadata = std::fs::symlink_metadata(&directory)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        std::fs::set_permissions(
            &directory,
            std::fs::Permissions::from_mode(metadata.mode() | 0o300),
        )?;
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                directories.push(entry.path());
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn prepare_code_tree_for_removal(_code_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Calculate directory size recursively.
fn dir_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }

    let mut size = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                size += entry.metadata().map(|m| m.len()).unwrap_or(0);
            } else if path.is_dir() {
                size += dir_size(&path);
            }
        }
    }
    size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_game_ids() {
        assert!(validate_game_id("my-game").is_ok());
        assert!(validate_game_id("game_123").is_ok());
        assert!(validate_game_id("GAME").is_ok());
        assert!(validate_game_id("a").is_ok());
        assert!(validate_game_id("a".repeat(64).as_str()).is_ok());
    }

    #[test]
    fn test_invalid_game_ids() {
        assert!(validate_game_id("").is_err());
        assert!(validate_game_id("a".repeat(65).as_str()).is_err());
        assert!(validate_game_id("../hack").is_err());
        assert!(validate_game_id("game/sub").is_err());
        assert!(validate_game_id("game.name").is_err());
        assert!(validate_game_id("game name").is_err());
    }

    #[test]
    fn test_game_paths_creation() {
        let paths = GamePaths::new("/data/files", "/data/cache", "test-game").unwrap();

        assert_eq!(paths.game_id(), "test-game");
        assert!(paths.code_dir().ends_with("migo/games/test-game/code"));
        assert!(
            paths
                .user_data_dir()
                .ends_with("migo/games/test-game/user_data")
        );
        assert!(paths.cache_dir().ends_with("migo/games/test-game"));
        assert!(paths.temp_dir().ends_with("migo/games/test-game/tmp"));
    }

    #[test]
    fn integrity_receipt_is_persistent_sibling_of_code() {
        let paths = GamePaths::new("/data/files", "/data/cache", "test-game").unwrap();

        assert_eq!(
            paths.integrity_receipt_path(),
            PathBuf::from("/data/files/migo/games/test-game/code.integrity-receipt-v1.json")
        );
        assert_eq!(
            paths.integrity_lock_path(),
            PathBuf::from("/data/files/migo/games/test-game/code.integrity-receipt-v1.lock")
        );
        assert!(
            !paths
                .integrity_receipt_path()
                .starts_with(paths.cache_dir())
        );
        assert!(!paths.integrity_receipt_path().starts_with(paths.code_dir()));
    }

    #[cfg(unix)]
    #[test]
    fn delete_all_removes_a_sealed_code_tree_and_receipt() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let root = std::env::temp_dir().join("migo_game_paths_sealed_delete");
        let _ = std::fs::remove_dir_all(&root);
        let paths = GamePaths::new(root.join("files"), root.join("cache"), "sealed").unwrap();
        paths.ensure_directories().unwrap();
        std::fs::create_dir_all(paths.code_dir().join("nested")).unwrap();
        std::fs::write(paths.code_dir().join("nested/game.js"), "code").unwrap();
        std::fs::write(paths.integrity_receipt_path(), "receipt").unwrap();
        std::fs::write(paths.integrity_lock_path(), "lock").unwrap();
        for directory in [
            paths.code_dir().join("nested"),
            paths.code_dir().to_path_buf(),
        ] {
            let mode = std::fs::metadata(&directory).unwrap().mode();
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(mode & !0o222))
                .unwrap();
        }

        paths.delete_all().unwrap();

        assert!(!root.join("files/migo/games/sealed").exists());
        assert!(!root.join("cache/migo/games/sealed").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
