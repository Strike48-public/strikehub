#!/bin/bash
set -e

# Build AppImage with connectors bundled
# This script downloads the connectors from GitHub releases and includes them

# Load environment variables if .env exists
if [ -f ".env" ]; then
    echo "Loading configuration from .env..."
    export $(grep -v '^#' .env | xargs)
fi

VERSION=${1:-latest}
ARCH=${2:-x86_64}
PICK_VERSION=${PICK_VERSION:-v0.1.2}
KUBESTUDIO_VERSION=${KUBESTUDIO_VERSION:-v0.1.3}

echo "Building StrikeHub AppImage with connectors..."
echo "============================================"
echo "Version: $VERSION"
echo "Architecture: $ARCH"
echo "Pick version: $PICK_VERSION"
echo "KubeStudio version: $KUBESTUDIO_VERSION"
echo ""

# Build the StrikeHub binary first
echo "Building StrikeHub..."
cargo build --release --target x86_64-unknown-linux-gnu --no-default-features --features desktop

# Create dist directory for connectors
echo ""
echo "Preparing connectors..."
mkdir -p dist
cd dist

# Download pentest-agent (Pick connector)
echo "Downloading pentest-agent..."
if [ ! -f "pentest-agent" ]; then
    if command -v gh &> /dev/null; then
        gh release download $PICK_VERSION \
            --repo Strike48-public/pick \
            --pattern "pentest-agent-linux-x86_64.tar.gz" \
            2>/dev/null || {
            echo "WARNING: Could not download pentest-agent from GitHub"
            echo "Try: wget https://github.com/Strike48-public/pick/releases/download/$PICK_VERSION/pentest-agent-linux-x86_64.tar.gz"
        }
    else
        wget -q "https://github.com/Strike48-public/pick/releases/download/$PICK_VERSION/pentest-agent-linux-x86_64.tar.gz" || {
            echo "WARNING: Could not download pentest-agent"
        }
    fi

    if [ -f "pentest-agent-linux-x86_64.tar.gz" ]; then
        tar -xzf pentest-agent-linux-x86_64.tar.gz
        rm -f pentest-agent-linux-x86_64.tar.gz
        echo "✓ pentest-agent downloaded"
    fi
else
    echo "✓ pentest-agent already exists"
fi

# Download ks-connector (KubeStudio connector)
echo "Downloading ks-connector..."
if [ ! -f "ks-connector" ]; then
    if command -v gh &> /dev/null; then
        gh release download $KUBESTUDIO_VERSION \
            --repo Strike48-public/kubestudio \
            --pattern "ks-connector-linux-x86_64.tar.gz" \
            2>/dev/null || {
            echo "WARNING: Could not download ks-connector from GitHub"
            echo "Try: wget https://github.com/Strike48-public/kubestudio/releases/download/$KUBESTUDIO_VERSION/ks-connector-linux-x86_64.tar.gz"
        }
    else
        wget -q "https://github.com/Strike48-public/kubestudio/releases/download/$KUBESTUDIO_VERSION/ks-connector-linux-x86_64.tar.gz" || {
            echo "WARNING: Could not download ks-connector"
        }
    fi

    if [ -f "ks-connector-linux-x86_64.tar.gz" ]; then
        tar -xzf ks-connector-linux-x86_64.tar.gz
        rm -f ks-connector-linux-x86_64.tar.gz
        echo "✓ ks-connector downloaded"
    fi
else
    echo "✓ ks-connector already exists"
fi

# Check what we have
echo ""
echo "Connectors in dist/:"
ls -la *.connector 2>/dev/null || ls -la *agent 2>/dev/null || echo "No connectors found"
cd ..

# Run the AppImage build
echo ""
echo "Building AppImage..."
./scripts/build-appimage.sh $VERSION $ARCH

# Check result
if [ -f "StrikeHub-${VERSION}-${ARCH}.AppImage" ]; then
    echo ""
    echo "✅ SUCCESS: AppImage built with connectors!"
    echo "File: StrikeHub-${VERSION}-${ARCH}.AppImage"
    echo "Size: $(du -h StrikeHub-${VERSION}-${ARCH}.AppImage | cut -f1)"
    echo ""
    echo "To run:"
    echo "  ./StrikeHub-${VERSION}-${ARCH}.AppImage"
    echo ""
    echo "The AppImage will automatically:"
    echo "  - Set STRIKE48_API_URL=https://studio.strike48.test"
    echo "  - Include ks-connector and pentest-agent"
    echo ""
    echo "To use a different API server:"
    echo "  STRIKE48_API_URL=https://your.server ./StrikeHub-${VERSION}-${ARCH}.AppImage"
else
    echo ""
    echo "❌ ERROR: AppImage was not created"
    exit 1
fi
