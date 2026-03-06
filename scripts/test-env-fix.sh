#!/bin/bash
# Get to the project root directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

echo "🔧 Testing AppImage environment variable fix..."
echo ""

# Clean up old AppImage
rm -f StrikeHub-test-x86_64.AppImage

# Build with the fixed script
echo "Building with fixed script..."
./scripts/build-appimage-with-connectors.sh test x86_64

# Extract and check AppRun
echo ""
echo "Checking if environment variables are set..."
./StrikeHub-test-x86_64.AppImage --appimage-extract usr/bin/strikehub > /dev/null 2>&1
if grep -q "STRIKE48_API_URL" squashfs-root/usr/bin/strikehub 2>/dev/null; then
    echo "✅ Environment variables found in wrapper script!"
    echo ""
    echo "Content of wrapper:"
    grep "export STRIKE48" squashfs-root/usr/bin/strikehub
else
    echo "❌ Environment variables NOT found"
fi

# Clean up
rm -rf squashfs-root

echo ""
echo "Now run: ./StrikeHub-test-x86_64.AppImage"
echo "You should see the login screen without setting any env vars!"
