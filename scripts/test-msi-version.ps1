#!/usr/bin/env pwsh
# Unit tests for the MSI version mapping in scripts/build-msi.ps1
# (ConvertTo-MsiVersion): the mapping that keeps every Windows Installer
# build numerically unique and monotonic, so rc.N -> rc.N+1 -> official
# upgrades actually overwrite the hub binary instead of stalling on
# "Existing file is of an equal version" (project-management#375).
#
# The function under test is EXTRACTED from build-msi.ps1 (not re-declared
# here) so this file cannot drift from the shipped code. Runs anywhere
# pwsh works, no WiX needed:  pwsh ./scripts/test-msi-version.ps1

$ErrorActionPreference = "Stop"
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$srcFile = Join-Path $scriptRoot "build-msi.ps1"

# ── Extract ConvertTo-MsiVersion from build-msi.ps1 ─────────────────────────
$lines = Get-Content $srcFile
$start = -1
for ($i = 0; $i -lt $lines.Count; $i++) {
    if ($lines[$i] -match '^function ConvertTo-MsiVersion \{') { $start = $i; break }
}
if ($start -lt 0) {
    Write-Host "FAIL: function ConvertTo-MsiVersion not found in build-msi.ps1" -ForegroundColor Red
    exit 1
}
$end = -1
for ($i = $start + 1; $i -lt $lines.Count; $i++) {
    if ($lines[$i] -match '^\}') { $end = $i; break }   # column-0 close ends the top-level function
}
if ($end -lt 0) {
    Write-Host "FAIL: could not find the closing brace of ConvertTo-MsiVersion" -ForegroundColor Red
    exit 1
}
$fnText = ($lines[$start..$end]) -join "`n"
$scriptBlock = [scriptblock]::Create($fnText)
. $scriptBlock   # dot-source: defines ConvertTo-MsiVersion in this session
if (-not (Get-Command ConvertTo-MsiVersion -ErrorAction SilentlyContinue)) {
    Write-Host "FAIL: extracted function did not define ConvertTo-MsiVersion" -ForegroundColor Red
    exit 1
}
Write-Host "Extracted ConvertTo-MsiVersion from build-msi.ps1 (lines $($start + 1)..$($end + 1))" -ForegroundColor Cyan

# ── Assertion helpers ────────────────────────────────────────────────────────
$failures = 0
$passes = 0

function Assert-Maps {
    param([string]$Version, [string]$Expected)
    $script:_lastErr = $null
    $actual = $null
    try { $actual = ConvertTo-MsiVersion $Version } catch { $script:_lastErr = $_.Exception.Message }
    if ($script:_lastErr) {
        Write-Host ("FAIL  {0,-18} -> expected '{1}', threw: {2}" -f $Version, $Expected, $script:_lastErr) -ForegroundColor Red
        $global:failures++
    } elseif ($actual -ne $Expected) {
        Write-Host ("FAIL  {0,-18} -> expected '{1}', got '{2}'" -f $Version, $Expected, $actual) -ForegroundColor Red
        $global:failures++
    } else {
        Write-Host ("PASS  {0,-18} -> {1}" -f $Version, $actual) -ForegroundColor Green
        $global:passes++
    }
}

function Assert-Throws {
    param([string]$Version)
    try {
        $actual = ConvertTo-MsiVersion $Version
        Write-Host ("FAIL  {0,-18} -> expected throw, got '{1}'" -f $Version, $actual) -ForegroundColor Red
        $global:failures++
    } catch {
        Write-Host ("PASS  {0,-18} -> threw (expected)" -f $Version) -ForegroundColor Green
        $global:passes++
    }
}

# ── Mapping table ────────────────────────────────────────────────────────────
# Prereleases: rc.N encodes in the 4th (revision) field.
Assert-Maps "0.1.23-rc.1"  "0.1.23.1"
Assert-Maps "0.1.23-rc.2"  "0.1.23.2"
Assert-Maps "0.1.23-rc.99" "0.1.23.99"
Assert-Maps "v0.1.23-rc.2" "0.1.23.2"      # leading v (raw tag form)
Assert-Maps " 0.1.23-rc.2 " "0.1.23.2"     # surrounding whitespace

# Official builds: .100 headroom above every rc.N (N <= 99).
Assert-Maps "0.1.23"  "0.1.23.100"
Assert-Maps "v0.1.23" "0.1.23.100"
Assert-Maps "0.1.22"  "0.1.22.100"
Assert-Maps "0.0.0"   "0.0.0.100"          # ci.yml build-installers smoke value

# Already-numeric 4-field versions pass through untouched.
Assert-Maps "0.1.23.7" "0.1.23.7"

# Fail loud: rc.N > 99 must throw (official .100 must stay above every rc).
Assert-Throws "0.1.23-rc.100"
Assert-Throws "0.1.23-rc.999"
# Fail loud: unknown suffixes must throw - silently stripping a suffix is
# exactly what produced the rc.1/rc.2 collision.
Assert-Throws "0.1.23-beta.1"
Assert-Throws "0.1.23.4.5"
Assert-Throws "0.1.23-rc"
Assert-Throws "not-a-version"
Assert-Throws "0.1"

# ── Structural invariants on a sample of outputs ─────────────────────────────
$samples = @("0.1.23-rc.1", "0.1.23-rc.99", "0.1.23", "0.1.24")
foreach ($s in $samples) {
    $out = ConvertTo-MsiVersion $s
    $parts = $out -split '\.'
    if ($parts.Count -ne 4) {
        Write-Host ("FAIL  {0} -> '{1}' is not a 4-field version" -f $s, $out) -ForegroundColor Red
        $global:failures++
    } elseif ($parts | Where-Object { -not ($_ -match '^\d+$') -or [int]$_ -gt 65534 }) {
        Write-Host ("FAIL  {0} -> '{1}' has a non-numeric or > 65534 field (WiX limit)" -f $s, $out) -ForegroundColor Red
        $global:failures++
    } else {
        Write-Host ("PASS  {0,-18} -> '{1}' is 4-field numeric, all fields <= 65534" -f $s, $out) -ForegroundColor Green
        $global:passes++
    }
}

# ── Monotonicity across a full release line ──────────────────────────────────
# v0.1.22 official < v0.1.23-rc.1 < v0.1.23-rc.2 < ... < rc.99 < v0.1.23
# official < v0.1.24-rc.1 < v0.1.24 official. Every forward install must be a
# strictly greater numeric version (MajorUpgrade upgrade, not repair/downgrade).
$line = @(
    "0.1.22", "0.1.23-rc.1", "0.1.23-rc.2", "0.1.23-rc.99", "0.1.23", "0.1.24-rc.1", "0.1.24"
) | ForEach-Object { [Version](ConvertTo-MsiVersion $_) }
$monoOk = $true
for ($i = 1; $i -lt $line.Count; $i++) {
    if ($line[$i - 1] -ge $line[$i]) {
        Write-Host ("FAIL  release line not monotonic: {0} !< {1}" -f $line[$i - 1], $line[$i]) -ForegroundColor Red
        $monoOk = $false
        $global:failures++
    }
}
if ($monoOk) {
    Write-Host ("PASS  release line monotonic: " + (($line | ForEach-Object { "$_" }) -join " < ")) -ForegroundColor Green
    $global:passes++
}

# ── Result ───────────────────────────────────────────────────────────────────
Write-Host ""
if ($global:failures -gt 0) {
    Write-Host "test-msi-version: $global:failures FAILED, $global:passes passed" -ForegroundColor Red
    exit 1
}
Write-Host "test-msi-version: all $global:passes passed" -ForegroundColor Green
exit 0
