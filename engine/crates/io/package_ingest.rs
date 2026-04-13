//! Zip-to-package ingest: converts a zip archive into a `.mpkg` package.
//!
//! This is the "ingest" step: zip is treated as an input/download format
//! and converted to the runtime-native package format for mounted access.

use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use shared::protocol::io_cmd::MAX_READ_LENGTH;
use shared::vfs::package::{PackageError, PackageIdentity, PackageWriter};

use crate::{
    pools::PoolError,
    scheduler::IoScheduler,
    task::{IoRequest, PriorityClass},
};

fn package_ingest_request_for(zip_path: &Path) -> IoRequest {
    let compressed_bytes = std::fs::metadata(zip_path)
        .map(|meta| meta.len() as usize)
        .unwrap_or(0);
    IoRequest::PackageIngest {
        priority: PriorityClass::Background,
        compressed_bytes,
    }
}

impl From<PoolError> for PackageError {
    fn from(err: PoolError) -> Self {
        match err {
            PoolError::Closed => PackageError::Io(std::io::Error::other("IO worker pool closed")),
        }
    }
}

/// Convert a zip archive into a `.mpkg` package file (zstd-chunked).
///
/// Each entry is split into 64 KiB chunks, each independently
/// zstd-compressed for random access.
pub fn ingest_zip_to_package(
    zip_path: &Path,
    pkg_path: &Path,
    package_name: &str,
    package_version: &str,
) -> Result<PackageIdentity, PackageError> {
    let zip_file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(BufReader::new(zip_file))
        .map_err(|e| PackageError::BadIndex(format!("invalid zip: {e}")))?;

    let tmp_pkg_path = pkg_path.with_extension("mpkg.tmp");
    let _ = std::fs::remove_file(&tmp_pkg_path);
    let pkg_file = std::fs::File::create(&tmp_pkg_path)?;
    let mut writer = PackageWriter::new(std::io::BufWriter::new(pkg_file))?;

    let ingest_result: Result<PackageIdentity, PackageError> = (|| {
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| {
                PackageError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })?;

            if entry.is_dir() {
                continue;
            }

            let name = entry.name().to_string();
            if name.starts_with("__MACOSX/") || name.ends_with(".DS_Store") {
                continue;
            }

            let entry_size = entry.size();
            if entry_size > MAX_READ_LENGTH {
                return Err(PackageError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("zip entry '{}' exceeds limit {}", name, MAX_READ_LENGTH),
                )));
            }

            let mut data = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = entry.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                let next_len = data.len().saturating_add(n);
                if next_len as u64 > MAX_READ_LENGTH {
                    return Err(PackageError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("zip entry '{}' exceeds limit {}", name, MAX_READ_LENGTH),
                    )));
                }
                data.extend_from_slice(&buf[..n]);
            }

            writer.add_entry(&name, &data)?;
        }

        writer.finish(package_name, package_version)
    })();

    let identity = match ingest_result {
        Ok(identity) => identity,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_pkg_path);
            return Err(e);
        }
    };

    if let Err(e) = std::fs::rename(&tmp_pkg_path, pkg_path) {
        let _ = std::fs::remove_file(&tmp_pkg_path);
        return Err(PackageError::Io(e));
    }

    Ok(identity)
}

