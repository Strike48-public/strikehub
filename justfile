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
