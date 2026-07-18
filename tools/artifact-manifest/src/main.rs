use std::{
    env,
    error::Error,
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use artifact_manifest::{
    PackageIndex, ReleaseAttestation, SliceManifest, SliceManifestSource, V8ComponentManifest,
    build_package_index, build_release_attestation, seal_slice_manifest,
    seal_v8_component_manifest, validate_slice_manifest, validate_v8_component_manifest,
    verify_package_index, verify_release_attestation, verify_v8_component_files,
};
use serde::{Serialize, de::DeserializeOwned};

const USAGE: &str = "usage:
  migo-artifact-manifest seal-v8-component <input.json> <output.json>
  migo-artifact-manifest verify-v8-component <manifest.json> <archive> <binding>
  migo-artifact-manifest seal-slice <input.json> <output.json>
  migo-artifact-manifest verify-slice <manifest.json>
  migo-artifact-manifest build-index <full|slim> <output.json> <package-path=manifest-file>...
  migo-artifact-manifest verify-index <index.json> <package-root>
  migo-artifact-manifest attest <package> <index.json> <output.json>
  migo-artifact-manifest verify-attestation <attestation.json> <package> <index.json>";

fn main() {
    if let Err(error) = run(env::args_os().skip(1).collect()) {
        eprintln!("migo-artifact-manifest: {error}");
        std::process::exit(2);
    }
}

fn run(arguments: Vec<OsString>) -> Result<(), Box<dyn Error>> {
    let (command, arguments) = arguments
        .split_first()
        .ok_or_else(|| invalid_input(USAGE))?;
    let command = command
        .to_str()
        .ok_or_else(|| invalid_input("command must be valid UTF-8"))?;

    match command {
        "seal-v8-component" => {
            require_count(command, arguments, 2)?;
            let input = Path::new(&arguments[0]);
            let output = Path::new(&arguments[1]);
            let mut manifest: V8ComponentManifest = read_json(input)?;
            seal_v8_component_manifest(&mut manifest)?;
            validate_v8_component_manifest(&manifest)?;
            write_json(output, &manifest)?;
            println!("{}", manifest.component_id);
        }
        "verify-v8-component" => {
            require_count(command, arguments, 3)?;
            let manifest: V8ComponentManifest = read_json(Path::new(&arguments[0]))?;
            verify_v8_component_files(
                &manifest,
                Path::new(&arguments[1]),
                Path::new(&arguments[2]),
            )?;
            println!("{}", manifest.component_id);
        }
        "seal-slice" => {
            require_count(command, arguments, 2)?;
            let input = Path::new(&arguments[0]);
            let output = Path::new(&arguments[1]);
            let mut manifest: SliceManifest = read_json(input)?;
            seal_slice_manifest(&mut manifest)?;
            validate_slice_manifest(&manifest)?;
            write_json(output, &manifest)?;
            println!("{}", manifest.artifact_id);
        }
        "verify-slice" => {
            require_count(command, arguments, 1)?;
            let manifest: SliceManifest = read_json(Path::new(&arguments[0]))?;
            validate_slice_manifest(&manifest)?;
            println!("{}", manifest.artifact_id);
        }
        "build-index" => {
            if arguments.len() < 3 {
                return Err(invalid_input(format!(
                    "build-index requires a profile, output, and at least one slice\n{USAGE}"
                ))
                .into());
            }
            let profile = arguments[0]
                .to_str()
                .ok_or_else(|| invalid_input("product profile must be valid UTF-8"))?;
            let output = Path::new(&arguments[1]);
            let mut sources = Vec::with_capacity(arguments.len() - 2);
            for source in &arguments[2..] {
                let source = source.to_str().ok_or_else(|| {
                    invalid_input("build-index slice arguments must be valid UTF-8")
                })?;
                let (package_path, file_path) = source.split_once('=').ok_or_else(|| {
                    invalid_input(format!(
                        "slice argument must be package-path=manifest-file, got {source:?}"
                    ))
                })?;
                sources.push(SliceManifestSource {
                    package_path: package_path.to_string(),
                    file_path: PathBuf::from(file_path),
                });
            }
            let index = build_package_index(profile, &sources)?;
            write_json(output, &index)?;
            println!("{}", index.slices.len());
        }
        "verify-index" => {
            require_count(command, arguments, 2)?;
            let index: PackageIndex = read_json(Path::new(&arguments[0]))?;
            verify_package_index(&index, Path::new(&arguments[1]))?;
            println!("{}", index.slices.len());
        }
        "attest" => {
            require_count(command, arguments, 3)?;
            let attestation =
                build_release_attestation(Path::new(&arguments[0]), Path::new(&arguments[1]))?;
            write_json(Path::new(&arguments[2]), &attestation)?;
            println!("{}", attestation.package_sha256);
        }
        "verify-attestation" => {
            require_count(command, arguments, 3)?;
            let attestation: ReleaseAttestation = read_json(Path::new(&arguments[0]))?;
            verify_release_attestation(
                &attestation,
                Path::new(&arguments[1]),
                Path::new(&arguments[2]),
            )?;
            println!("{}", attestation.package_sha256);
        }
        _ => {
            return Err(invalid_input(format!("unknown command {command:?}\n{USAGE}")).into());
        }
    }
    Ok(())
}

fn require_count(command: &str, arguments: &[OsString], expected: usize) -> io::Result<()> {
    if arguments.len() == expected {
        Ok(())
    } else {
        Err(invalid_input(format!(
            "{command} expects {expected} argument(s), got {}\n{USAGE}",
            arguments.len()
        )))
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, Box<dyn Error>> {
    let bytes = fs::read(path).map_err(|error| {
        io::Error::new(error.kind(), format!("read {}: {error}", path.display()))
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| invalid_input(format!("parse JSON {}: {error}", path.display())).into())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), Box<dyn Error>> {
    let parent = path.parent().filter(|path| !path.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("create output directory {}: {error}", parent.display()),
            )
        })?;
    }
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid_input("output file name must be valid UTF-8"))?;
    let temporary = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
    let result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("write JSON {}: {error}", path.display()),
        )
        .into()
    })
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
