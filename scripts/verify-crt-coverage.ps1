# Verify that every DLL imported by the shipped Windows executables is
# covered: either OS-provided (allow-list) or staged app-locally in redist\
# by scripts/build-msi.ps1.
#
# This is the guard that would have caught the v0.1.22 field failure
# ("VCRUNTIME140_1.dll was not found" on a clean Windows 10 VM): the release
# binaries were dynamically linking the MSVC CRT while the installer carried
# none of the redist DLLs. If a future binary (strikehub or a bundled
# connector) starts importing a new non-OS DLL (e.g. MSVCP140.dll after a
# C++ stdlib dependency lands), CI fails until the redist\ staging set in
# scripts/build-msi.ps1 (and wix\main.wxs) is extended to cover it.
#
# Runs in CI after the MSI build, from the repo root. Requires dumpbin
# (present on the GitHub windows-latest runners with VS BuildTools).
param(
    # Directory holding the executables to check (CI: dist\).
    [string]$ExeDir = "dist",
    # Directory holding the staged app-local redist DLLs (CI: redist\).
    [string]$RedistDir = "redist"
)

$ErrorActionPreference = "Stop"

$dumpbin = Get-ChildItem "C:\Program Files (x86)\Microsoft Visual Studio\2022\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" -ErrorAction SilentlyContinue |
    Sort-Object FullName -Descending | Select-Object -First 1
if (-not $dumpbin) {
    Write-Host "ERROR: dumpbin not found under VS 2022; cannot verify CRT coverage." -ForegroundColor Red
    exit 1
}
Write-Host "Using dumpbin: $($dumpbin.FullName)"

# Extract the set of imported DLL names (regular + delay-load sections) from
# `dumpbin /dependents` output. dumpbin lists them as bare indented names
# under "Image has the following [delay load] dependencies:" - the format has
# no per-line label, so section state is tracked line by line.
function Get-ExeDllImports {
    param([string]$ExePath, [string]$DumpbinExe)
    $lines = & $DumpbinExe /dependents $ExePath 2>$null
    $found = @{}
    $inDepsSection = $false
    foreach ($line in $lines) {
        if ($line -match 'following dependencies:') {
            $inDepsSection = $true
            continue
        }
        if ($line -match '^\s*Summary' ) {
            $inDepsSection = $false
            continue
        }
        # Tolerate the labeled variant too (older dumpbin / other tools).
        if ($line -match '^\s*DLL Name:\s*(\S+\.dll)') {
            $found[$Matches[1].ToLowerInvariant()] = $true
            continue
        }
        if ($inDepsSection -and $line -match '^\s{2,}(\S+\.dll)\s*$') {
            $found[$Matches[1].ToLowerInvariant()] = $true
        }
    }
    return @($found.Keys | Sort-Object -Unique)
}

# DLLs provided by Windows itself (any supported Windows 10/11). The
# api-ms-win-* forwarders resolve to OS API sets / the universal CRT, which
# every Windows 10 machine ships. Extend this list only for OS-provided
# DLLs, and say why in a comment.
$osProvided = @(
    "advapi32.dll",
    "api-ms-win-*",
    "bcrypt.dll",
    "bcryptprimitives.dll",
    "comctl32.dll",
    "crypt32.dll",
    "dwmapi.dll",
    "gdi32.dll",
    "iphlpapi.dll",
    "kernel32.dll",
    "mswsock.dll",
    "ntdll.dll",
    "ole32.dll",
    "oleaut32.dll",
    "pdh.dll",
    "powrprof.dll",
    "psapi.dll",
    "secur32.dll",
    "shell32.dll",
    "shlwapi.dll",
    "user32.dll",
    "uxtheme.dll",
    "wlanapi.dll",
    "ws2_32.dll"
)

$staged = @(Get-ChildItem (Join-Path $RedistDir "*.dll") -ErrorAction SilentlyContinue |
    ForEach-Object { $_.Name.ToLowerInvariant() })
Write-Host ("Staged app-local redist DLLs: {0}" -f ($staged -join ", "))
if ($staged.Count -eq 0) {
    Write-Host "ERROR: no staged redist DLLs found in $RedistDir\" -ForegroundColor Red
    exit 1
}

$missing = @()
$exes = @(Get-ChildItem (Join-Path $ExeDir "*.exe"))
if ($exes.Count -eq 0) {
    Write-Host "ERROR: no executables found in $ExeDir\" -ForegroundColor Red
    exit 1
}
foreach ($exe in $exes) {
    $deps = Get-ExeDllImports -ExePath $exe.FullName -DumpbinExe $dumpbin.FullName
    Write-Host ""
    Write-Host ("=== {0}: {1} import(s) ===" -f $exe.Name, $deps.Count)
    if ($deps.Count -eq 0) {
        # Fail closed: a PE with a readable /dependents output but zero
        # imports means the parser saw something it did not expect.
        $missing += ($exe.Name + ": (no imports parsed - dumpbin output changed?)")
        continue
    }
    foreach ($d in $deps) {
        $covered = $null
        foreach ($pat in $osProvided) {
            if ($d -like $pat) { $covered = "OS"; break }
        }
        if (-not $covered -and $d -in $staged) { $covered = "app-local" }
        if (-not $covered) {
            $missing += "{0}: {1}" -f $exe.Name, $d
            Write-Host ("  MISSING-COVERAGE  {0}" -f $d) -ForegroundColor Red
        } else {
            Write-Host ("  {0,-28} {1}" -f $d, $covered)
        }
    }
}

Write-Host ""
if ($missing.Count -gt 0) {
    Write-Host "UNCOVERED DLL IMPORTS: the MSI would ship executables that cannot load on a clean Windows machine." -ForegroundColor Red
    foreach ($m in $missing) { Write-Host "  $m" -ForegroundColor Red }
    Write-Host "Fix: add the DLL to the redist\ staging in scripts\build-msi.ps1 (+ wix\main.wxs File elements), or to the OS allow-list above if it is OS-provided." -ForegroundColor Yellow
    exit 1
}
Write-Host "OK: every exe import is covered by the OS allow-list or the staged app-local redist." -ForegroundColor Green
exit 0
