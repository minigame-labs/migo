#!/usr/bin/env python3
"""Generate migo's Linux package metadata from cargo's own link information.

The link line for a non-cargo consumer is derived from
`cargo rustc -- --print native-static-libs` rather than maintained by hand.
Hand-writing it is what failed during the C ABI runtime slice, and deriving it
removes that class of error rather than correcting one instance of it: the list
is whatever cargo itself would pass, so it cannot drift from the build.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys

_NOTE_RE = re.compile(r"native-static-libs:\s*(?P<libs>.+)")
_PACKAGE_VERSION_RE = re.compile(
    r"^(?:0|[1-9][0-9]*)\."
    r"(?:0|[1-9][0-9]*)\."
    r"(?:0|[1-9][0-9]*)"
    r"(?:-(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)"
    r"(?:\.(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*))*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)

TARGET = "x86_64-unknown-linux-gnu"
OS = "linux"
ABI = "gnu"
ARCH = "x86_64"
CPU_BASELINE = "x86-64-v1"
REQUIRED_CPU_FEATURES = ["cmov", "sse2"]
GLIBC_FLOOR = "2.31"
GLIBCXX_FLOOR = "3.4.28"

# The Linux SDK links V8 without a startup snapshot. Stated rather than implied:
# an absent key cannot be told apart from a forgotten one, and "no snapshot" is
# exactly the claim a future Linux snapshot would have to stop making. The
# validator requires the policy and the list to agree, so neither can drift.
SNAPSHOT_POLICY = "none"


def package_version(value: str) -> str:
    """Return a safe SemVer used in generated metadata and soname paths."""
    try:
        encoded = value.encode("ascii")
    except UnicodeEncodeError as error:
        raise ValueError("package version must be ASCII SemVer") from error
    if len(encoded) > 128 or _PACKAGE_VERSION_RE.fullmatch(value) is None:
        raise ValueError(f"package version is not valid SemVer: {value!r}")
    return value


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact_identity(prefix: pathlib.Path, path: pathlib.Path) -> tuple[str, dict]:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"package artifact must be a regular file: {path}")
    try:
        relative = path.relative_to(prefix).as_posix()
    except ValueError as error:
        raise ValueError(f"artifact is outside package prefix: {path}") from error
    return relative, {
        "size_bytes": path.stat().st_size,
        "sha256": sha256_file(path),
    }


def package_artifacts(
    prefix: pathlib.Path, *, exclude: set[str] | None = None
) -> dict[str, dict]:
    """Hash the complete staged regular-file set, excluding the manifest itself."""
    excluded = exclude or set()
    artifacts: dict[str, dict] = {}
    paths = sorted(
        prefix.rglob("*"),
        key=lambda path: path.relative_to(prefix).as_posix().encode("utf-8"),
    )
    for path in paths:
        if path.is_symlink() or not path.is_file():
            continue
        relative, identity = artifact_identity(prefix, path)
        if relative not in excluded:
            artifacts[relative] = identity
    return artifacts


def parse_native_static_libs(text: str) -> list[str]:
    match = _NOTE_RE.search(text)
    if not match:
        raise ValueError("no 'native-static-libs:' note found in cargo output")
    seen: set[str] = set()
    libs: list[str] = []
    for token in match["libs"].split():
        if token not in seen:
            seen.add(token)
            libs.append(token)
    return libs


def render_pkg_config(version: str, libs: list[str], *, shared: bool) -> str:
    version = package_version(version)
    # The package stages both forms, and `-lmigo` picks the shared one unless the
    # consumer asks for --static, so the description names neither.
    _ = shared
    return f"""\
prefix=${{pcfiledir}}/../..
exec_prefix=${{prefix}}
libdir=${{prefix}}/lib
includedir=${{prefix}}/include

