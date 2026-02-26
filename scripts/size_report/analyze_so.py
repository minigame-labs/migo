#!/usr/bin/env python3
"""
Analyze .so file size breakdown from an AAR or directly.

Usage:
    python scripts/size_report/analyze_so.py [aar_or_so_path]

Defaults to: platforms/android/dist/migo-release.aar

Requires: llvm-readelf and llvm-nm on PATH (from Android NDK).
"""

import subprocess
import sys
import os
import zipfile
import tempfile
import shutil
from pathlib import Path


def find_tool(name):
    """Find llvm tool, checking NDK paths."""
    # Try PATH first
    result = shutil.which(name)
    if result:
        return result
    # Try common NDK locations
    for env in ["ANDROID_NDK_HOME", "NDK_HOME", "ANDROID_NDK"]:
        ndk = os.environ.get(env)
        if ndk:
            tool = Path(ndk) / "toolchains" / "llvm" / "prebuilt" / "*" / "bin" / name
            import glob
            matches = glob.glob(str(tool))
            if matches:
                return matches[0]
    return None


def run(cmd, **kwargs):
    """Run command and return stdout."""
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=30, **kwargs)
        return r.stdout
    except Exception as e:
        return f"ERROR: {e}"


def parse_sections(readelf_output):
    """Parse section headers from llvm-readelf -S output."""
    sections = []
    for line in readelf_output.splitlines():
        line = line.strip()
        if not line or not line.startswith("["):
            continue
        parts = line.split()
        if len(parts) < 7:
            continue
        try:
            nr = parts[0].strip("[]")
            int(nr)  # validate it's a number
        except ValueError:
            continue
        name = parts[1]
        if not name.startswith("."):
            continue
        # Size is at index 5 (hex)
        try:
            size = int(parts[5], 16)
        except (ValueError, IndexError):
            continue
        sections.append((name, size))
    return sections


def parse_symbols(nm_output):
    """Parse dynamic symbols from llvm-nm -D output."""
    symbols = {"total": 0, "text": 0, "jni": 0, "data": 0, "other": 0, "non_jni_text": []}
    for line in nm_output.splitlines():
        line = line.strip()
        if not line:
            continue
        symbols["total"] += 1
        parts = line.split()
        if len(parts) >= 3:
            sym_type = parts[1]
            sym_name = parts[2]
            if sym_type == "T":
                symbols["text"] += 1
                if "Java_" in sym_name or "JNI_OnLoad" in sym_name:
                    symbols["jni"] += 1
                else:
                    symbols["non_jni_text"].append(sym_name)
            elif sym_type in ("D", "B", "R"):
                symbols["data"] += 1
            else:
                symbols["other"] += 1
    return symbols


def fmt_size(size_bytes):
    """Format bytes to human readable."""
    if size_bytes >= 1024 * 1024:
        return f"{size_bytes / 1024 / 1024:.2f} MB"
    elif size_bytes >= 1024:
        return f"{size_bytes / 1024:.1f} KB"
    return f"{size_bytes} B"


def analyze_so(so_path, readelf_bin, nm_bin):
    """Analyze a single .so file."""
    file_size = os.path.getsize(so_path)
    print(f"\n{'=' * 65}")
    print(f"  {os.path.basename(so_path)}  ({fmt_size(file_size)})")
    print(f"{'=' * 65}")

    # Section analysis
    output = run([readelf_bin, "-S", so_path])
    sections = parse_sections(output)

    if sections:
        total_alloc = sum(s for _, s in sections)
        print(f"\n  Section Breakdown:")
        print(f"  {'Section':<25} {'Size':>12} {'%':>8}")
        print(f"  {'-' * 48}")

        # Group by category
        categories = {
            "Code (.text + .plt)": [".text", ".plt", "__lcxx_override", "malloc_hook"],
            "Read-only data (.rodata)": [".rodata"],
            "Unwind info (.eh_frame*)": [".eh_frame", ".eh_frame_hdr", ".gcc_except_table"],
            "Relocations (.rela.*)": [".rela.dyn", ".rela.plt"],
            "GOT/PLT data": [".got", ".got.plt"],
            "Writable data": [".data", ".data.rel.ro", ".bss", ".init_array", ".fini_array"],
            "Dynamic linking": [".dynsym", ".dynstr", ".gnu.hash", ".hash", ".gnu.version", ".gnu.version_r", ".dynamic"],
        }

        for name, size in sorted(sections, key=lambda x: -x[1]):
            pct = size / total_alloc * 100 if total_alloc else 0
            if size > 1024:
                print(f"  {name:<25} {fmt_size(size):>12} {pct:>7.1f}%")

        print(f"  {'-' * 48}")

        # Category summary
        print(f"\n  Category Summary:")
        print(f"  {'Category':<35} {'Size':>12} {'%':>8}")
        print(f"  {'-' * 58}")
        cat_sizes = []
        for cat_name, cat_sections in categories.items():
            cat_size = sum(s for n, s in sections if n in cat_sections)
            cat_sizes.append((cat_name, cat_size))
        for cat_name, cat_size in sorted(cat_sizes, key=lambda x: -x[1]):
            if cat_size > 0:
                pct = cat_size / total_alloc * 100
                print(f"  {cat_name:<35} {fmt_size(cat_size):>12} {pct:>7.1f}%")

    # Symbol analysis
    output = run([nm_bin, "-D", so_path])
    symbols = parse_symbols(output)

    print(f"\n  Exported Symbols:")
    print(f"    Total dynamic symbols:  {symbols['total']}")
    print(f"    Text (functions):       {symbols['text']}")
    print(f"    JNI symbols:            {symbols['jni']}")
    print(f"    Non-JNI text symbols:   {len(symbols['non_jni_text'])}")

    if symbols["non_jni_text"]:
        # Group by prefix
        prefixes = {}
        for sym in symbols["non_jni_text"]:
            prefix = sym.split("_")[0] if "_" in sym else sym
            prefixes.setdefault(prefix, []).append(sym)

        print(f"\n    Non-JNI exported symbol groups (potential visibility leak):")
        for prefix, syms in sorted(prefixes.items(), key=lambda x: -len(x[1])):
            print(f"      {prefix:<30} {len(syms):>4} symbols")
            for s in syms[:3]:
                print(f"        - {s}")
            if len(syms) > 3:
                print(f"        ... and {len(syms) - 3} more")