pub async fn ingest_zip_to_package_with_scheduler(
    scheduler: Arc<IoScheduler>,
    zip_path: PathBuf,
    pkg_path: PathBuf,
    package_name: String,
    package_version: String,
) -> Result<PackageIdentity, PackageError> {
    let request = package_ingest_request_for(&zip_path);

    scheduler
        .run_async(request, move || {
            ingest_zip_to_package(&zip_path, &pkg_path, &package_name, &package_version)
        })
        .await
        .map_err(PackageError::from)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("migo_ingest_test_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn create_test_zip(dir: &Path) -> std::path::PathBuf {
        let zip_path = dir.join("input.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        zip.start_file("main.js", options).unwrap();
        zip.write_all(b"console.log('hello')").unwrap();

        zip.start_file("lib/utils.js", options).unwrap();
        zip.write_all(b"export function x() {}").unwrap();

        zip.start_file("img/bg.png", options).unwrap();
        zip.write_all(b"\x89PNG_fake_data").unwrap();

        zip.finish().unwrap();
        zip_path
    }

    #[test]
    fn ingest_basic() {
        let dir = make_test_dir("ingest_basic");
        let zip_path = create_test_zip(&dir);
        let pkg_path = dir.join("output.mpkg");

        let identity = ingest_zip_to_package(&zip_path, &pkg_path, "test-game", "1.0.0").unwrap();

        assert_eq!(identity.name, "test-game");
        assert_eq!(identity.version, "1.0.0");

        // Verify the package is readable.
        let reader =
            shared::vfs::package::PackageReader::open(&pkg_path, "test-game", "1.0.0").unwrap();
        assert_eq!(reader.entry_count(), 3);
        assert_eq!(
            reader.read_entry("main.js").unwrap(),
            b"console.log('hello')"
        );
        assert_eq!(
            reader.read_entry("lib/utils.js").unwrap(),
            b"export function x() {}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ingest_with_compression() {
        let dir = make_test_dir("ingest_compress");
        let zip_path = create_test_zip(&dir);
        let pkg_path = dir.join("output.mpkg");

        let _identity = ingest_zip_to_package(&zip_path, &pkg_path, "test-game", "1.0.0").unwrap();

        let reader =
            shared::vfs::package::PackageReader::open(&pkg_path, "test-game", "1.0.0").unwrap();
        // JS files should be compressed, PNG should be stored.
        assert_eq!(
            reader.read_entry("main.js").unwrap(),
            b"console.log('hello')"
        );
        // PNG entry should still be readable.
        assert_eq!(
            reader.read_entry("img/bg.png").unwrap(),
            b"\x89PNG_fake_data"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_ingest_requests_default_to_background_priority() {
        let request = package_ingest_request_for(Path::new("/tmp/archive.zip"));
        match request {
            IoRequest::PackageIngest { priority, .. } => {
                assert_eq!(priority, PriorityClass::Background);
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn ingest_full_install_flow() {
        use shared::vfs::mount::{MountTable, StagingArea};

        let dir = make_test_dir("ingest_install");
        let code_dir = dir.join("code");
        std::fs::create_dir_all(&code_dir).unwrap();

        let zip_path = create_test_zip(&dir);

        // 1. Create staging area.
        let staging = StagingArea::create(&dir, "stage1").unwrap();

        // 2. Ingest zip to package in staging.
        let pkg_filename = "stage1.mpkg";
        let staged_pkg = staging.dir().join(pkg_filename);
        ingest_zip_to_package(&zip_path, &staged_pkg, "stage1", "1.0").unwrap();

        // 3. Mount table with base code dir.
        let mt = MountTable::new(code_dir.clone());

        // 4. Atomic install: validates, renames, mounts.
        let final_pkg = dir.join("packages").join("stage1.mpkg");
        let identity = staging
            .install_package(
                &mt,
                pkg_filename,
                &final_pkg,
                "subpackages/stage1",
                "stage1",
                "1.0",
            )
            .unwrap();

        assert_eq!(identity.name, "stage1");

        // 5. Verify mount works.
        assert!(mt.exists("subpackages/stage1/main.js"));
        let data = mt.read("subpackages/stage1/main.js").unwrap();
        assert_eq!(data, b"console.log('hello')");

        // 6. Base code dir is still accessible.
        std::fs::write(code_dir.join("base.js"), "// base").unwrap();
        assert!(mt.exists("base.js"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_then_replace_subpackage() {
        use shared::vfs::mount::{MountTable, StagingArea};

        let dir = make_test_dir("install_replace");
        let code_dir = dir.join("code");
        std::fs::create_dir_all(&code_dir).unwrap();
        let cache_dir = dir.join("cache");
        let pkgs_dir = cache_dir.join("migo_packages");

        // Create v1 zip.
        let zip_v1 = dir.join("v1.zip");
        {
            let file = std::fs::File::create(&zip_v1).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("game.js", opts).unwrap();
            zip.write_all(b"// version 1").unwrap();
            zip.finish().unwrap();
        }

        // Create v2 zip with different content.
        let zip_v2 = dir.join("v2.zip");
        {
            let file = std::fs::File::create(&zip_v2).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("game.js", opts).unwrap();
            zip.write_all(b"// version 2").unwrap();
            zip.start_file("new_asset.png", opts).unwrap();
            zip.write_all(b"PNG_DATA").unwrap();
            zip.finish().unwrap();
        }

        let mt = MountTable::new(code_dir.clone());
        let pkg_filename = "stage1.mpkg";
        let final_pkg_path = pkgs_dir.join(pkg_filename);

        // --- Install v1 ---
        {
            let staging = StagingArea::create(&cache_dir, "stage1").unwrap();
            let staged_pkg = staging.dir().join(pkg_filename);
            ingest_zip_to_package(&zip_v1, &staged_pkg, "stage1", "1.0").unwrap();
            staging
                .install_package(
                    &mt,
                    pkg_filename,
                    &final_pkg_path,
                    "sub/stage1",
                    "stage1",
                    "1.0",
                )
                .unwrap();
        }

        // v1 content visible.
        let data = mt.read("sub/stage1/game.js").unwrap();
        assert_eq!(data, b"// version 1");
        assert!(!mt.exists("sub/stage1/new_asset.png"));
        let gen_v1 = mt.generation();

        // --- Replace with v2 ---
        {
            let staging = StagingArea::create(&cache_dir, "stage1").unwrap();
            let staged_pkg = staging.dir().join(pkg_filename);
            ingest_zip_to_package(&zip_v2, &staged_pkg, "stage1", "2.0").unwrap();
            staging
                .install_package(
                    &mt,
                    pkg_filename,
                    &final_pkg_path,
                    "sub/stage1",
                    "stage1",
                    "2.0",
                )
                .unwrap();
        }

        // v2 content visible, v1 gone.
        let data = mt.read("sub/stage1/game.js").unwrap();
        assert_eq!(data, b"// version 2");
        assert!(mt.exists("sub/stage1/new_asset.png"));
        assert!(
            mt.generation() > gen_v1,
            "generation must increase after replace"
        );

        // Base still accessible.
        std::fs::write(code_dir.join("base.js"), "// base").unwrap();
        assert!(mt.exists("base.js"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_failure_does_not_pollute() {
        use shared::vfs::mount::{MountTable, StagingArea};

        let dir = make_test_dir("install_fail");
        let code_dir = dir.join("code");
        std::fs::create_dir_all(&code_dir).unwrap();
        let cache_dir = dir.join("cache");

        // Install a valid v1 first.
        let zip_v1 = dir.join("v1.zip");
        {
            let file = std::fs::File::create(&zip_v1).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("game.js", opts).unwrap();
            zip.write_all(b"// v1 ok").unwrap();
            zip.finish().unwrap();
        }

        let mt = MountTable::new(code_dir.clone());
        let pkg_filename = "stage1.mpkg";
        let pkgs_dir = cache_dir.join("migo_packages");
        let final_pkg_path = pkgs_dir.join(pkg_filename);

        {
            let staging = StagingArea::create(&cache_dir, "stage1").unwrap();
            let staged_pkg = staging.dir().join(pkg_filename);
            ingest_zip_to_package(&zip_v1, &staged_pkg, "stage1", "1.0").unwrap();
            staging
                .install_package(
                    &mt,
                    pkg_filename,
                    &final_pkg_path,
                    "sub/stage1",
                    "stage1",
                    "1.0",
                )
                .unwrap();
        }

        // v1 is live.
        assert_eq!(mt.read("sub/stage1/game.js").unwrap(), b"// v1 ok");

        // Try to install a corrupt "zip" (not a valid zip file).
        let bad_zip = dir.join("bad.zip");
        std::fs::write(&bad_zip, b"THIS IS NOT A ZIP").unwrap();

        {
            let staging = StagingArea::create(&cache_dir, "stage1_bad").unwrap();
            let staged_pkg = staging.dir().join(pkg_filename);
            let result = ingest_zip_to_package(&bad_zip, &staged_pkg, "stage1", "2.0");
            assert!(result.is_err(), "bad zip should fail ingest");
            // Staging is dropped/cleaned up automatically.
        }

        // v1 must still be live and readable.
        assert_eq!(mt.read("sub/stage1/game.js").unwrap(), b"// v1 ok");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Per-game manifest + restore across sessions
    // -----------------------------------------------------------------------

    #[test]
    fn manifest_restore_across_sessions() {
        use shared::vfs::mount::{
            MountTable, PackageManifest, StagingArea, package_store_dir, restore_installed_packages,
        };

        let dir = make_test_dir("manifest_restore");
        let code_dir = dir.join("code");
        let game_cache_dir = dir.join("cache");
        std::fs::create_dir_all(&code_dir).unwrap();

        let zip_path = create_test_zip(&dir);
        let store = package_store_dir(&game_cache_dir);

        // --- Session 1: install a subpackage ---
        {
            let mt = MountTable::new(code_dir.clone());

            let staging = StagingArea::create(&game_cache_dir, "stage1").unwrap();
            let staged_pkg = staging.dir().join("stage1.mpkg");
            ingest_zip_to_package(&zip_path, &staged_pkg, "stage1", "1.0").unwrap();
            staging
                .install_package(
                    &mt,
                    "stage1.mpkg",
                    &store.join("stage1.mpkg"),
                    "subpackages/stage1",
                    "stage1",
                    "1.0",
                )
                .unwrap();

            // Write manifest.
            let mut manifest = PackageManifest::load(&store);
            manifest.record("stage1".into(), "subpackages/stage1".into(), "1.0".into());
            manifest.save(&store).unwrap();

            // Verify readable in session 1.
            assert_eq!(
                mt.read("subpackages/stage1/main.js").unwrap(),
                b"console.log('hello')"
            );
        }
        // Session 1 ends — MountTable dropped.

        // --- Session 2: fresh MountTable, restore from manifest ---
        {
            let mt = MountTable::new(code_dir.clone());
            // No overlays yet.
            assert!(mt.read("subpackages/stage1/main.js").is_err());

            // Restore from manifest.
            restore_installed_packages(&mt, &game_cache_dir, false);

            // Now the subpackage is visible again!
            assert_eq!(
                mt.read("subpackages/stage1/main.js").unwrap(),
                b"console.log('hello')"
            );
            assert!(mt.exists("subpackages/stage1/lib/utils.js"));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn code_tree_subpackage_runs_without_download() {
        use shared::vfs::mount::MountTable;

        let dir = make_test_dir("code_tree_sub");
        let code_dir = dir.join("code");

        // Simulate subpackage already in code tree (pre-extracted).
        let sub_dir = code_dir.join("subpackages").join("stage1");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::write(sub_dir.join("game.js"), b"// stage1 game").unwrap();

        let mt = MountTable::new(code_dir.clone());

        // No overlay, no package store — but the base DirSource covers it.
        assert!(mt.exists("subpackages/stage1/game.js"));
        assert_eq!(
            mt.read("subpackages/stage1/game.js").unwrap(),
            b"// stage1 game"
        );

        // loadSubpackage's _tryLocalExecute would call amdRequire which
        // calls op_require_resolve_and_read which goes through MountTable.
        // This proves the code tree path works without download.

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn predownload_then_load_no_redownload() {
        use shared::vfs::mount::{MountTable, PackageManifest, StagingArea, package_store_dir};

        let dir = make_test_dir("predownload");
        let code_dir = dir.join("code");
        let game_cache_dir = dir.join("cache");
        std::fs::create_dir_all(&code_dir).unwrap();

        let zip_path = create_test_zip(&dir);
        let store = package_store_dir(&game_cache_dir);

        // preDownloadSubpackage: install but don't execute.
        let mt = MountTable::new(code_dir.clone());
        {
            let staging = StagingArea::create(&game_cache_dir, "stage1").unwrap();
            let staged_pkg = staging.dir().join("stage1.mpkg");
            ingest_zip_to_package(&zip_path, &staged_pkg, "stage1", "1.0").unwrap();
            staging
                .install_package(
                    &mt,
                    "stage1.mpkg",
                    &store.join("stage1.mpkg"),
                    "subpackages/stage1",
                    "stage1",
                    "1.0",
                )
                .unwrap();

            let mut manifest = PackageManifest::load(&store);
            manifest.record("stage1".into(), "subpackages/stage1".into(), "1.0".into());
            manifest.save(&store).unwrap();
        }

        // loadSubpackage: should find it already mounted, no download needed.
        // (In the real JS flow, _tryLocalExecute would succeed here.)
        assert!(mt.exists("subpackages/stage1/main.js"));
        assert_eq!(
            mt.read("subpackages/stage1/main.js").unwrap(),
            b"console.log('hello')"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
