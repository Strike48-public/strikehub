# StrikeHub development commands

# Paths to sibling connector repos
pick_dir     := env("PICK_DIR", home_directory() / "work/pick")
kube_dir     := env("KUBE_DIR", home_directory() / "work/kubestudio")
bin_dir      := env("BIN_DIR", home_directory() / "bin")

# Build all connectors and StrikeHub, copy binaries, then run
default: build-all run

# Build everything: connectors + hub
build-all: build-pick build-kube build-hub

# Build Pick connector (pentest-agent)
build-pick:
    cargo build --manifest-path {{pick_dir}}/Cargo.toml --bin pentest-agent
    cp {{pick_dir}}/target/debug/pentest-agent {{bin_dir}}/pentest-agent
    @echo "✓ pentest-agent updated"

# Build KubeStudio connector (ks-connector)
build-kube:
    cargo build --manifest-path {{kube_dir}}/Cargo.toml --bin ks-connector --features connector
    cp {{kube_dir}}/target/debug/ks-connector {{bin_dir}}/ks-connector
    @echo "✓ ks-connector updated"

# Build StrikeHub
build-hub:
    cargo build --features desktop
    @echo "✓ strikehub updated"

# Run StrikeHub: kill stale processes then launch (preserves credentials)
run: kill
    RUST_LOG=info cargo run --features desktop

# Clean rebuild everything from scratch
rebuild-all: clean-all build-all

# Clean all targets
clean-all:
    cargo clean
    cargo clean --manifest-path {{pick_dir}}/Cargo.toml
    cargo clean --manifest-path {{kube_dir}}/Cargo.toml

