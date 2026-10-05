param(
    [string]$Version = "0.1.0",
    [string]$Arch = "x86_64",
    # Pick release to bundle when the pentest-agent binary is not pre-staged in
    # dist/ (local/offline builds). CI builds pentest-agent from source instead,
    # so this only applies to manual builds. Empty means "read PICK_REF from
    # connector-versions.env" below, so this default can't drift from the pipeline.
    [string]$PickVersion = ""
)

$ErrorActionPreference = "Stop"

# Default the Pick version to the release tag pinned in connector-versions.env
# (the same file CI/release reads) unless the caller passed -PickVersion.
if (-not $PickVersion) {
    $PickVersion = "v0.1.10"
    $verFile = Join-Path $PSScriptRoot "..\connector-versions.env"
    if (Test-Path $verFile) {
        $m = Select-String -Path $verFile -Pattern '^\s*PICK_REF\s*=\s*(.+?)\s*$'
        if ($m) { $PickVersion = $m.Matches[0].Groups[1].Value }
    }
}

# Map arch to Rust target triple and WiX arch identifier
switch ($Arch) {
    "x86_64"  { $Target = "x86_64-pc-windows-msvc";  $WixArch = "x64";   $WixPlatform = "x64" }
    "aarch64" { $Target = "aarch64-pc-windows-msvc";  $WixArch = "arm64"; $WixPlatform = "arm64" }
    default   { Write-Host "Unsupported arch: $Arch" -ForegroundColor Red; exit 1 }
}

Write-Host "Building StrikeHub MSI installer v$Version ($Arch)" -ForegroundColor Cyan
Write-Host "==========================================="

# Check WiX is installed
$wixDir = $null
$candidates = @(
    "${env:WIX}bin",
    "C:\Program Files (x86)\WiX Toolset v3.14\bin",
    "C:\Program Files (x86)\WiX Toolset v3.11\bin"
)
foreach ($d in $candidates) {
    if (Test-Path "$d\candle.exe") { $wixDir = $d; break }
}
if (-not $wixDir) {
    # Try PATH
    if (Get-Command candle.exe -ErrorAction SilentlyContinue) {
        $wixDir = Split-Path (Get-Command candle.exe).Source
    } else {
        Write-Host "ERROR: WiX Toolset not found." -ForegroundColor Red
        Write-Host "Install: choco install wixtoolset  (or https://wixtoolset.org)" -ForegroundColor Yellow
        exit 1
    }
}
Write-Host "WiX: $wixDir"

# Build the release binary if needed
$exe = "target\$Target\release\strikehub.exe"
if (-not (Test-Path $exe)) {
    Write-Host "Building release binary..." -ForegroundColor Yellow
    cargo build --release --target $Target --no-default-features --features desktop
    if ($LASTEXITCODE -ne 0) { exit 1 }
}

# Ensure strikehub.exe is in dist/ (CI signs it there; local builds copy it)
New-Item -ItemType Directory -Path dist -Force | Out-Null
if (-not (Test-Path "dist\strikehub.exe")) {
    Write-Host "Copying strikehub.exe to dist/..." -ForegroundColor Yellow
    Copy-Item $exe "dist\strikehub.exe"
}

# -- Stage the VC++ 2015-2022 runtime DLLs (app-local deployment) ----------
# strikehub.exe and the bundled connectors are built with the default MSVC
# dynamic CRT, so they import VCRUNTIME140.dll (all) and VCRUNTIME140_1.dll
# (strikehub) - verified with `dumpbin /dependents` on the release MSI
# (v0.1.22, 2026-10-05). A clean Windows 10 machine ships no VC++ redist, so
# the MSI carries exactly those two DLLs next to every executable it installs
# (see wix\main.wxs). The DLLs are NOT vendored in git; they are copied at
# build time from the build host's MSVC toolset:
#   provenance: <VS 2022 flavor>\VC\Tools\MSVC\<ver>\bin\Hostx64\<arch>\
#   fallback:   C:\Program Files (x86)\Windows Kits\10\bin\<sdkver>\<arch>\
# Both locations hold the identical redistributable CRT the toolchain itself
# uses to run its own host tools. Override the search with $env:SHVCREDIST_DIR
# (a directory containing the DLLs directly) for exotic toolchains.
$requiredCrt = @("vcruntime140.dll", "vcruntime140_1.dll")
$crtStage = "redist"
$crtArchDir = if ($WixArch -eq "x64") { "x64" } else { "arm64" }
$crtExpectedMachine = if ($WixArch -eq "x64") { 0x8664 } else { 0xAA64 }

