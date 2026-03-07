#!/bin/bash
set -e

# Simple AppImage build that ensures environment variables are set

VERSION=${1:-latest}
ARCH=${2:-x86_64}
APPDIR="StrikeHub.AppDir"

echo "Building StrikeHub AppImage with working env vars..."

# Clean up
rm -rf "$APPDIR"
rm -f StrikeHub*.AppImage

# Download appimagetool if needed
if [ ! -f "appimagetool-x86_64.AppImage" ]; then
    echo "Downloading appimagetool..."
    wget -q "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage"
    chmod +x appimagetool-x86_64.AppImage
fi

# Create AppDir structure
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"

# Copy binaries
echo "Copying binaries..."
cp "target/x86_64-unknown-linux-gnu/release/strikehub" "$APPDIR/usr/bin/strikehub-real"
chmod +x "$APPDIR/usr/bin/strikehub-real"

# Create wrapper script that sets env vars
cat > "$APPDIR/usr/bin/strikehub" << 'EOF'
#!/bin/bash
HERE="$(dirname "$(readlink -f "${0}")")"

# Set default Strike48 URLs if not already set
if [ -z "$STRIKE48_API_URL" ]; then
    export STRIKE48_API_URL="https://studio.strike48.com"
fi
if [ -z "$STRIKE48_URL" ]; then
    export STRIKE48_URL="wss://studio.strike48.com"
fi

# Execute the real binary
exec "$HERE/strikehub-real" "$@"
EOF
chmod +x "$APPDIR/usr/bin/strikehub"

# Copy connectors if they exist
if [ -f "dist/ks-connector" ]; then
    echo "✓ Adding ks-connector..."
    cp "dist/ks-connector" "$APPDIR/usr/bin/"
    chmod +x "$APPDIR/usr/bin/ks-connector"
fi

if [ -f "dist/pentest-agent" ]; then
    echo "✓ Adding pentest-agent..."
    cp "dist/pentest-agent" "$APPDIR/usr/bin/"
    chmod +x "$APPDIR/usr/bin/pentest-agent"
fi

# Copy desktop file and icon
cp "assets/strikehub.desktop" "$APPDIR/usr/share/applications/"
cp "assets/strikehub.desktop" "$APPDIR/"
cp "crates/sh-ui/src/assets/icons/strike48-mark.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/strikehub.svg"
cp "crates/sh-ui/src/assets/icons/strike48-mark.svg" "$APPDIR/strikehub.svg"

# Create a simple AppRun
cat > "$APPDIR/AppRun" << 'EOF'
#!/bin/bash
HERE="$(dirname "$(readlink -f "${0}")")"
export PATH="${HERE}/usr/bin:${PATH}"
export LD_LIBRARY_PATH="${HERE}/usr/lib:${HERE}/usr/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH}"
exec "$HERE/usr/bin/strikehub" "$@"
EOF
chmod +x "$APPDIR/AppRun"

# Build the AppImage
echo "Creating AppImage..."
ARCH=$ARCH ./appimagetool-x86_64.AppImage "$APPDIR" "StrikeHub-${VERSION}-${ARCH}.AppImage"

echo ""
echo "✅ AppImage created: StrikeHub-${VERSION}-${ARCH}.AppImage"
echo "This version properly sets environment variables!"
