# ============================================================
# Android dynamic library build script
# Location: engine/scripts/build-android.ps1
#
# Usage:
#   ./build-android.ps1
#   ./build-android.ps1 arm64-v8a
#   ./build-android.ps1 arm64-v8a x86_64 release
#   ./build-android.ps1 all release
# ============================================================

param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Args
)

# ------------------------------------------------------------
# Constants
# ------------------------------------------------------------
$ANDROID_API = 21

$CRATE_NAME = "platform"
$CRATE_DIR  = "crates/$CRATE_NAME"

# cargo output naming rule: lib{crate}.so
$CRATE_SO_NAME = "lib$CRATE_NAME.so"

$OUTPUT_SO_NAME = "libminigame_host.so"

$PLATFORM_MAP = @{
    "arm64-v8a" = "aarch64-linux-android"
    "x86_64"    = "x86_64-linux-android"
    "all"       = "all"
}

# ------------------------------------------------------------
# Logging helpers
# ------------------------------------------------------------
function Print-Info    { param($m) Write-Host "[INFO] $m"    -ForegroundColor Cyan }
function Print-Success { param($m) Write-Host "[SUCCESS] $m" -ForegroundColor Green }
function Print-Warning { param($m) Write-Host "[WARNING] $m" -ForegroundColor Yellow }
function Print-Error   { param($m) Write-Host "[ERROR] $m"   -ForegroundColor Red }

function Resolve-Paths {
    $projectRoot = Split-Path $PSScriptRoot -Parent

    $engineRoot = Join-Path $projectRoot "engine"

    if (-not (Test-Path $engineRoot)) {
        Print-Error "engine directory not found at $engineRoot"
        exit 1
    }

    return @{
        Project   = $projectRoot
        Engine    = $engineRoot
        Crate     = Join-Path $engineRoot $CRATE_DIR
        Target    = Join-Path $engineRoot "target"
        JniLibs   = Join-Path $engineRoot "jniLibs"
        v8Libs    = Join-Path $engineRoot "third_party/rusty_v8"
        Platforms = Join-Path $engineRoot "platforms"
    }
}

# ------------------------------------------------------------
# Dependency check
# ------------------------------------------------------------
function Check-Dependencies {
    Print-Info "Checking dependencies..."

    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Print-Error "cargo not found"
        exit 1
    }

    if (-not (Get-Command cargo-ndk -ErrorAction SilentlyContinue)) {
        Print-Error "cargo-ndk not found (install with: cargo install cargo-ndk)"
        exit 1
    }

    if (-not $env:ANDROID_NDK_HOME) {
        Print-Error "ANDROID_NDK_HOME is not set"
        exit 1
    }

    Print-Success "All dependencies are ready"
}

# ------------------------------------------------------------
# ABI mapping
# ------------------------------------------------------------
function Get-AbiName($platform) {
    switch ($platform) {
        "arm64-v8a" { "arm64-v8a" }
        "x86_64"    { "x86_64" }
        default     { "unknown" }
    }
}

# ------------------------------------------------------------
# Locate arm64 clang builtins
# ------------------------------------------------------------
function Find-Arm64Builtins {
    $prebuilt = Join-Path $env:ANDROID_NDK_HOME "toolchains\llvm\prebuilt"
    if (-not (Test-Path $prebuilt)) {
        return $null
    }

    Get-ChildItem $prebuilt -Directory | ForEach-Object {
        $clangRoot = Join-Path $_.FullName "lib64\clang"
        if (Test-Path $clangRoot) {
            Get-ChildItem $clangRoot -Directory | ForEach-Object {
                Get-ChildItem $_.FullName -Recurse `
                    -Filter "libclang_rt.builtins-aarch64-android.a" `
                    -ErrorAction SilentlyContinue |
                    Select-Object -First 1
            }
        }
    } | Select-Object -First 1
}

