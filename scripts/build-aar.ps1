param(
    [ValidateSet("release", "debug")]
    [string]$BuildType = "release",
    [ValidateSet("full", "slim")]
    [string]$ProductProfile = "full",
    [ValidateSet("z", "2", "3")]
    [string]$CodegenProfile = "z",
    [string]$OutputDir = "dist",
    [string[]]$Architectures = @("all"),
    [switch]$SkipRustBuild = $false,
    [switch]$WorkerSnapshot = $false,
    [switch]$Help = $false
)

# =========================
# Help
# =========================
if ($Help) {
    Write-Host "MiniGame Android AAR Builder"
    Write-Host ""
    Write-Host "Usage:"
    Write-Host "  .\build-aar.ps1 [-BuildType release|debug] [-CodegenProfile z|2|3]"
    Write-Host "                    [-Architectures all|arm64-v8a,...]"
    Write-Host "                    [-SkipRustBuild] [-WorkerSnapshot]"
    exit 0
}

# =========================
# Path Resolution
# =========================
$ScriptDir  = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot   = Resolve-Path (Join-Path $ScriptDir "..")
$AndroidDir = Join-Path $RepoRoot "platforms/android"
$LibraryDir = Join-Path $AndroidDir "library"

if ($BuildType -eq "debug" -and $CodegenProfile -ne "z") {
    throw "Codegen profile $CodegenProfile requires a release build"
}
if ($WorkerSnapshot.IsPresent -and ($BuildType -ne "release" -or $ProductProfile -ne "full")) {
    throw "Worker snapshot requires a full release build"
}

$SourceDateEpoch = $env:SOURCE_DATE_EPOCH
$SourceDateEpochMetadata = $null
if (-not [string]::IsNullOrEmpty($SourceDateEpoch)) {
    [long]$SourceDateEpochSeconds = 0
    if ($SourceDateEpoch -notmatch '^[0-9]+$' -or
            -not [long]::TryParse($SourceDateEpoch, [ref]$SourceDateEpochSeconds) -or
            $SourceDateEpochSeconds -gt 9223372036854775) {
        throw "Invalid SOURCE_DATE_EPOCH: expected non-negative Unix seconds that fit in milliseconds"
    }
    $SourceDateEpochMetadata = $SourceDateEpoch
}

$CodegenSuffix = ""
$CargoProfile = "debug"
if ($BuildType -eq "release") {
    switch ($CodegenProfile) {
        "z" { $CargoProfile = "release" }
        "2" {
            $CargoProfile = "release-hot2"
            $CodegenSuffix = "-opt2"
        }
        "3" {
            $CargoProfile = "release-hot3"
            $CodegenSuffix = "-opt3"
        }
    }
}
$WorkerSnapshotSuffix = if ($WorkerSnapshot.IsPresent) { "-worker-snapshot" } else { "" }
$ArtifactSuffix = "$CodegenSuffix$WorkerSnapshotSuffix"

$RustBuildScript     = Join-Path $ScriptDir "build-android-so.ps1"
$SnapshotBuildScript = Join-Path $ScriptDir "build-snapshot.ps1"
$ExternalJniLibs     = Join-Path $RepoRoot "engine/jniLibs/$ProductProfile$ArtifactSuffix"

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
            Write-Host "→ Rust build: $arch ($BuildType, codegen=$CodegenProfile)"
            $rustArgs = @(
                $arch,
                $BuildType,
                "--product-profile=$ProductProfile",
                "--codegen-profile=$CodegenProfile"
            )
            if ($WorkerSnapshot.IsPresent) {
                $rustArgs += "--worker-snapshot"
            }
            & $RustBuildScript @rustArgs

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
# Validate JNI Libraries
# =========================
function Test-NativeLibraries {
    param([string[]]$TargetArchitectures)

    Write-Host "Validating $ProductProfile JNI libraries..."

    if (-not (Test-Path $ExternalJniLibs)) {
        throw "jniLibs directory not found: $ExternalJniLibs"
    }

    foreach ($arch in $TargetArchitectures) {
        $src = Join-Path $ExternalJniLibs $arch
        if (-not (Test-Path $src)) {
            throw "Missing native libs for $ProductProfile/$arch"
        }
        foreach ($library in @("libmigo.so", "libc++_shared.so")) {
            if (-not (Test-Path (Join-Path $src $library))) {
                throw "Missing $ProductProfile/$arch/$library"
            }
        }
        Write-Host "✓ $ProductProfile/$arch ready"
    }
}

# =========================
# Build AAR
# =========================
function Build-AAR {
    param([string[]]$TargetArchitectures)

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

        $profileTask = (Get-Culture).TextInfo.ToTitleCase($ProductProfile)
        $typeTask = (Get-Culture).TextInfo.ToTitleCase($BuildType)
        $abiProperty = "-PmigoAbis=" + ($TargetArchitectures -join ",")
        $codegenProperty = "-PmigoCodegenProfile=$CodegenProfile"
        $workerSnapshotProperty = "-PmigoWorkerSnapshot=$($WorkerSnapshot.IsPresent.ToString().ToLowerInvariant())"
        & $gradle $abiProperty $codegenProperty $workerSnapshotProperty "assemble$profileTask$typeTask"

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
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null

    $aarDir = Join-Path $LibraryDir "build/outputs/aar"
    $aar = Join-Path $aarDir "migo-$ProductProfile-$BuildType.aar"
    if (-not (Test-Path $aar)) { throw "Expected AAR not found: $aar" }
    $artifactName = "migo-$ProductProfile-$BuildType$ArtifactSuffix.aar"
    Copy-Item $aar (Join-Path $outDir $artifactName) -Force

    @{
        productProfile = $ProductProfile
        buildType = $BuildType
        codegenProfile = $CodegenProfile
        cargoProfile = $CargoProfile
        workerSnapshot = $WorkerSnapshot.IsPresent
        sourceDateEpoch = $SourceDateEpochMetadata
        buildTime = (Get-Date -Format "yyyy-MM-dd HH:mm:ss")
    } | ConvertTo-Json | Out-File (Join-Path $outDir "version-$ProductProfile$ArtifactSuffix.json") -Encoding utf8

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
$SupportedArchitectures = @("arm64-v8a", "x86_64")
if ($Architectures -contains "all") {
    $ResolvedArchitectures = $SupportedArchitectures
}
else {
    $ResolvedArchitectures = @($Architectures | Select-Object -Unique)
    foreach ($arch in $ResolvedArchitectures) {
        if ($SupportedArchitectures -notcontains $arch) {
            throw "Unsupported architecture: $arch"
        }
    }
}

Build-RustLibrary -TargetArchitectures $ResolvedArchitectures
Test-NativeLibraries -TargetArchitectures $ResolvedArchitectures
Build-AAR -TargetArchitectures $ResolvedArchitectures
Collect-Outputs

Write-Host ""
Write-Host "========================================"
Write-Host "✅ Android AAR build completed"
Write-Host "========================================"