# Read the PE "Machine" field so a wrong-arch DLL can never be staged.
function Get-PeMachine {
    param([string]$Path)
    $fs = [IO.File]::OpenRead($Path)
    try {
        $head = New-Object byte[] 4096
        [void]$fs.Read($head, 0, 4096)
        if ($head[0] -ne 0x4D -or $head[1] -ne 0x5A) { return -1 }   # not MZ
        $peOff = [BitConverter]::ToInt32($head, 0x3C)
        if ($head[$peOff] -ne 0x50 -or $head[$peOff + 1] -ne 0x45) { return -1 }  # not PE
        return [int][BitConverter]::ToUInt16($head, $peOff + 4)  # COFF file header starts at peOff+4; Machine is its first field
    } finally {
        $fs.Close()
    }
}

Write-Host "Staging VC++ runtime DLLs (app-local) for $crtArchDir..." -ForegroundColor Cyan
$crtRoots = @()
if ($env:SHVCREDIST_DIR) { $crtRoots += $env:SHVCREDIST_DIR }
$vsToolsetRoots = @(
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC",
    "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC",
    "C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Tools\MSVC",
    "C:\Program Files\Microsoft Visual Studio\2022\Enterprise\VC\Tools\MSVC"
)
foreach ($vs in $vsToolsetRoots) {
    if (Test-Path $vs) {
        Get-ChildItem $vs -Directory -ErrorAction SilentlyContinue |
            Sort-Object Name -Descending |
            ForEach-Object { $crtRoots += Join-Path $_.FullName "bin\Hostx64\$crtArchDir" }
    }
}
$sdkBinRoot = "C:\Program Files (x86)\Windows Kits\10\bin"
if (Test-Path $sdkBinRoot) {
    Get-ChildItem $sdkBinRoot -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending |
        ForEach-Object { $crtRoots += Join-Path $_.FullName $crtArchDir }
}

New-Item -ItemType Directory -Path $crtStage -Force | Out-Null
foreach ($dll in $requiredCrt) {
    $found = $null
    foreach ($root in $crtRoots) {
        $candidate = Join-Path $root $dll
        if (Test-Path $candidate) { $found = $candidate; break }
    }
    if (-not $found) {
        Write-Host "ERROR: $dll not found on this build host." -ForegroundColor Red
        Write-Host "Install Visual Studio 2022 Build Tools (C++ workload) or set SHVCREDIST_DIR to a directory containing the $crtArchDir VC++ runtime DLLs." -ForegroundColor Yellow
        exit 1
    }
    $machine = Get-PeMachine $found
    if ($machine -ne $crtExpectedMachine) {
        Write-Host ("ERROR: {0} is PE machine 0x{1:X4}, expected 0x{2:X4} ({3}). Wrong-arch toolset on this build host?" -f $found, $machine, $crtExpectedMachine, $crtArchDir) -ForegroundColor Red
        exit 1
    }
    $stagedPath = Join-Path $crtStage $dll
    Copy-Item $found $stagedPath -Force
    $hash = (Get-FileHash $stagedPath -Algorithm SHA256).Hash
    Write-Host ("  staged {0,-20} <- {1}  (machine 0x{2:X4}, sha256 {3}...)" -f $dll, $found, $machine, $hash.Substring(0, 16))
}
Write-Host "VC++ runtime staged in $crtStage\ (shipped app-locally by the MSI)" -ForegroundColor Green

# Download connectors if needed
if (-not (Test-Path "dist\ks-connector.exe")) {
    Write-Host "Downloading ks-connector..." -ForegroundColor Yellow
    New-Item -ItemType Directory -Path dist -Force | Out-Null
    try {
        $ksUrl = "https://github.com/Strike48-public/kubestudio/releases/download/v0.1.1/ks-connector-windows-x86_64.zip"
        Invoke-WebRequest -Uri $ksUrl -OutFile "dist\ks-connector.zip"
        Expand-Archive -Path "dist\ks-connector.zip" -DestinationPath "dist" -Force
        Remove-Item "dist\ks-connector.zip"
        Write-Host "  downloaded" -ForegroundColor Green
    } catch {
        Write-Host "  WARNING: could not download ks-connector" -ForegroundColor Yellow
    }
}