# ------------------------------------------------------------
# Build one platform
# ------------------------------------------------------------
function Build-Platform {
    param(
        [string]$platform,
        [string]$buildType,
        [hashtable]$paths
    )

    if ($platform -in @("armv7", "x86")) {
        Print-Error "$platform is not supported"
        return $false
    }

    $targetTriple = $PLATFORM_MAP[$platform]
    $profileFlag  = if ($buildType -eq "release") { "--release" } else { "" }
    $outDir       = if ($buildType -eq "release") { "release" } else { "debug" }

    # --------------------------------------------------------
    # Rusty V8 config
    # --------------------------------------------------------
    $arch = if ($platform -eq "arm64-v8a") { "aarch64" } else { "x86_64" }

    $v8Archive = Join-Path $paths.v8Libs "$arch\librusty_v8.a"
    $v8Binding = Join-Path $paths.v8Libs "$arch\src_binding.rs"

    if (Test-Path $v8Archive) {
        $env:RUSTY_V8_ARCHIVE = $v8Archive
        Print-Info "RUSTY_V8_ARCHIVE = $v8Archive"
    }

    if (Test-Path $v8Binding) {
        $env:RUSTY_V8_SRC_BINDING_PATH = $v8Binding
        Print-Info "RUSTY_V8_SRC_BINDING_PATH = $v8Binding"
    }

    # --------------------------------------------------------
    # RUSTFLAGS (arm64 builtins)
    # --------------------------------------------------------
    $origRUSTFLAGS = $env:RUSTFLAGS

    if ($platform -eq "arm64-v8a") {
        $builtins = Find-Arm64Builtins
        if (-not $builtins) {
            Print-Error "libclang_rt.builtins-aarch64-android.a not found"
            return $false
        }

        $builtinsDir = Split-Path $builtins -Parent
        $env:RUSTFLAGS = "$origRUSTFLAGS -L $builtinsDir -l static=clang_rt.builtins-aarch64-android"

        Print-Info "Using arm64 clang builtins"
    }

    # --------------------------------------------------------
    # Build
    # --------------------------------------------------------
    Print-Info "Building $platform ($targetTriple) [$buildType]"

    Push-Location $paths.Crate

    $cargoArgs = @(
        "ndk",
        "--target", $targetTriple,
        "--platform", $ANDROID_API,
        "--",
        "build"
    )
    if ($profileFlag) { $cargoArgs += $profileFlag }

    $proc = Start-Process `
        -FilePath "cargo" `
        -ArgumentList $cargoArgs `
        -Wait `
        -NoNewWindow `
        -PassThru

    Pop-Location

    if ($proc.ExitCode -ne 0) {
        Print-Error "Build failed for $platform"
        $env:RUSTFLAGS = $origRUSTFLAGS
        return $false
    }

    # --------------------------------------------------------
    # Copy output .so
    # --------------------------------------------------------
    $abi     = Get-AbiName $platform
    $dstDir = Join-Path $paths.JniLibs $abi

    if (-not (Test-Path $dstDir)) {
        New-Item -ItemType Directory -Path $dstDir | Out-Null
    }

    $srcSo = Join-Path $paths.Target "$targetTriple\$outDir\$CRATE_SO_NAME"
    $dstSo = Join-Path $dstDir $OUTPUT_SO_NAME

    if (Test-Path $srcSo) {
        Copy-Item $srcSo $dstSo -Force
        Print-Success "Copied -> $dstSo"
    } else {
        Print-Warning "Output .so not found: $srcSo"
    }

    # --------------------------------------------------------
    # Copy libc++_shared.so (required by cpal/oboe)
    # --------------------------------------------------------
    $libcppSrc = Join-Path $env:ANDROID_NDK_HOME "toolchains\llvm\prebuilt\windows-x86_64\sysroot\usr\lib\$targetTriple\libc++_shared.so"
    $libcppDst = Join-Path $dstDir "libc++_shared.so"

    if (Test-Path $libcppSrc) {
        Copy-Item $libcppSrc $libcppDst -Force
        Print-Success "Copied -> $libcppDst"
    } else {
        Print-Warning "libc++_shared.so not found: $libcppSrc"
    }

    $env:RUSTFLAGS = $origRUSTFLAGS
    return $true
}

# ------------------------------------------------------------
# Main
# ------------------------------------------------------------
Check-Dependencies
$paths = Resolve-Paths

$buildType = "debug"
$platforms = @()

foreach ($arg in $Args) {
    if ($arg -eq "release") {
        $buildType = "release"
    } elseif ($PLATFORM_MAP.ContainsKey($arg)) {
        if ($arg -eq "all") {
            $platforms = @("arm64-v8a", "x86_64")
            break
        } else {
            $platforms += $arg
        }
    }
}

if ($platforms.Count -eq 0) {
    $platforms = @("arm64-v8a", "x86_64")
    Print-Info "No platform specified, building default ABIs"
}

Print-Info "Build type : $buildType"
Print-Info "Platforms  : $($platforms -join ', ')"

$failed = @()
foreach ($p in $platforms) {
    if (-not (Build-Platform $p $buildType $paths)) {
        $failed += $p
    }
}

if ($failed.Count -eq 0) {
    Print-Success "All Android builds succeeded"
    exit 0
} else {
    Print-Error "Failed platforms: $($failed -join ', ')"
    exit 1
}
