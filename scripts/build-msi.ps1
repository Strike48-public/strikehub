param(
    [string]$Version = "0.1.0",
    [string]$Arch = "x86_64"
)

$ErrorActionPreference = "Stop"

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

# Download connectors if needed
if (-not (Test-Path "dist\ks-connector.exe")) {
    Write-Host "Downloading ks-connector..." -ForegroundColor Yellow
    New-Item -ItemType Directory -Path dist -Force | Out-Null
    try {
        $ksUrl = "https://github.com/Strike48-public/kubestudio/releases/download/v0.1.0/ks-connector-windows-x86_64.zip"
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
        $paUrl = "https://github.com/Strike48-public/pick/releases/download/v0.1.1/pentest-agent-windows-x86_64.zip"
        Invoke-WebRequest -Uri $paUrl -OutFile "dist\pentest-agent.zip"
        Expand-Archive -Path "dist\pentest-agent.zip" -DestinationPath "dist" -Force
        Remove-Item "dist\pentest-agent.zip"
        Write-Host "  downloaded" -ForegroundColor Green
    } catch {
        Write-Host "  WARNING: could not download pentest-agent" -ForegroundColor Yellow
    }
}

# Compile WiX source
Write-Host "Compiling..." -ForegroundColor Yellow
& "$wixDir\candle.exe" -nologo `
    -dVersion="$Version" `
    -dTargetDir="target\$Target\release" `
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
