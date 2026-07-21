# ============================================================
# V8 Snapshot Generator
#
# Builds and runs migo-snapshot-gen on the host platform (Windows)
# to produce SNAPSHOT.bin for embedding into the Android runtime.
#
# Usage:
#   .\build-snapshot.ps1            # build + generate snapshot
#   .\build-snapshot.ps1 -Clean     # clean build artifacts first
#
# Output:
#   engine/crates/runtime-v8/SNAPSHOT.bin
#
# Notes:
#   - Requires a host-platform V8 static library.  If RUSTY_V8_ARCHIVE
#     is set to an Android archive, this script temporarily unsets it
#     so that the v8 crate downloads the correct Windows build.
#   - The snapshot must be regenerated whenever JS extension source,
#     deno_core version, or extension registration order changes.
# ============================================================

param(
    [switch]$Clean = $false,
    [switch]$Help = $false
)

if ($Help) {
    Write-Host "V8 Snapshot Generator"
    Write-Host ""
    Write-Host "Usage:"
    Write-Host "  .\build-snapshot.ps1           # build + generate snapshot"
    Write-Host "  .\build-snapshot.ps1 -Clean    # clean first, then build"
    exit 0
}

# ------------------------------------------------------------
# Logging
# ------------------------------------------------------------
function Print-Info    { param($m) Write-Host "[INFO] $m"    -ForegroundColor Cyan }
function Print-Success { param($m) Write-Host "[SUCCESS] $m" -ForegroundColor Green }
function Print-Error   { param($m) Write-Host "[ERROR] $m"   -ForegroundColor Red }

# ------------------------------------------------------------
# Paths
# ------------------------------------------------------------
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot  = Resolve-Path (Join-Path $ScriptDir "..")
$EngineDir = Join-Path $RepoRoot "engine"
$SnapshotBin = Join-Path $EngineDir "crates/runtime-v8/SNAPSHOT.bin"

if (-not (Test-Path $EngineDir)) {
    Print-Error "Engine directory not found: $EngineDir"
    exit 1
}

# ------------------------------------------------------------
# Save and clear Android-specific V8 env vars
# ------------------------------------------------------------
# When building for Android, RUSTY_V8_ARCHIVE points to the
# aarch64 static library. We must clear it so the v8 crate
# downloads/uses the Windows host build instead.
$savedV8Archive = $env:RUSTY_V8_ARCHIVE
$savedV8Binding = $env:RUSTY_V8_SRC_BINDING_PATH

$env:RUSTY_V8_ARCHIVE = $null
$env:RUSTY_V8_SRC_BINDING_PATH = $null

# Also check if the Windows V8 archive is pre-cached
$windowsV8Dir = Join-Path $EngineDir "third_party/rusty_v8/x86_64-windows"
$windowsV8Archive = Join-Path $windowsV8Dir "librusty_v8.a"
if (Test-Path $windowsV8Archive) {
    $env:RUSTY_V8_ARCHIVE = $windowsV8Archive
    Print-Info "Using cached Windows V8 archive: $windowsV8Archive"
}

try {
    # --------------------------------------------------------
    # Clean (optional)
    # --------------------------------------------------------
    if ($Clean) {
        Print-Info "Cleaning snapshot-gen build artifacts..."
        Push-Location $EngineDir
        cargo clean -p migo-snapshot-gen 2>$null
        Pop-Location
    }

    # --------------------------------------------------------
    # Remove old snapshot so build.rs detects the change
    # --------------------------------------------------------
    if (Test-Path $SnapshotBin) {
        Print-Info "Removing old SNAPSHOT.bin..."
        Remove-Item $SnapshotBin -Force
    }

    # --------------------------------------------------------
    # Build and run snapshot generator
    # --------------------------------------------------------
    Print-Info "Building and running migo-snapshot-gen (host platform)..."

    Push-Location $EngineDir
    cargo run -p migo-snapshot-gen

    $exitCode = $LASTEXITCODE
    Pop-Location

    if ($exitCode -ne 0) {
        Print-Error "Snapshot generation failed (exit code: $exitCode)"
        exit 1
    }

    # --------------------------------------------------------
    # Verify output
    # --------------------------------------------------------
    if (-not (Test-Path $SnapshotBin)) {
        Print-Error "SNAPSHOT.bin was not created"
        exit 1
    }

    $size = (Get-Item $SnapshotBin).Length
    Print-Success "SNAPSHOT.bin generated ($size bytes, $([math]::Round($size / 1024, 1)) KB)"
    Print-Success "Path: $SnapshotBin"
}
finally {
    # Restore original env vars
    $env:RUSTY_V8_ARCHIVE = $savedV8Archive
    $env:RUSTY_V8_SRC_BINDING_PATH = $savedV8Binding
}