def analyze_aar(aar_path, readelf_bin, nm_bin):
    """Analyze all .so files inside an AAR."""
    aar_size = os.path.getsize(aar_path)
    print(f"AAR: {aar_path}")
    print(f"AAR size: {fmt_size(aar_size)}")

    with zipfile.ZipFile(aar_path) as z:
        print(f"\nAAR Contents (>1KB):")
        print(f"  {'Compressed':>12} {'Original':>12}  {'Name'}")
        print(f"  {'-' * 50}")
        so_entries = []
        for info in sorted(z.infolist(), key=lambda x: -x.compress_size):
            if info.file_size > 1024:
                print(f"  {fmt_size(info.compress_size):>12} {fmt_size(info.file_size):>12}  {info.filename}")
            if info.filename.endswith(".so"):
                so_entries.append(info)

        if so_entries:
            tmpdir = tempfile.mkdtemp()
            try:
                for entry in so_entries:
                    z.extract(entry, tmpdir)
                    so_path = os.path.join(tmpdir, entry.filename)
                    analyze_so(so_path, readelf_bin, nm_bin)
            finally:
                shutil.rmtree(tmpdir, ignore_errors=True)


def main():
    repo_root = Path(__file__).resolve().parent.parent.parent
    default_aar = repo_root / "platforms" / "android" / "dist" / "migo-release.aar"

    target = sys.argv[1] if len(sys.argv) > 1 else str(default_aar)

    readelf_bin = find_tool("llvm-readelf")
    nm_bin = find_tool("llvm-nm")

    if not readelf_bin:
        print("ERROR: llvm-readelf not found. Add Android NDK to PATH.")
        sys.exit(1)
    if not nm_bin:
        print("ERROR: llvm-nm not found. Add Android NDK to PATH.")
        sys.exit(1)

    print(f"Using: {readelf_bin}")
    print(f"Using: {nm_bin}")
    print()

    if not os.path.exists(target):
        print(f"ERROR: {target} not found")
        sys.exit(1)

    if target.endswith(".aar") or target.endswith(".zip"):
        analyze_aar(target, readelf_bin, nm_bin)
    elif target.endswith(".so"):
        analyze_so(target, readelf_bin, nm_bin)
    else:
        print(f"ERROR: unsupported file type: {target}")
        sys.exit(1)

    # Optimization suggestions based on analysis
    print(f"\n{'=' * 65}")
    print(f"  Optimization Suggestions")
    print(f"{'=' * 65}")
    print("""
  1. [eh_frame] Add `-C link-arg=-Wl,--gc-sections` and consider
     `-C link-arg=-Wl,--icf=safe` to merge identical code/unwind info.
     Also add `RUSTFLAGS='-C force-unwind-tables=no'` if panic=abort.

  2. [symbols] Use a version script to hide non-JNI symbols:
     Create a linker script that only exports Java_* and JNI_OnLoad.

  3. [dependencies] Run `cargo bloat --release --crates` to identify
     which crates contribute most to binary size.

  4. [libc++_shared] Consider static linking libc++ to avoid shipping
     the separate .so (saves ~6.6MB compressed in AAR).

  5. [rodata] Check for large embedded strings/data. Use `llvm-strings`
     to find large constant strings that could be loaded at runtime.
""")


if __name__ == "__main__":
    main()
