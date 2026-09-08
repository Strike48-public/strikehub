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

# Default connector versions to the release tags pinned in connector-versions.env
# (the same file CI/release reads) so these offline-build defaults can't drift
# from the pipeline; still overridable via the PICK_VERSION/KUBESTUDIO_VERSION env
# vars. These must be release *tags* for the download paths below.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ -f "$REPO_ROOT/connector-versions.env" ]; then
    # shellcheck disable=SC1090,SC1091
    source "$REPO_ROOT/connector-versions.env"
fi
PICK_VERSION=${PICK_VERSION:-${PICK_REF:-v0.1.10}}
KUBESTUDIO_VERSION=${KUBESTUDIO_VERSION:-${KUBESTUDIO_REF:-v0.2.1}}

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

# Download pentest-agent (Pick connector).
# Pick v0.1.9 renamed the release archive pentest-agent-* -> pick-agent-*; the
# binary inside is still "pentest-agent", so only the archive name changed.
echo "Downloading pentest-agent..."
if [ ! -f "pentest-agent" ]; then
    if command -v gh &> /dev/null; then
        gh release download $PICK_VERSION \
            --repo Strike48-public/pick \
            --pattern "pick-agent-linux-x86_64.tar.gz" \
            2>/dev/null || {
            echo "WARNING: Could not download pentest-agent from GitHub"
            echo "Try: wget https://github.com/Strike48-public/pick/releases/download/$PICK_VERSION/pick-agent-linux-x86_64.tar.gz"
        }
    else
        wget -q "https://github.com/Strike48-public/pick/releases/download/$PICK_VERSION/pick-agent-linux-x86_64.tar.gz" || {
            echo "WARNING: Could not download pentest-agent"
        }
    fi

    if [ -f "pick-agent-linux-x86_64.tar.gz" ]; then
        # Verify against the release's SHA256SUMS.txt before trusting the archive:
        # this executable ships inside user-facing installers, so a tampered or
        # re-uploaded asset must fail the build rather than flow into a bundle.
        rm -f SHA256SUMS.txt
        if command -v gh &> /dev/null; then
            gh release download "$PICK_VERSION" --repo Strike48-public/pick \
                --pattern "SHA256SUMS.txt" --dir . 2>/dev/null || true
        fi
        if [ ! -s SHA256SUMS.txt ]; then
            wget -q "https://github.com/Strike48-public/pick/releases/download/$PICK_VERSION/SHA256SUMS.txt" -O SHA256SUMS.txt || true
        fi
        sum_line="$(awk '$2=="pick-agent-linux-x86_64.tar.gz"' SHA256SUMS.txt 2>/dev/null || true)"
        if [ -z "$sum_line" ]; then
            echo "ERROR: no SHA256 entry for pick-agent-linux-x86_64.tar.gz in pick $PICK_VERSION - refusing to bundle an unverified connector." >&2
            exit 1
        fi
        if ! printf '%s\n' "$sum_line" | sha256sum -c -; then
            echo "ERROR: SHA256 checksum mismatch for pick-agent-linux-x86_64.tar.gz." >&2
            exit 1
        fi
        rm -f SHA256SUMS.txt
        tar -xzf pick-agent-linux-x86_64.tar.gz
        rm -f pick-agent-linux-x86_64.tar.gz
        echo "✓ pentest-agent downloaded and checksum-verified"
    fi
else
    echo "✓ pentest-agent already exists"
fi

# Fail loud rather than building an AppImage without the Pick connector: a bad
# asset name or a failed download must not silently produce a broken bundle.
if [ ! -f "pentest-agent" ]; then
    echo "ERROR: pentest-agent binary missing - cannot bundle the Pick connector." >&2
    echo "Ensure pick $PICK_VERSION publishes pick-agent-linux-x86_64.tar.gz." >&2
    exit 1
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

# Fail loud rather than bundling an AppImage without the KubeStudio connector -
# the same silent-failure class the pentest-agent guard above closes. (kubestudio
# publishes no SHA256SUMS.txt for ks-connector, so there is no checksum to verify
# here, unlike the pick archive above.)
if [ ! -f "ks-connector" ]; then
    echo "ERROR: ks-connector binary missing - cannot bundle the KubeStudio connector." >&2
    echo "Ensure kubestudio $KUBESTUDIO_VERSION publishes ks-connector-linux-x86_64.tar.gz." >&2
    exit 1
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
