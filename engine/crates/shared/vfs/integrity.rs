//! Code integrity verification: manifest signature + file hash checks.
//!
//! Provides Ed25519 signature verification of a `manifest.json` file
//! and SHA256 hash verification of individual source files listed in the manifest.
//!
//! # Manifest Format
//!
//! ```json
//! {
//!   "version": 1,
//!   "entry": "game.js",
//!   "timestamp": 1709078400,
//!   "files": {
//!     "game.js": "a1b2c3d4e5f6...",
//!     "lib/utils.js": "f7e8d9c0b1a2..."
//!   }
//! }
//! ```
//!
//! # Signature
//!
//! The `manifest.sig` file contains the raw Ed25519 signature (64 bytes)
//! of the exact bytes of `manifest.json`.
//!
//! # Hash Cache
//!
//! File hashes are cached by (path, mtime, size, file identity) to avoid
//! redundant re-hashing on repeated calls (e.g., game restart without changes).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{EngineError, EngineResult, ErrorCode};

// ---------------------------------------------------------------------------
// Manifest schema
// ---------------------------------------------------------------------------

/// Manifest schema for code signing verification.
///
/// Describes the game package contents, entry point, and per-file SHA256 hashes.
/// The manifest itself is signed with Ed25519 to prevent tampering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version. Currently must be 1.
    pub version: u32,
    /// Entry point file path (relative to code_dir), e.g. `"game.js"`.
    pub entry: String,
    /// Build timestamp (Unix seconds UTC). Used for staleness detection.
    pub timestamp: u64,
    /// Map of relative file paths to their lowercase SHA256 hex digests.
    pub files: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Hash cache entry
// ---------------------------------------------------------------------------

/// Cached hash for a single file, keyed by (mtime, size).
#[derive(Debug, Clone)]
struct CachedHash {
    mtime: SystemTime,
    size: u64,
    file_id: FileIdentity,
    hash_hex: String,
}

/// Best-effort stable file identity used to make cache hits safer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

impl FileIdentity {
    fn from_metadata(meta: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            return Self {
                dev: meta.dev(),
                ino: meta.ino(),
            };
        }
        #[cfg(not(unix))]
        {
            let _ = meta;
            Self {}
        }
    }
}

// ---------------------------------------------------------------------------
// IntegrityVerifier
// ---------------------------------------------------------------------------

/// Code integrity verifier with Ed25519 signature check and file hash cache.
///
/// # Lifecycle
///
/// 1. Construct once per host with [`IntegrityVerifier::from_hex_pubkey`].
/// 2. Call [`verify_entry`] before each `evaluate_module` to check the
///    manifest signature + entry file hash.
/// 3. Optionally call [`verify_all_files`] for full-package verification (P1).
/// 4. Call [`clear_cache`] on game restart if files may have changed.
#[derive(Debug)]
pub struct IntegrityVerifier {
    pubkey: VerifyingKey,
    cache: HashMap<PathBuf, CachedHash>,
}

impl IntegrityVerifier {
    /// Create a verifier from raw 32-byte Ed25519 public key bytes.
    pub fn from_pubkey_bytes(bytes: &[u8; 32]) -> EngineResult<Self> {
        let pubkey = VerifyingKey::from_bytes(bytes).map_err(|e| {
            EngineError::new(ErrorCode::CodeSignatureInvalid)
                .with_msg("invalid Ed25519 public key")
                .with_detail(e.to_string())
        })?;
        Ok(Self {
            pubkey,
            cache: HashMap::new(),
        })
    }