Name: migo
Description: Migo native HTML5 / mini-game runtime
URL: https://github.com/minigame-labs/migo
Version: {version}
Cflags: -I${{includedir}}
Libs: -L${{libdir}} -lmigo
Libs.private: {" ".join(libs)}
"""


def _cmake_link_libraries(libs: list[str]) -> str:
    return ";".join(lib[2:] for lib in libs if lib.startswith("-l"))


def render_cmake_config(version: str, libs: list[str], *, shared: bool) -> str:
    version = package_version(version)
    kind = "SHARED" if shared else "STATIC"
    filename = "libmigo.so" if shared else "libmigo.a"
    # Only the static form propagates the system libraries. libmigo.so records
    # them in DT_NEEDED itself, so repeating them here would make every consumer
    # need the -dev package of each one -- the CMake counterpart of pkg-config
    # putting them in Libs.private rather than Libs.
    interface_libs = "" if shared else _cmake_link_libraries(libs)
    return f"""\
# Generated by scripts/gen-linux-package-metadata.py -- do not edit.
cmake_minimum_required(VERSION 3.16)

get_filename_component(MIGO_PREFIX "${{CMAKE_CURRENT_LIST_DIR}}/../../.." ABSOLUTE)

set(MIGO_VERSION "{version}")
set(MIGO_INCLUDE_DIRS "${{MIGO_PREFIX}}/include")
set(MIGO_LIBRARY "${{MIGO_PREFIX}}/lib/{filename}")

add_library(migo::migo {kind} IMPORTED)
set_target_properties(migo::migo PROPERTIES
    IMPORTED_LOCATION "${{MIGO_LIBRARY}}"
    INTERFACE_INCLUDE_DIRECTORIES "${{MIGO_INCLUDE_DIRS}}"
    INTERFACE_LINK_LIBRARIES "{interface_libs}")

set(migo_FOUND TRUE)
"""


def render_cmake_version(version: str) -> str:
    version = package_version(version)
    return f"""\
# Generated by scripts/gen-linux-package-metadata.py -- do not edit.
set(PACKAGE_VERSION "{version}")
if(PACKAGE_VERSION VERSION_LESS PACKAGE_FIND_VERSION)
    set(PACKAGE_VERSION_COMPATIBLE FALSE)
else()
    set(PACKAGE_VERSION_COMPATIBLE TRUE)
    if(PACKAGE_VERSION VERSION_EQUAL PACKAGE_FIND_VERSION)
        set(PACKAGE_VERSION_EXACT TRUE)
    endif()
