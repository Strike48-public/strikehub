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

# Compile WiX source
Write-Host "Compiling..." -ForegroundColor Yellow
& "$wixDir\candle.exe" -nologo `
    -dVersion="$Version" `
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