# Clear stale connector credentials
clear-creds:
    rm -rf ~/.strike48/credentials/*.json
    rm -rf ~/.strike48/keys/*.pem
    @echo "✓ stale credentials cleared"

# Kill all running StrikeHub, Pick, and KubeStudio processes
kill:
    -pkill -f "strikehub" 2>/dev/null
    -pkill -f "pentest-agent" 2>/dev/null
    -pkill -f "ks-connector" 2>/dev/null
    @echo "✓ all processes killed"

# Full fresh start: kill, clean creds, rebuild, run
fresh: kill clear-creds rebuild-all run

# Show connector versions from connector-versions.env and local repo HEADs
versions:
    @echo "=== connector-versions.env (CI/release) ==="
    @grep -v '^#' connector-versions.env | grep '=' | while IFS='=' read -r key val; do \
        printf "  %-25s %s\n" "$key" "$val"; \
    done
    @echo ""
    @echo "=== Local repo HEADs ==="
    @printf "  %-25s %s\n" "pick" "$(git -C {{pick_dir}} describe --tags --always 2>/dev/null || echo 'not found')"
    @printf "  %-25s %s\n" "kubestudio" "$(git -C {{kube_dir}} describe --tags --always 2>/dev/null || echo 'not found')"
    @echo ""
    @echo "=== Installed binaries ==="
    @if [ -f {{bin_dir}}/pentest-agent ]; then printf "  %-25s %s\n" "pentest-agent" "$(ls -lh {{bin_dir}}/pentest-agent | awk '{print $5, $6, $7, $8}')"; else echo "  pentest-agent             not found"; fi
    @if [ -f {{bin_dir}}/ks-connector ]; then printf "  %-25s %s\n" "ks-connector" "$(ls -lh {{bin_dir}}/ks-connector | awk '{print $5, $6, $7, $8}')"; else echo "  ks-connector              not found"; fi

# Build Pick at a specific git ref (tag, branch, or commit hash)
build-pick-at ref:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "→ Checking out {{ref}} in {{pick_dir}}"
    git -C {{pick_dir}} fetch --all --tags
    git -C {{pick_dir}} checkout {{ref}}
    cargo build --manifest-path {{pick_dir}}/Cargo.toml --bin pentest-agent
    cp {{pick_dir}}/target/debug/pentest-agent {{bin_dir}}/pentest-agent
    echo "✓ pentest-agent built at $(git -C {{pick_dir}} describe --tags --always)"

# Build KubeStudio at a specific git ref (tag, branch, or commit hash)
build-kube-at ref:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "→ Checking out {{ref}} in {{kube_dir}}"
    git -C {{kube_dir}} fetch --all --tags
    git -C {{kube_dir}} checkout {{ref}}
    cargo build --manifest-path {{kube_dir}}/Cargo.toml --bin ks-connector --features connector
    cp {{kube_dir}}/target/debug/ks-connector {{bin_dir}}/ks-connector
    echo "✓ ks-connector built at $(git -C {{kube_dir}} describe --tags --always)"

# Build both connectors at the git refs pinned in connector-versions.env
build-pinned:
    #!/usr/bin/env bash
    set -euo pipefail
    source connector-versions.env
    echo "Building Pick at $PICK_REF, KubeStudio at $KUBESTUDIO_REF"
    just build-pick-at "$PICK_REF"
    just build-kube-at "$KUBESTUDIO_REF"
    echo "✓ All connectors built at pinned refs"

# Update connector-versions.env to the latest main HEAD of each connector repo
pin:
    #!/usr/bin/env bash
    set -euo pipefail
    pick_sha=$(git -C {{pick_dir}} ls-remote origin main | awk '{print $1}')
    kube_sha=$(git -C {{kube_dir}} ls-remote origin main | awk '{print $1}')
    printf "PICK_REF=%s\nKUBESTUDIO_REF=%s\n" "$pick_sha" "$kube_sha" > connector-versions.env
    echo "✓ Pinned connector refs:"
    cat connector-versions.env

# Build on a remote host via SSH.
# Syncs the repo (excluding target/), builds natively, and copies the
# artifact back. First run installs Rust and system deps automatically.
#
# Examples:
#   just remote-build dgx-spark appimage 0.1.0
#   just remote-build my-arm-box msi 0.2.0
#   just remote-build user@10.0.0.5 appimage
remote-build host artifact="appimage" version="0.0.0-dev":
    #!/usr/bin/env bash
    set -euo pipefail

    HOST="{{host}}"
    ARTIFACT="{{artifact}}"
    VERSION="{{version}}"
    REMOTE_DIR="strikehub-build"

    echo "→ Remote build: ${ARTIFACT} v${VERSION} on ${HOST}"

    # Bootstrap: install Rust and build deps if not present
    echo "→ Checking build environment on ${HOST}..."
    ssh "$HOST" bash -s << 'BOOTSTRAP'
    set -e
    NEED_DEPS=0

    # Check Rust
    if ! command -v cargo &>/dev/null; then
        echo "  Installing Rust toolchain..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        NEED_DEPS=1
    fi
    source "$HOME/.cargo/env" 2>/dev/null || true
    echo "  rustc: $(rustc --version)"

    # Check system deps (only try to install if missing)
    if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
        NEED_DEPS=1
    fi

    if [ "$NEED_DEPS" = "1" ]; then
        echo "  Installing system build dependencies..."
        sudo apt-get update -qq
        sudo apt-get install -y -qq \
            build-essential pkg-config git \
            libwebkit2gtk-4.1-dev libgtk-3-dev \
            libayatana-appindicator3-dev libxdo-dev \
            libpcap-dev libssl-dev \
            file wget imagemagick libfuse2 \
            protobuf-compiler
    fi
    echo "  ✓ Build environment ready"
    BOOTSTRAP

    # Sync the repo to the remote (exclude build artifacts)
    echo "→ Syncing repo to ${HOST}:~/${REMOTE_DIR}/..."
    rsync -az --delete \
        --exclude='target/' \
        --exclude='.git/objects/' \
        --exclude='*.AppImage' \
        --exclude='*.msi' \
        --exclude='StrikeHub.AppDir/' \
        --exclude='appimagetool-*.AppImage' \
        -e ssh \
        ./ "${HOST}:~/${REMOTE_DIR}/"

    # Also sync .git/HEAD and refs so git works on remote
    rsync -az -e ssh ./.git/ "${HOST}:~/${REMOTE_DIR}/.git/" \
        --include='HEAD' --include='refs/***' --include='config' \
        --exclude='objects/' --exclude='hooks/'

    # Determine the target triple from remote arch
    REMOTE_ARCH=$(ssh "$HOST" uname -m)
    case "$REMOTE_ARCH" in
        x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
        aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
        *)       echo "ERROR: Unsupported remote arch: $REMOTE_ARCH"; exit 1 ;;
    esac
    echo "→ Remote arch: ${REMOTE_ARCH} (target: ${TARGET})"

    # Run the build on the remote
    echo "→ Building on ${HOST}..."
    ssh "$HOST" bash -s "$REMOTE_DIR" "$TARGET" "$VERSION" "$ARTIFACT" "$REMOTE_ARCH" << 'BUILD'
    set -ex
    REMOTE_DIR="$1"; TARGET="$2"; VERSION="$3"; ARTIFACT="$4"; ARCH="$5"
    source "$HOME/.cargo/env"
    cd ~/"$REMOTE_DIR"

    # Ensure the right Rust target is installed
    rustup target add "$TARGET"

    # Build StrikeHub
    cargo build --release --target "$TARGET" --no-default-features --features desktop

    # Build connectors
    source connector-versions.env
    mkdir -p dist

    if [ ! -d /tmp/pick ]; then
        git clone https://github.com/Strike48-public/pick.git /tmp/pick
    fi
    git -C /tmp/pick fetch --all
    git -C /tmp/pick checkout "$PICK_REF"
    cargo build --release --manifest-path /tmp/pick/Cargo.toml --bin pentest-agent --target "$TARGET"
    cp "/tmp/pick/target/${TARGET}/release/pentest-agent" dist/

    if [ ! -d /tmp/kubestudio ]; then
        git clone https://github.com/Strike48-public/kubestudio.git /tmp/kubestudio
    fi
    git -C /tmp/kubestudio fetch --all
    git -C /tmp/kubestudio checkout "$KUBESTUDIO_REF"
    cargo build --release --manifest-path /tmp/kubestudio/Cargo.toml --bin ks-connector --features connector --target "$TARGET"
    cp "/tmp/kubestudio/target/${TARGET}/release/ks-connector" dist/

    # Build the artifact
    case "$ARTIFACT" in
        appimage)
            chmod +x scripts/build-appimage.sh
            ./scripts/build-appimage.sh "$VERSION" "$ARCH"
            echo "RESULT=$(ls -1 StrikeHub-*.AppImage)"
            ;;
        msi)
            echo "ERROR: MSI builds require Windows"; exit 1
            ;;
        *)
            echo "ERROR: Unknown artifact type: $ARTIFACT"; exit 1
            ;;
    esac
    BUILD

    # Copy the built artifact back
    echo "→ Fetching artifact from ${HOST}..."
    case "$ARTIFACT" in
        appimage)
            scp "${HOST}:~/${REMOTE_DIR}/StrikeHub-*-${REMOTE_ARCH}.AppImage" .
            RESULT="$(ls -1t StrikeHub-*-${REMOTE_ARCH}.AppImage 2>/dev/null | head -1)"
            ;;
    esac

    if [ -n "${RESULT:-}" ]; then
        echo ""
        echo "✅ ${RESULT} ($(du -h "$RESULT" | cut -f1))"
    else
        echo ""
        echo "❌ Build failed — no artifact found."
        exit 1
    fi


# Package StrikeHub with connector binaries into a dist directory.
# Copies strikehub + ks-connector + pentest-agent next to each other
# so the app can find them at runtime via resolve_binary().
package profile="debug":
    #!/usr/bin/env bash
    set -euo pipefail
    dist="dist/{{profile}}"
    mkdir -p "$dist"
    hub="target/{{profile}}/strikehub"
    pick_bin="{{pick_dir}}/target/{{profile}}/pentest-agent"
    kube_bin="{{kube_dir}}/target/{{profile}}/ks-connector"
    # On Windows, append .exe
    if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" ]] || command -v cmd.exe &>/dev/null; then
        hub="${hub}.exe"
        pick_bin="${pick_bin}.exe"
        kube_bin="${kube_bin}.exe"
    fi
    for bin in "$hub" "$pick_bin" "$kube_bin"; do
        if [ ! -f "$bin" ]; then
            echo "ERROR: $bin not found — build all targets first (just build-all)"
            exit 1
        fi
        cp "$bin" "$dist/"
        echo "  → $(basename "$bin")"
    done
    echo "✓ Packaged to $dist/"