endif()
"""


def render_manifest(*, version, needed, v8, sysroot, build_metadata, artifacts) -> dict:
    """The package's description of itself.

    Every field here is checked against reality by
    scripts/test-linux-sdk-contract.sh; the manifest is a claim the gate
    verifies, not documentation that drifts.
    """
    version = package_version(version)
    return {
        "schema": "migo-linux-package-manifest/v2",
        "version": version,
        "product_profile": "full",
        "build_type": "release",
        "codegen_profile": "z",
        "target": TARGET,
        # os/abi/arch are carried separately from the triple so a consumer can
        # reject a mismatch without parsing the triple. "linux" alone is not an
        # ABI: Android and OpenHarmony are Linux kernels too, and a package built
        # for one is not loadable on another.
        "os": OS,
        "abi": ABI,
        "arch": ARCH,
        "cpu_baseline": CPU_BASELINE,
        "required_cpu_features": sorted(REQUIRED_CPU_FEATURES),
        "glibc_floor": GLIBC_FLOOR,
        "glibcxx_floor": GLIBCXX_FLOOR,
        "sysroot": sysroot,
        "dynamic_dependencies": sorted(needed),
        "snapshot_policy": SNAPSHOT_POLICY,
        "snapshots": [],
        "v8": v8,
        "toolchain": build_metadata["toolchain"],
        "graphics": {
            "backend_family": "gles-native",
            "required_api": "OpenGL ES 3.0",
        },
        "provenance": build_metadata["provenance"],
        "artifacts": artifacts,
    }


def _read_v8_provenance(path: pathlib.Path) -> dict:
    if not path.is_file():
        raise ValueError(f"missing verified V8 component manifest: {path}")
    value = json.loads(path.read_text())
    if value.get("schema") != "migo-v8-component-manifest/v1":
        raise ValueError(f"unsupported V8 component manifest schema: {path}")
    return value


def _read_build_metadata(path: pathlib.Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if value.get("schema") != "migo-linux-build-metadata/v1":
        raise ValueError(f"unsupported Linux build metadata schema: {path}")
    if set(value) != {"schema", "toolchain", "provenance"}:
        raise ValueError(f"Linux build metadata has unexpected fields: {path}")
    provenance = value.get("provenance")
    if not isinstance(provenance, dict):
        raise ValueError(f"Linux build metadata provenance is not an object: {path}")
    if provenance.get("build_recipe") != "scripts/build-linux-sdk.sh":
        raise ValueError("Linux SDK build metadata names the wrong build recipe")
    recipe = pathlib.Path(__file__).resolve().parent / "build-linux-sdk.sh"
    if provenance.get("build_recipe_sha256") != sha256_file(recipe):
        raise ValueError("Linux SDK build recipe hash does not match the repository")
    licenses = provenance.get("licenses")
    if not isinstance(licenses, list) or "BSL-1.1" not in licenses:
        raise ValueError("Linux SDK metadata must record the current BSL-1.1 license")
    return {"toolchain": value["toolchain"], "provenance": provenance}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix", required=True, help="staged package prefix")
    parser.add_argument("--version", required=True)
    parser.add_argument("--cargo-output",
                        help="file holding the cargo --print native-static-libs output")
    parser.add_argument("--shared", action="store_true")
    parser.add_argument("--manifest", action="store_true",
                        help="write the artifact manifest instead of the link metadata")
    parser.add_argument("--needed-from", help="file of DT_NEEDED lines")
    parser.add_argument("--sysroot", default="")
    parser.add_argument("--v8-component-manifest")
    parser.add_argument("--build-metadata")
    args = parser.parse_args()

    prefix = pathlib.Path(args.prefix)

    if args.manifest:
        if not args.v8_component_manifest or not args.build_metadata:
            parser.error("--manifest requires --v8-component-manifest and --build-metadata")
        needed = []
        if args.needed_from:
            needed = [
                line.strip()
                for line in pathlib.Path(args.needed_from).read_text().splitlines()
                if line.strip()
            ]
        manifest_relative = "share/migo/linux-x86_64-manifest.json"
        artifacts = package_artifacts(
            prefix,
            exclude={manifest_relative},
        )
        manifest = render_manifest(
            version=args.version,
            needed=needed,
            v8=_read_v8_provenance(pathlib.Path(args.v8_component_manifest)),
            sysroot=args.sysroot,
            build_metadata=_read_build_metadata(pathlib.Path(args.build_metadata)),
            artifacts=artifacts,
        )
        out_dir = prefix / "share" / "migo"
        out_dir.mkdir(parents=True, exist_ok=True)
        (prefix / manifest_relative).write_text(
            json.dumps(manifest, indent=2) + "\n")
        print(f"wrote artifact manifest under {out_dir}")
        return 0

    if not args.cargo_output:
        parser.error("--cargo-output is required unless --manifest is given")

    libs = parse_native_static_libs(pathlib.Path(args.cargo_output).read_text())

    pc_dir = prefix / "lib" / "pkgconfig"
    cmake_dir = prefix / "lib" / "cmake" / "migo"
    pc_dir.mkdir(parents=True, exist_ok=True)
    cmake_dir.mkdir(parents=True, exist_ok=True)

    (pc_dir / "migo.pc").write_text(
        render_pkg_config(args.version, libs, shared=args.shared))
    (cmake_dir / "migo-config.cmake").write_text(
        render_cmake_config(args.version, libs, shared=args.shared))
    (cmake_dir / "migo-config-version.cmake").write_text(
        render_cmake_version(args.version))

    print(f"wrote package metadata under {prefix}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
