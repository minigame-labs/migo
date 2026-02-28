param(
    [string]$BuildType = "release",
    [string]$OutputDir = "dist",
    [string[]]$Architectures = @("all"),
    [switch]$SkipRustBuild = $false,
    [switch]$Help = $false
)

# =========================
# Help
# =========================
if ($Help) {
    Write-Host "MiniGame Android AAR Builder"
    Write-Host ""
    Write-Host "Usage:"
    Write-Host "  .\build-aar.ps1 [-BuildType release|debug]"
    Write-Host "                    [-Architectures all|arm64-v8a,...]"
    Write-Host "                    [-SkipRustBuild]"
    exit 0
}

# =========================
# Path Resolution
# =========================
$ScriptDir  = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot   = Resolve-Path (Join-Path $ScriptDir "..")
$AndroidDir = Join-Path $RepoRoot "platforms/android"
$LibraryDir = Join-Path $AndroidDir "library"

$RustBuildScript     = Join-Path $ScriptDir "build-android-so.ps1"
$SnapshotBuildScript = Join-Path $ScriptDir "build-snapshot.ps1"
$ExternalJniLibs     = Join-Path $RepoRoot "engine/jniLibs"

Write-Host "========================================"
Write-Host "MiniGame Android AAR Builder"
Write-Host "========================================"
Write-Host "RepoRoot:   $RepoRoot"
Write-Host "AndroidDir: $AndroidDir"
Write-Host "LibraryDir: $LibraryDir"
Write-Host ""

# =========================
# Sanity Checks
# =========================
if (-not (Test-Path $AndroidDir)) {
    throw "Android directory not found: $AndroidDir"
}
if (-not (Test-Path $LibraryDir)) {
    throw "Android library module not found: $LibraryDir"
}
if (-not (Test-Path $RustBuildScript)) {
    throw "Rust build script not found: $RustBuildScript"
}

# =========================
# Build Rust (.so)
# =========================
function Build-RustLibrary {
    param([string[]]$TargetArchitectures)

    if ($SkipRustBuild) {
        Write-Host "Skipping Rust build"
        return
    }

    Write-Host "Building Rust Android .so..."

    Push-Location $ScriptDir
    try {
        foreach ($arch in $TargetArchitectures) {
            Write-Host "→ Rust build: $arch ($BuildType)"
            & $RustBuildScript $arch $BuildType

            if ($LASTEXITCODE -ne 0) {
                throw "Rust build failed for $arch"
            }
        }
    }
    finally {
        Pop-Location
    }

    Write-Host "✓ Rust build done"
}

# =========================
# Copy JNI Libraries
# =========================
function Copy-NativeLibraries {
    param([string[]]$TargetArchitectures)

    Write-Host "Copying JNI libraries..."

    $jniLibsDir = Join-Path $LibraryDir "src/main/jniLibs"

    if (Test-Path $jniLibsDir) {
        Remove-Item $jniLibsDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $jniLibsDir -Force | Out-Null

    if (-not (Test-Path $ExternalJniLibs)) {
        throw "jniLibs directory not found: $ExternalJniLibs"
    }

    if ($TargetArchitectures -ne @("all")) {
        foreach ($arch in $TargetArchitectures) {
            $src = Join-Path $ExternalJniLibs $arch
            $dst = Join-Path $jniLibsDir $arch

            if (-not (Test-Path $src)) {
                throw "Missing native libs for $arch"
            }

            New-Item -ItemType Directory -Path $dst -Force | Out-Null
            Copy-Item "$src/*" $dst -Force
            Write-Host "✓ $arch copied"
        }
    }
    else {
        Copy-Item "$ExternalJniLibs/*" $jniLibsDir -Recurse -Force
        Write-Host "✓ All architectures copied"
    }
}

# =========================
# Build AAR
# =========================
function Build-AAR {
    Write-Host "Building AAR..."

    Push-Location $AndroidDir
    try {
        if (Test-Path ".\gradlew.bat") {
            $gradle = ".\gradlew.bat"
        } elseif (Get-Command gradle -ErrorAction SilentlyContinue) {
            $gradle = "gradle"
        } else {
            throw "Gradle not found"
        }

        & $gradle clean
        if ($LASTEXITCODE -ne 0) { throw "Gradle clean failed" }

        if ($BuildType -eq "release") {
            & $gradle assembleRelease
        } else {
            & $gradle assembleDebug
        }

        if ($LASTEXITCODE -ne 0) {
            throw "AAR build failed"
        }
    }
    finally {
        Pop-Location
    }

    Write-Host "✓ AAR build success"
}

# =========================
# Collect Outputs
# =========================
function Collect-Outputs {
    Write-Host "Collecting outputs..."

    $outDir = Join-Path $AndroidDir $OutputDir
    if (Test-Path $outDir) {
        Remove-Item $outDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null

    $aarDir = Join-Path $LibraryDir "build/outputs/aar"
    Copy-Item "$aarDir/*" $outDir -Force

    @{
        buildType = $BuildType
        buildTime = (Get-Date -Format "yyyy-MM-dd HH:mm:ss")
    } | ConvertTo-Json | Out-File (Join-Path $outDir "version.json") -Encoding utf8

    Write-Host "✓ Outputs ready: $outDir"
}

# =========================
# Generate V8 Snapshot (currently disabled)
# =========================
# Snapshot generation is disabled because the Android V8 is a custom
# termux-packages build incompatible with the official rusty_v8 releases.
# When a compatible V8 build is available, uncomment the Build-Snapshot call.
#
# function Build-Snapshot {
#     if ($BuildType -ne "release") {
#         Write-Host "Skipping snapshot generation (debug build)"
#         return
#     }
#     Write-Host "Generating V8 snapshot for release build..."
#     if (-not (Test-Path $SnapshotBuildScript)) {
#         throw "Snapshot build script not found: $SnapshotBuildScript"
#     }
#     & $SnapshotBuildScript
#     if ($LASTEXITCODE -ne 0) {
#         throw "V8 snapshot generation failed"
#     }
#     Write-Host "V8 snapshot generated"
# }

# =========================
# Main
# =========================
# Build-Snapshot  # Disabled — see comment above
Build-RustLibrary -TargetArchitectures $Architectures
Copy-NativeLibraries -TargetArchitectures $Architectures
Build-AAR
Collect-Outputs

Write-Host ""
Write-Host "========================================"
Write-Host "✅ Android AAR build completed"
Write-Host "========================================"