    /// Create a verifier from a hex-encoded public key string (64 hex chars = 32 bytes).
    pub fn from_hex_pubkey(hex_str: &str) -> EngineResult<Self> {
        let bytes = hex::decode(hex_str).map_err(|e| {
            EngineError::new(ErrorCode::CodeSignatureInvalid)
                .with_msg("invalid hex public key")
                .with_detail(e.to_string())
        })?;
        if bytes.len() != 32 {
            return Err(EngineError::new(ErrorCode::CodeSignatureInvalid)
                .with_msg("public key must be 32 bytes")
                .with_detail(format!("got {} bytes", bytes.len())));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Self::from_pubkey_bytes(&arr)
    }

    // -----------------------------------------------------------------------
    // Public verification API
    // -----------------------------------------------------------------------

    /// Verify manifest signature and entry file integrity (MVP).
    ///
    /// Steps:
    /// 1. Read `manifest.json` from `code_dir`.
    /// 2. Read `manifest.sig` and verify the Ed25519 signature.
    /// 3. Parse the manifest and validate schema version.
    /// 4. Ensure the manifest's `entry` matches the requested `entry`.
    /// 5. Verify the entry file's SHA256 hash against the manifest.
    ///
    /// # Returns
    ///
    /// The parsed [`Manifest`] on success, allowing the caller to optionally
    /// run [`verify_all_files`] for full-package verification.
    pub fn verify_entry(&mut self, code_dir: &Path, entry: &str) -> EngineResult<Manifest> {
        // 1. Read manifest.json
        let manifest_path = code_dir.join("manifest.json");
        let manifest_bytes = std::fs::read(&manifest_path).map_err(|e| {
            EngineError::new(ErrorCode::CodeSignatureInvalid)
                .with_msg("read manifest.json")
                .with_detail(format!("{}: {}", manifest_path.display(), e))
        })?;

        // 2. Read manifest.sig
        let sig_path = code_dir.join("manifest.sig");
        let sig_bytes = std::fs::read(&sig_path).map_err(|e| {
            EngineError::new(ErrorCode::CodeSignatureInvalid)
                .with_msg("read manifest.sig")
                .with_detail(format!("{}: {}", sig_path.display(), e))
        })?;

        // 3. Verify Ed25519 signature
        self.verify_signature(&manifest_bytes, &sig_bytes)?;

        // 4. Parse manifest
        let manifest: Manifest =
            deno_core::serde_json::from_slice(&manifest_bytes).map_err(|e| {
                EngineError::new(ErrorCode::CodeSignatureInvalid)
                    .with_msg("parse manifest.json")
                    .with_detail(e.to_string())
            })?;

        // 5. Validate schema version
        if manifest.version != 1 {
            return Err(EngineError::new(ErrorCode::CodeSignatureInvalid)
                .with_msg("unsupported manifest version")
                .with_detail(format!("expected 1, got {}", manifest.version)));
        }

        // 6. Verify manifest entry matches requested entry
        if manifest.entry != entry {
            return Err(EngineError::new(ErrorCode::CodeIntegrityFailed)
                .with_msg("entry mismatch")
                .with_detail(format!(
                    "manifest entry='{}', requested='{}'",
                    manifest.entry, entry
                )));
        }

        // 7. Verify entry file hash
        self.verify_file_hash(code_dir, entry, &manifest.files)?;

        Ok(manifest)
    }

    /// Verify all files listed in the manifest (P1 full-package verification).
    ///
    /// Call this after [`verify_entry`] to ensure every file in the manifest
    /// matches its declared SHA256 hash.
    pub fn verify_all_files(&mut self, code_dir: &Path, manifest: &Manifest) -> EngineResult<()> {
        for (path, _) in &manifest.files {
            self.verify_file_hash(code_dir, path, &manifest.files)?;
        }
        Ok(())
    }

    /// Clear the file hash cache.
    ///
    /// Call on game restart or after file updates to force re-verification.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Verify the Ed25519 signature of raw manifest bytes.
    fn verify_signature(&self, manifest_bytes: &[u8], sig_bytes: &[u8]) -> EngineResult<()> {
        if sig_bytes.len() != 64 {
            return Err(EngineError::new(ErrorCode::CodeSignatureInvalid)
                .with_msg("invalid signature length")
                .with_detail(format!("expected 64 bytes, got {}", sig_bytes.len())));
        }

        let sig = Signature::from_slice(sig_bytes).map_err(|e| {
            EngineError::new(ErrorCode::CodeSignatureInvalid)
                .with_msg("parse Ed25519 signature")
                .with_detail(e.to_string())
        })?;

        self.pubkey.verify(manifest_bytes, &sig).map_err(|e| {
            EngineError::new(ErrorCode::CodeSignatureInvalid)
                .with_msg("manifest signature verification failed")
                .with_detail(e.to_string())
        })?;

        Ok(())
    }

    /// Verify a single file's SHA256 hash against the manifest.
    fn verify_file_hash(
        &mut self,
        code_dir: &Path,
        relative_path: &str,
        files: &HashMap<String, String>,
    ) -> EngineResult<()> {
        validate_manifest_relative_path(relative_path)?;
        let expected_hash = files.get(relative_path).ok_or_else(|| {
            EngineError::new(ErrorCode::CodeIntegrityFailed)
                .with_msg("file not listed in manifest")
                .with_detail(format!("'{}'", relative_path))
        })?;

        let full_path = secure_join_under_code_dir(code_dir, relative_path)?;
        let actual_hash = self.hash_file_cached(&full_path)?;

        if actual_hash != *expected_hash {
            return Err(EngineError::new(ErrorCode::CodeIntegrityFailed)
                .with_msg("file hash mismatch")
                .with_detail(format!(
                    "file='{}', expected='{}', actual='{}'",
                    relative_path, expected_hash, actual_hash
                )));
        }

        Ok(())
    }

    /// Compute SHA256 of a file with metadata/file-id cache validation.
    ///
    /// If the file's mtime and size match a cached entry, the cached hash
    /// is returned without re-reading the file.
    fn hash_file_cached(&mut self, path: &Path) -> EngineResult<String> {
        let meta = std::fs::metadata(path).map_err(|e| {
            EngineError::new(ErrorCode::CodeIntegrityFailed)
                .with_msg("read file metadata")
                .with_detail(format!("{}: {}", path.display(), e))
        })?;

        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let size = meta.len();
        let file_id = FileIdentity::from_metadata(&meta);

        // Check cache hit
        if let Some(cached) = self.cache.get(path) {
            if cached.mtime == mtime && cached.size == size && cached.file_id == file_id {
                return Ok(cached.hash_hex.clone());
            }
        }

        // Cache miss: compute hash
        let hash_hex = sha256_file(path)?;

        // Store in cache
        self.cache.insert(
            path.to_path_buf(),
            CachedHash {
                mtime,
                size,
                file_id,
                hash_hex: hash_hex.clone(),
            },
        );

        Ok(hash_hex)
    }
}

fn validate_manifest_relative_path(relative_path: &str) -> EngineResult<()> {
    if relative_path.is_empty() {
        return Err(
            EngineError::new(ErrorCode::CodeIntegrityFailed).with_msg("manifest path is empty")
        );
    }
    if relative_path.starts_with('/') || relative_path.starts_with('\\') {
        return Err(EngineError::new(ErrorCode::CodeIntegrityFailed)
            .with_msg("manifest path must be relative")
            .with_detail(relative_path.to_string()));
    }

    for b in relative_path.bytes() {
        if b < 0x20 {
            return Err(EngineError::new(ErrorCode::CodeIntegrityFailed)
                .with_msg("manifest path contains control characters")
                .with_detail(relative_path.to_string()));
        }
        if b == b'\\' {
            return Err(EngineError::new(ErrorCode::CodeIntegrityFailed)
                .with_msg("manifest path must use '/' separators")
                .with_detail(relative_path.to_string()));
        }
    }

    for component in Path::new(relative_path).components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(EngineError::new(ErrorCode::CodeIntegrityFailed)
                    .with_msg("manifest path escapes code directory")
                    .with_detail(relative_path.to_string()));
            }
        }
    }

    Ok(())
}

