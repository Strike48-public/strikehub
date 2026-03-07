#!/bin/bash
set -e

# Build a single-file Windows SFX executable (like AppImage but for Windows)
#
# Creates: StrikeHub-{VERSION}-x86_64.exe
#
# How it works:
#   1. Packages strikehub.exe + connectors + launcher into a 7z archive
#   2. Prepends the 7-Zip SFX stub + config
#   3. Result is a single .exe that extracts to %LOCALAPPDATA%\StrikeHub and runs
#
# This is the Windows equivalent of AppImage: one file, double-click, it works.

VERSION=${1:-latest}
PICK_VERSION=${PICK_VERSION:-v0.1.0}
KUBESTUDIO_VERSION=${KUBESTUDIO_VERSION:-v0.1.0}

echo "Building StrikeHub Windows SFX..."
echo "================================="
echo "Version: $VERSION"
echo ""

STAGING="sfx-staging"
rm -rf "$STAGING"
mkdir -p "$STAGING"

# Copy the main binary
echo "Copying binaries..."
cp "target/x86_64-pc-windows-msvc/release/strikehub.exe" "$STAGING/strikehub.exe"

# Copy connectors
if [ -f "dist/ks-connector.exe" ]; then
    echo "✓ Adding ks-connector.exe"
    cp "dist/ks-connector.exe" "$STAGING/"
fi

# Create a VBScript launcher (no console window, unlike .bat)
cat > "$STAGING/StrikeHub-Launch.vbs" << 'LAUNCHER'
Set WshShell = CreateObject("WScript.Shell")
Set WshEnv = WshShell.Environment("Process")

' Set default Strike48 URLs if not already set
If WshEnv("STRIKE48_API_URL") = "" Then
    WshEnv("STRIKE48_API_URL") = "https://studio.strike48.com"
End If
If WshEnv("STRIKE48_URL") = "" Then
    WshEnv("STRIKE48_URL") = "wss://studio.strike48.com"
End If

' Get directory this script lives in
scriptDir = Left(WScript.ScriptFullName, InStrRev(WScript.ScriptFullName, "\"))

' Launch strikehub.exe with no visible console window
WshShell.Run """" & scriptDir & "strikehub.exe""", 0, False
LAUNCHER

# Create the 7-Zip SFX config
cat > sfx-config.txt << 'SFX_CONFIG'
;!@Install@!UTF-8!
Title="StrikeHub"
RunProgram="wscript.exe StrikeHub-Launch.vbs"
InstallPath="%LOCALAPPDATA%\\StrikeHub"
GUIMode="2"
OverwriteMode="2"
;!@InstallEnd@!
SFX_CONFIG

# Build the 7z archive
echo "Creating archive..."
7z a -mx=9 sfx-payload.7z "./$STAGING/*"

# Download the 7-Zip SFX module if needed
if [ ! -f "7zSD.sfx" ]; then
    echo "Downloading 7-Zip SFX module..."
    curl -sL "https://github.com/nicehash/7zsfxmm/raw/master/files/7zSD.sfx" -o 7zSD.sfx
fi

# Combine: SFX stub + config + archive = single exe
echo "Creating single-file executable..."
cat 7zSD.sfx sfx-config.txt sfx-payload.7z > "StrikeHub-${VERSION}-x86_64.exe"

# Cleanup
rm -rf "$STAGING" sfx-config.txt sfx-payload.7z

echo ""
echo "✅ StrikeHub-${VERSION}-x86_64.exe"
SIZE=$(du -h "StrikeHub-${VERSION}-x86_64.exe" | cut -f1)
echo "Size: $SIZE"
echo ""
echo "This is a single file. Double-click to run."
echo "It extracts to %LOCALAPPDATA%\StrikeHub and launches automatically."
