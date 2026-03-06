#!/bin/bash
# Quick script to build and run StrikeHub AppImage with connectors

# Get to the project root directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

echo "🚀 Building StrikeHub AppImage with connectors..."
./scripts/build-appimage-with-connectors.sh test x86_64

if [ -f "StrikeHub-test-x86_64.AppImage" ]; then
    echo ""
    echo "🎯 Running StrikeHub AppImage..."
    echo "Default API URL: https://studio.strike48.test"
    echo "Press Ctrl+C to stop"
    echo ""
    ./StrikeHub-test-x86_64.AppImage
else
    echo "❌ Build failed"
    exit 1
fi
