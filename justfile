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

# Build both connectors at the versions pinned in connector-versions.env
build-pinned:
    #!/usr/bin/env bash
    set -euo pipefail
    source connector-versions.env
    echo "Building Pick at $PICK_VERSION, KubeStudio at $KUBESTUDIO_VERSION"
    just build-pick-at "$PICK_VERSION"
    just build-kube-at "$KUBESTUDIO_VERSION"
    echo "✓ All connectors built at pinned versions"

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
