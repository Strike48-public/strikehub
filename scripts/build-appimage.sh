#!/bin/bash
set -e

# Simple AppImage build that ensures environment variables are set

VERSION=${1:-latest}
ARCH=${2:-x86_64}
APPDIR="StrikeHub.AppDir"

# Derive the Rust target triple from ARCH
case "$ARCH" in
  x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
  aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
  *)       echo "Unsupported arch: $ARCH"; exit 1 ;;
esac

echo "Building StrikeHub AppImage ($ARCH) with working env vars..."

# Clean up
rm -rf "$APPDIR"
rm -f StrikeHub*.AppImage

# Download appimagetool if needed
APPIMAGETOOL="appimagetool-${ARCH}.AppImage"
if [ ! -f "$APPIMAGETOOL" ]; then
    echo "Downloading $APPIMAGETOOL..."
    wget -q "https://github.com/AppImage/AppImageKit/releases/download/continuous/${APPIMAGETOOL}"
    chmod +x "$APPIMAGETOOL"
fi

# Create AppDir structure
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"

# Copy binaries (BIN_DIR can be overridden for debug builds)
BIN_DIR="${BIN_DIR:-target/${TARGET}/release}"
echo "Copying binaries from ${BIN_DIR}..."
cp "${BIN_DIR}/strikehub" "$APPDIR/usr/bin/strikehub-real"
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

# Force X11 backend to avoid Wayland protocol errors with WebKitGTK
export GDK_BACKEND=x11

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

# Generate PNG icons from SVG for desktop environment compatibility.
# Many DEs (GNOME, KDE, XFCE) prefer PNG over SVG for app icons.
SVG_ICON="crates/sh-ui/src/assets/icons/strike48-mark.svg"
for SIZE in 256 128 64 48 32 16; do
    DIR="$APPDIR/usr/share/icons/hicolor/${SIZE}x${SIZE}/apps"
    mkdir -p "$DIR"
    if command -v rsvg-convert &> /dev/null; then
        rsvg-convert -w "$SIZE" -h "$SIZE" "$SVG_ICON" -o "$DIR/strikehub.png"
    else
        convert -background none "$SVG_ICON" -resize "${SIZE}x${SIZE}" "$DIR/strikehub.png"
    fi
done

# AppImage requires a PNG icon at the AppDir root and a .DirIcon for file managers.
cp "$APPDIR/usr/share/icons/hicolor/256x256/apps/strikehub.png" "$APPDIR/strikehub.png"
cp "$APPDIR/usr/share/icons/hicolor/256x256/apps/strikehub.png" "$APPDIR/.DirIcon"

# Create a simple AppRun
cat > "$APPDIR/AppRun" << RUNEOF
#!/bin/bash
HERE="\$(dirname "\$(readlink -f "\${0}")")"
export PATH="\${HERE}/usr/bin:\${PATH}"
export LD_LIBRARY_PATH="\${HERE}/usr/lib:\${HERE}/usr/lib/${ARCH}-linux-gnu:\${LD_LIBRARY_PATH}"
exec "\$HERE/usr/bin/strikehub" "\$@"
RUNEOF
chmod +x "$APPDIR/AppRun"

# Build the AppImage
echo "Creating AppImage..."
ARCH=$ARCH "./${APPIMAGETOOL}" "$APPDIR" "StrikeHub-${VERSION}-${ARCH}.AppImage"

echo ""
echo "✅ AppImage created: StrikeHub-${VERSION}-${ARCH}.AppImage"
echo "This version properly sets environment variables!"
