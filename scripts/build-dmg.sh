#!/bin/bash
set -e

# Build a macOS .dmg installer for StrikeHub

VERSION=${1:-latest}
ARCH=${2:-aarch64}
APP_NAME="StrikeHub"
BUNDLE_ID="com.strike48.strikehub"
DMG_NAME="${APP_NAME}-${VERSION}-${ARCH}.dmg"
APP_DIR="${APP_NAME}.app"
STAGING_DIR="dmg-staging"

if [ "$ARCH" = "aarch64" ]; then
    RUST_TARGET="aarch64-apple-darwin"
elif [ "$ARCH" = "x86_64" ]; then
    RUST_TARGET="x86_64-apple-darwin"
else
    echo "Unsupported architecture: $ARCH"
    exit 1
fi

echo "Building ${APP_NAME} DMG for ${ARCH}..."

# Clean up previous builds
rm -rf "$APP_DIR" "$STAGING_DIR" "$DMG_NAME"

# Create .app bundle structure
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# Copy the main binary
cp "target/${RUST_TARGET}/release/strikehub" "$APP_DIR/Contents/MacOS/strikehub"
chmod +x "$APP_DIR/Contents/MacOS/strikehub"

# Copy connectors if they exist
if [ -f "dist/ks-connector" ]; then
    echo "Adding ks-connector..."
    cp "dist/ks-connector" "$APP_DIR/Contents/MacOS/"
    chmod +x "$APP_DIR/Contents/MacOS/ks-connector"
fi

if [ -f "dist/pentest-agent" ]; then
    echo "Adding pentest-agent..."
    cp "dist/pentest-agent" "$APP_DIR/Contents/MacOS/"
    chmod +x "$APP_DIR/Contents/MacOS/pentest-agent"
fi

# Convert SVG icon to icns if possible
ICON_SRC="crates/sh-ui/src/assets/icons/strike48-logo.svg"
if command -v sips &> /dev/null && command -v iconutil &> /dev/null; then
    echo "Generating app icon..."
    ICONSET_DIR="StrikeHub.iconset"
    mkdir -p "$ICONSET_DIR"

    # Use rsvg-convert if available, otherwise fall back to sips
    if command -v rsvg-convert &> /dev/null; then
        for size in 16 32 64 128 256 512; do
            rsvg-convert -w $size -h $size "$ICON_SRC" -o "$ICONSET_DIR/icon_${size}x${size}.png"
            double=$((size * 2))
            rsvg-convert -w $double -h $double "$ICON_SRC" -o "$ICONSET_DIR/icon_${size}x${size}@2x.png"
        done
    else
        echo "Warning: rsvg-convert not found, using placeholder icon"
        # Create a simple 512x512 PNG placeholder with sips from any available image
        for size in 16 32 64 128 256 512; do
            # sips cannot convert SVG, so we skip icon generation
            true
        done
    fi

    # Only create icns if we have PNGs
    if ls "$ICONSET_DIR"/*.png &> /dev/null; then
        iconutil -c icns "$ICONSET_DIR" -o "$APP_DIR/Contents/Resources/StrikeHub.icns"
    fi
    rm -rf "$ICONSET_DIR"
fi

# Create Info.plist
cat > "$APP_DIR/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>strikehub</string>
    <key>CFBundleIconFile</key>
    <string>StrikeHub</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST

# Ad-hoc sign the bundle so Gatekeeper sees a consistent signature
# (the linker-applied signature on the binary doesn't cover bundle resources)
codesign --force --deep --sign - "$APP_DIR"

# Create DMG staging area with app and Applications symlink
mkdir -p "$STAGING_DIR"
cp -R "$APP_DIR" "$STAGING_DIR/"
ln -s /Applications "$STAGING_DIR/Applications"

# Create the DMG
echo "Creating DMG..."
hdiutil create -volname "$APP_NAME" \
    -srcfolder "$STAGING_DIR" \
    -ov -format UDZO \
    "$DMG_NAME"

# Clean up
rm -rf "$APP_DIR" "$STAGING_DIR"

echo ""
echo "DMG created: $DMG_NAME"