fn secure_join_under_code_dir(code_dir: &Path, relative_path: &str) -> EngineResult<PathBuf> {
    let full_path = code_dir.join(relative_path);

    // Phase 1: Textual normalization (fast, no I/O)
    let normalized = normalize_textual_path(&full_path);
    let normalized_base = normalize_textual_path(code_dir);
    if !normalized.starts_with(&normalized_base) {
        return Err(EngineError::new(ErrorCode::CodeIntegrityFailed)
            .with_msg("manifest path resolves outside code directory")
            .with_detail(relative_path.to_string()));
    }

    // Phase 2: Filesystem canonicalization (resolves symlinks).
    // If code_dir itself is a symlink, the textual check above could be
    // bypassed.  Canonicalize both paths and re-verify containment.
    if full_path.exists() {
        let canonical_full = std::fs::canonicalize(&full_path).map_err(|e| {
            EngineError::new(ErrorCode::CodeIntegrityFailed)
                .with_msg("canonicalize file path")
                .with_detail(format!("{}: {}", full_path.display(), e))
        })?;
        let canonical_base = std::fs::canonicalize(code_dir).map_err(|e| {
            EngineError::new(ErrorCode::CodeIntegrityFailed)
                .with_msg("canonicalize code directory")
                .with_detail(format!("{}: {}", code_dir.display(), e))
        })?;
        if !canonical_full.starts_with(&canonical_base) {
            return Err(EngineError::new(ErrorCode::CodeIntegrityFailed)
                .with_msg("file resolves outside code directory (symlink escape)")
                .with_detail(relative_path.to_string()));
        }
    }

    Ok(full_path)
}

