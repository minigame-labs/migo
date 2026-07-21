<#
.SYNOPSIS
  Compile the C ABI layout lanes with MSVC, for both Windows ABIs.

.DESCRIPTION
  The lanes under tests/c_abi are compiler- and ABI-sensitive by design, and
  every other lane runs under GCC or Clang on a SysV target. This one covers
  what those cannot reach:

    * LLP64. Windows x64 is the only supported ABI where a pointer is 64 bits
      and `long` is 32. The headers use fixed-width types throughout, so this
      is meant to pass -- but "meant to" is the state every other gap in this
      file was in before it was compiled.
    * The `__declspec` half of MIGO_API. types.h picks dllexport, dllimport or
      nothing from MIGO_BUILD_SHARED / MIGO_USE_SHARED, and outside Windows all
      three collapse to the GNU visibility attribute. Each is compiled here.
    * MSVC's own C dialect under /std:c11 /W4 /WX /permissive-, which accepts a
      visibly different subset from GCC's -std=c11 -Wall -Wextra -Werror.
    * x86, where __cdecl is a real calling convention rather than the only one.

  This does not build the engine: the lanes compile without linking, and
  migo-capi-abi has no dependencies, so no V8, Skia or ANGLE is involved.

.NOTES
  Requires Visual Studio Build Tools 2022 with the C++ workload. Run from the
  repository root, or pass -RepoRoot.
#>
[CmdletBinding()]
param(
    [string]$RepoRoot = (Resolve-Path "$PSScriptRoot\..").Path
)

$ErrorActionPreference = 'Stop'

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path $vswhere)) {
    Write-Error "vswhere not found at $vswhere -- install Visual Studio Build Tools 2022 with the C++ workload"
}

$vsRoot = & $vswhere -products '*' `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
    -latest -format value -property installationPath
if (-not $vsRoot) {
    Write-Error "no Visual Studio install carries the MSVC x86/x64 build tools"
}

$include = Join-Path $RepoRoot 'include'
$lanes = @(
    'core_contract.c'
    'old_client_contract.c'
    'old_client_outbound_contract.c'
    'platform_contract.c'
)
$cppLanes = @('core_contract.cc', 'platform_contract.cc')
# Absent means MIGO_API expands to nothing, which is how a host links the
# static library; the two shared-library spellings are the branches no
# non-Windows compiler ever reaches.
$apiModes = @('', 'MIGO_BUILD_SHARED', 'MIGO_USE_SHARED')

# Every path used below is already absolute, and cmd.exe refuses to start in a
# UNC working directory -- it prints a warning and falls back to C:\Windows,
# which surfaces as a NativeCommandError rather than as the compile result.
# Running from a local directory removes the warning at the source. A WSL
# checkout reached as \\wsl.localhost\... is the normal case here.
Set-Location -LiteralPath $env:TEMP

# The compiles below report through $LASTEXITCODE, which is checked at every
# call site; leaving this at Stop would turn any warning MSVC writes to stderr
# into a terminating error and hide the exit code that actually matters.
$ErrorActionPreference = 'Continue'

$failed = 0
foreach ($arch in @('x64', 'x86')) {
    $vcvars = Join-Path $vsRoot "VC\Auxiliary\Build\vcvars$(if ($arch -eq 'x64') { '64' } else { '32' }).bat"
    if (-not (Test-Path $vcvars)) {
        Write-Error "vcvars script not found: $vcvars"
    }

    foreach ($mode in $apiModes) {
        $define = if ($mode) { "/D$mode" } else { '' }
        $label = if ($mode) { $mode } else { 'static (MIGO_API empty)' }

        foreach ($lane in $lanes) {
            $source = Join-Path $RepoRoot "tests\c_abi\$lane"
            # cmd carries the vcvars environment into cl; `&&` keeps a failed
            # environment setup from being reported as a passing compile. The
            # leading `cd` matters when the repository lives on a UNC path --
            # a WSL checkout reached as \\wsl.localhost\... is the normal case
            # here -- because cmd refuses a UNC working directory and would
            # otherwise fail before running anything. Every path below is
            # absolute, so the working directory is irrelevant to the compile.
            $cmd = "cd /d `"$env:TEMP`" && `"$vcvars`" && cl /nologo /c /std:c11 /W4 /WX /permissive- $define /I`"$include`" /Fo:NUL `"$source`""
            $out = & cmd.exe /c $cmd 2>&1
            if ($LASTEXITCODE -ne 0) {
                Write-Host "FAIL  $arch  $label  $lane" -ForegroundColor Red
                $out | ForEach-Object { Write-Host "    $_" }
                $failed = 1
            } else {
                Write-Host "pass  $arch  $label  $lane"
            }
        }

        foreach ($lane in $cppLanes) {
            $source = Join-Path $RepoRoot "tests\c_abi\$lane"
            $cmd = "cd /d `"$env:TEMP`" && `"$vcvars`" && cl /nologo /c /std:c++17 /W4 /WX /permissive- /EHsc $define /I`"$include`" /Fo:NUL `"$source`""
            $out = & cmd.exe /c $cmd 2>&1
            if ($LASTEXITCODE -ne 0) {
                Write-Host "FAIL  $arch  $label  $lane" -ForegroundColor Red
                $out | ForEach-Object { Write-Host "    $_" }
                $failed = 1
            } else {
                Write-Host "pass  $arch  $label  $lane"
            }
        }
    }
}

if ($failed -ne 0) {
    Write-Host "C ABI MSVC lane: FAIL" -ForegroundColor Red
    exit 1
}
Write-Host "C ABI MSVC lane: PASS (x64 + x86, all MIGO_API modes)" -ForegroundColor Green