if (-not (Test-Path "dist\pentest-agent.exe")) {
    Write-Host "Downloading pentest-agent..." -ForegroundColor Yellow
    New-Item -ItemType Directory -Path dist -Force | Out-Null
    try {
        # Pick v0.1.9 renamed the release archive pentest-agent-* -> pick-agent-*.
        # The binary inside is still pentest-agent.exe, so downstream stays the same.
        $paUrl = "https://github.com/Strike48-public/pick/releases/download/$PickVersion/pick-agent-windows-x86_64.zip"
        Invoke-WebRequest -Uri $paUrl -OutFile "dist\pentest-agent.zip"

        # Verify against the release's SHA256SUMS.txt before trusting the archive:
        # this executable ships inside the installer, so a tampered or re-uploaded
        # asset must fail the build rather than flow into the MSI.
        $sumsUrl = "https://github.com/Strike48-public/pick/releases/download/$PickVersion/SHA256SUMS.txt"
        Invoke-WebRequest -Uri $sumsUrl -OutFile "dist\SHA256SUMS.txt"
        $expected = Get-Content "dist\SHA256SUMS.txt" |
            Where-Object { $_ -match '\s+pick-agent-windows-x86_64\.zip\s*$' } |
            ForEach-Object { ($_ -split '\s+')[0] } |
            Select-Object -First 1
        if (-not $expected) {
            throw "no SHA256 entry for pick-agent-windows-x86_64.zip in pick $PickVersion"
        }
        $actual = (Get-FileHash "dist\pentest-agent.zip" -Algorithm SHA256).Hash
        if ($actual -ne $expected.Trim().ToUpper()) {
            throw "SHA256 checksum mismatch for pick-agent-windows-x86_64.zip"
        }
        Remove-Item "dist\SHA256SUMS.txt"

        Expand-Archive -Path "dist\pentest-agent.zip" -DestinationPath "dist" -Force
        Remove-Item "dist\pentest-agent.zip"
        Write-Host "  downloaded and checksum-verified" -ForegroundColor Green
    } catch {
        Write-Host "  WARNING: could not download/verify pentest-agent: $_" -ForegroundColor Yellow
    }
}

# Fail loud rather than shipping a connector-less installer: a bad asset name or
# a failed download must not silently produce an MSI without pentest-agent.
if (-not (Test-Path "dist\pentest-agent.exe")) {
    Write-Host "ERROR: dist\pentest-agent.exe is missing - cannot build a working MSI." -ForegroundColor Red
    Write-Host "Pre-stage it in dist\ or ensure pick $PickVersion publishes pick-agent-windows-x86_64.zip." -ForegroundColor Yellow
    exit 1
}

# WiX Product/@Version is strictly numeric x.x.x.x (integers 0..65534); candle
# rejects prerelease versions with CNDL0108 - e.g. 0.1.22-rc.1, which the
# Release workflow's "Sync Cargo.toml to tag version" step writes for PRERELEASE
# tags and the MSI step passes through as-is. Feed WiX only the leading numeric
# groups; $Version stays raw for the output filename so the workflow's Move-Item
# contract (StrikeHub-<rawVersion>-<arch>.msi) is unchanged.
if ($Version -match '^(\d+(?:\.\d+){0,3})') {
    $WixVersion = $Matches[1]
} else {
    $WixVersion = $Version   # non-numeric input: pass through (candle rejects as before)
}
if ($WixVersion -ne $Version) {
    Write-Host "  (WiX ProductVersion mapped to numeric $WixVersion)" -ForegroundColor Yellow
}

# Compile WiX source
Write-Host "Compiling..." -ForegroundColor Yellow
& "$wixDir\candle.exe" -nologo `
    -dVersion="$WixVersion" `
    -dPlatform="$WixPlatform" `
    -arch $WixArch `
    -out wix\main.wixobj `
    wix\main.wxs
if ($LASTEXITCODE -ne 0) { exit 1 }

# Link MSI
Write-Host "Linking MSI..." -ForegroundColor Yellow
& "$wixDir\light.exe" -nologo `
    -ext WixUIExtension `
    -out "StrikeHub-$Version-$Arch.msi" `
    wix\main.wixobj
if ($LASTEXITCODE -ne 0) { exit 1 }

# Cleanup
Remove-Item "wix\main.wixobj" -ErrorAction SilentlyContinue

$msi = "StrikeHub-$Version-$Arch.msi"
$size = [math]::Round((Get-Item $msi).Length / 1MB, 2)
Write-Host ""
Write-Host "$msi ($size MB)" -ForegroundColor Green
Write-Host ""
Write-Host "Install:     msiexec /i $msi"
Write-Host "Silent:      msiexec /i $msi /qn"
Write-Host "Uninstall:   msiexec /x $msi"