fn normalize_textual_path(path: &Path) -> PathBuf {
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
// Standalone hash utilities
// ---------------------------------------------------------------------------

/// Compute the SHA256 hex digest of a file using streaming reads
/// to avoid loading the entire file into memory at once.
pub fn sha256_file(path: &Path) -> EngineResult<String> {
    use std::io::Read;

    let file = std::fs::File::open(path).map_err(|e| {
        EngineError::new(ErrorCode::CodeIntegrityFailed)
            .with_msg("read file for hashing")
            .with_detail(format!("{}: {}", path.display(), e))
    })?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf).map_err(|e| {
            EngineError::new(ErrorCode::CodeIntegrityFailed)
                .with_msg("read file for hashing")
                .with_detail(format!("{}: {}", path.display(), e))
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Compute the SHA256 hex digest of a byte slice.
pub fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::fs;

    /// Create a unique temp directory for a test.
    fn make_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("migo_integrity_test_{}", name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Sign manifest bytes with the given signing key.
    fn sign_manifest(signing_key: &SigningKey, manifest_json: &[u8]) -> Vec<u8> {
        let sig = signing_key.sign(manifest_json);
        sig.to_bytes().to_vec()
    }

    /// Set up a complete signed game package and return (signing_key, pubkey_bytes).
    fn setup_signed_package(
        dir: &Path,
        entry: &str,
        entry_content: &str,
    ) -> (SigningKey, [u8; 32]) {
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();

        // Write entry file
        fs::write(dir.join(entry), entry_content).unwrap();

        // Compute hash
        let hash = sha256_bytes(entry_content.as_bytes());

        // Create manifest
        let mut files = HashMap::new();
        files.insert(entry.to_string(), hash);

        let manifest = Manifest {
            version: 1,
            entry: entry.to_string(),
            timestamp: 1709078400,
            files,
        };

        let manifest_json = deno_core::serde_json::to_vec_pretty(&manifest).unwrap();

        // Sign and write
        let sig = sign_manifest(&signing_key, &manifest_json);
        fs::write(dir.join("manifest.json"), &manifest_json).unwrap();
        fs::write(dir.join("manifest.sig"), &sig).unwrap();

        (signing_key, verifying_key.to_bytes())
    }

    // -----------------------------------------------------------------------
    // Basic success case
    // -----------------------------------------------------------------------

    #[test]
    fn test_valid_signature_and_hash() {
        let dir = make_test_dir("valid");
        let (_sk, pubkey) = setup_signed_package(&dir, "game.js", "console.log('hello');");

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();
        let manifest = verifier.verify_entry(&dir, "game.js").unwrap();

        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.entry, "game.js");
        assert_eq!(manifest.timestamp, 1709078400);
        assert!(manifest.files.contains_key("game.js"));

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Tampered JS file
    // -----------------------------------------------------------------------

    #[test]
    fn test_tampered_js_file_rejected() {
        let dir = make_test_dir("tampered_js");
        let (_sk, pubkey) = setup_signed_package(&dir, "game.js", "console.log('hello');");

        // Tamper with the JS file after signing
        fs::write(dir.join("game.js"), "console.log('HACKED');").unwrap();

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();
        let result = verifier.verify_entry(&dir, "game.js");

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeIntegrityFailed);

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Tampered manifest (signature mismatch)
    // -----------------------------------------------------------------------

    #[test]
    fn test_tampered_manifest_rejected() {
        let dir = make_test_dir("tampered_manifest");
        let (_sk, pubkey) = setup_signed_package(&dir, "game.js", "console.log('hello');");

        // Tamper with manifest.json (change timestamp) without re-signing
        let manifest_path = dir.join("manifest.json");
        let mut manifest_json: deno_core::serde_json::Value =
            deno_core::serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest_json["timestamp"] = deno_core::serde_json::json!(9999999);
        fs::write(
            &manifest_path,
            deno_core::serde_json::to_vec_pretty(&manifest_json).unwrap(),
        )
        .unwrap();

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();
        let result = verifier.verify_entry(&dir, "game.js");

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeSignatureInvalid);

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Wrong public key
    // -----------------------------------------------------------------------

    #[test]
    fn test_wrong_public_key_rejected() {
        let dir = make_test_dir("wrong_key");
        let (_sk, _pubkey) = setup_signed_package(&dir, "game.js", "console.log('hello');");

        // Derive a different key pair
        let wrong_signing = SigningKey::from_bytes(&[99u8; 32]);
        let wrong_pubkey = wrong_signing.verifying_key().to_bytes();

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&wrong_pubkey).unwrap();
        let result = verifier.verify_entry(&dir, "game.js");

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeSignatureInvalid);

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Missing manifest files
    // -----------------------------------------------------------------------

    #[test]
    fn test_missing_manifest_rejected() {
        let dir = make_test_dir("missing_manifest");
        fs::write(dir.join("game.js"), "console.log('hello');").unwrap();

        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let pubkey = signing_key.verifying_key().to_bytes();

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();
        let result = verifier.verify_entry(&dir, "game.js");

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeSignatureInvalid);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_missing_signature_rejected() {
        let dir = make_test_dir("missing_sig");

        // Write manifest without signature
        let mut files = HashMap::new();
        files.insert(
            "game.js".to_string(),
            sha256_bytes(b"console.log('hello');"),
        );
        let manifest = Manifest {
            version: 1,
            entry: "game.js".to_string(),
            timestamp: 1709078400,
            files,
        };
        fs::write(
            dir.join("manifest.json"),
            deno_core::serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(dir.join("game.js"), "console.log('hello');").unwrap();

        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let pubkey = signing_key.verifying_key().to_bytes();

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();
        let result = verifier.verify_entry(&dir, "game.js");

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeSignatureInvalid);

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Entry mismatch
    // -----------------------------------------------------------------------

    #[test]
    fn test_entry_mismatch_rejected() {
        let dir = make_test_dir("entry_mismatch");
        let (_sk, pubkey) = setup_signed_package(&dir, "game.js", "console.log('hello');");

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();
        // Request a different entry than what's in the manifest
        let result = verifier.verify_entry(&dir, "other.js");

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeIntegrityFailed);

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Hash cache
    // -----------------------------------------------------------------------

    #[test]
    fn test_hash_cache_works() {
        let dir = make_test_dir("hash_cache");
        let (_sk, pubkey) = setup_signed_package(&dir, "game.js", "console.log('hello');");

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();

        // First call populates cache
        assert!(verifier.verify_entry(&dir, "game.js").is_ok());
        assert!(!verifier.cache.is_empty());

        // Second call uses cache (no I/O for the JS file hash)
        assert!(verifier.verify_entry(&dir, "game.js").is_ok());

        // Clear and verify cache is empty
        verifier.clear_cache();
        assert!(verifier.cache.is_empty());

        // Still works after clearing
        assert!(verifier.verify_entry(&dir, "game.js").is_ok());

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // verify_all_files (P1)
    // -----------------------------------------------------------------------

    #[test]
    fn test_verify_all_files_pass() {
        let dir = make_test_dir("verify_all_pass");
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let pubkey = signing_key.verifying_key().to_bytes();

        // Write multiple files
        fs::write(dir.join("game.js"), "main code").unwrap();
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::write(dir.join("lib/utils.js"), "utils code").unwrap();

        // Create manifest
        let mut files = HashMap::new();
        files.insert("game.js".to_string(), sha256_bytes(b"main code"));
        files.insert("lib/utils.js".to_string(), sha256_bytes(b"utils code"));

        let manifest = Manifest {
            version: 1,
            entry: "game.js".to_string(),
            timestamp: 1709078400,
            files,
        };
        let manifest_json = deno_core::serde_json::to_vec_pretty(&manifest).unwrap();
        let sig = sign_manifest(&signing_key, &manifest_json);
        fs::write(dir.join("manifest.json"), &manifest_json).unwrap();
        fs::write(dir.join("manifest.sig"), &sig).unwrap();

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();
        let parsed = verifier.verify_entry(&dir, "game.js").unwrap();
        assert!(verifier.verify_all_files(&dir, &parsed).is_ok());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_verify_all_files_tampered_dependency() {
        let dir = make_test_dir("verify_all_tampered");
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let pubkey = signing_key.verifying_key().to_bytes();

        // Write files
        fs::write(dir.join("game.js"), "main code").unwrap();
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::write(dir.join("lib/utils.js"), "utils code").unwrap();

        // Create manifest
        let mut files = HashMap::new();
        files.insert("game.js".to_string(), sha256_bytes(b"main code"));
        files.insert("lib/utils.js".to_string(), sha256_bytes(b"utils code"));

        let manifest = Manifest {
            version: 1,
            entry: "game.js".to_string(),
            timestamp: 1709078400,
            files,
        };
        let manifest_json = deno_core::serde_json::to_vec_pretty(&manifest).unwrap();
        let sig = sign_manifest(&signing_key, &manifest_json);
        fs::write(dir.join("manifest.json"), &manifest_json).unwrap();
        fs::write(dir.join("manifest.sig"), &sig).unwrap();

        // Tamper with a dependency
        fs::write(dir.join("lib/utils.js"), "HACKED").unwrap();

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();
        let parsed = verifier.verify_entry(&dir, "game.js").unwrap();

        // Entry passes, but full verification catches the tampered dependency
        let result = verifier.verify_all_files(&dir, &parsed);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeIntegrityFailed);

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Utility function tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_sha256_bytes_known_vector() {
        // SHA256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        let hash = sha256_bytes(b"hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_sha256_bytes_empty() {
        // SHA256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let hash = sha256_bytes(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_from_hex_pubkey_valid() {
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let pubkey = signing_key.verifying_key();
        let hex_str = hex::encode(pubkey.to_bytes());

        assert!(IntegrityVerifier::from_hex_pubkey(&hex_str).is_ok());
    }

    #[test]
    fn test_from_hex_pubkey_too_short() {
        let result = IntegrityVerifier::from_hex_pubkey("aabbccdd");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeSignatureInvalid);
    }

    #[test]
    fn test_from_hex_pubkey_invalid_hex() {
        let result = IntegrityVerifier::from_hex_pubkey("zzzzzz");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeSignatureInvalid);
    }

    #[test]
    fn test_invalid_manifest_version() {
        let dir = make_test_dir("invalid_version");
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let pubkey = signing_key.verifying_key().to_bytes();

        fs::write(dir.join("game.js"), "code").unwrap();

        let mut files = HashMap::new();
        files.insert("game.js".to_string(), sha256_bytes(b"code"));

        // Version 99 is unsupported
        let manifest = Manifest {
            version: 99,
            entry: "game.js".to_string(),
            timestamp: 1709078400,
            files,
        };
        let manifest_json = deno_core::serde_json::to_vec_pretty(&manifest).unwrap();
        let sig = sign_manifest(&signing_key, &manifest_json);
        fs::write(dir.join("manifest.json"), &manifest_json).unwrap();
        fs::write(dir.join("manifest.sig"), &sig).unwrap();

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();
        let result = verifier.verify_entry(&dir, "game.js");

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeSignatureInvalid);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_invalid_signature_length() {
        let dir = make_test_dir("bad_sig_len");
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let pubkey = signing_key.verifying_key().to_bytes();

        fs::write(dir.join("game.js"), "code").unwrap();

        let mut files = HashMap::new();
        files.insert("game.js".to_string(), sha256_bytes(b"code"));

        let manifest = Manifest {
            version: 1,
            entry: "game.js".to_string(),
            timestamp: 1709078400,
            files,
        };
        let manifest_json = deno_core::serde_json::to_vec_pretty(&manifest).unwrap();
        fs::write(dir.join("manifest.json"), &manifest_json).unwrap();
        // Write a truncated signature
        fs::write(dir.join("manifest.sig"), &[0u8; 32]).unwrap();

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();
        let result = verifier.verify_entry(&dir, "game.js");

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeSignatureInvalid);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_manifest_rejects_parent_dir_path() {
        let dir = make_test_dir("parent_dir_path");
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let pubkey = signing_key.verifying_key().to_bytes();

        fs::write(dir.join("game.js"), "code").unwrap();
        fs::write(dir.join("secret.js"), "secret").unwrap();

        let mut files = HashMap::new();
        files.insert("../secret.js".to_string(), sha256_bytes(b"secret"));

        let manifest = Manifest {
            version: 1,
            entry: "../secret.js".to_string(),
            timestamp: 1709078400,
            files,
        };
        let manifest_json = deno_core::serde_json::to_vec_pretty(&manifest).unwrap();
        let sig = sign_manifest(&signing_key, &manifest_json);
        fs::write(dir.join("manifest.json"), &manifest_json).unwrap();
        fs::write(dir.join("manifest.sig"), &sig).unwrap();

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();
        let result = verifier.verify_entry(&dir, "../secret.js");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeIntegrityFailed);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_manifest_rejects_absolute_path() {
        let dir = make_test_dir("absolute_path");
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let pubkey = signing_key.verifying_key().to_bytes();

        let mut files = HashMap::new();
        files.insert("/etc/passwd".to_string(), sha256_bytes(b"x"));

        let manifest = Manifest {
            version: 1,
            entry: "/etc/passwd".to_string(),
            timestamp: 1709078400,
            files,
        };
        let manifest_json = deno_core::serde_json::to_vec_pretty(&manifest).unwrap();
        let sig = sign_manifest(&signing_key, &manifest_json);
        fs::write(dir.join("manifest.json"), &manifest_json).unwrap();
        fs::write(dir.join("manifest.sig"), &sig).unwrap();

        let mut verifier = IntegrityVerifier::from_pubkey_bytes(&pubkey).unwrap();
        let result = verifier.verify_entry(&dir, "/etc/passwd");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CodeIntegrityFailed);

        let _ = fs::remove_dir_all(&dir);
    }
}
