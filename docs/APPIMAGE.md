# AppImage Build Documentation

## Overview

StrikeHub provides AppImage packages for Linux users, offering a portable, distribution-agnostic way to run the application without installation.

## What is AppImage?

AppImage is a format for distributing portable software on Linux without needing superuser permissions to install. The application runs directly from the AppImage file and includes all necessary dependencies.

## Building AppImage Locally

### Prerequisites

```bash
# Install required dependencies on Ubuntu/Debian
# NOTE: no host FUSE library is needed — the AppImage embeds a static
# FUSE-3 type-2 runtime (only fusermount3 is used at run time, and it is
# present on stock Ubuntu 24.04+).
sudo apt-get update
# Pin the full six-package WebKitGTK set to 2.44.0-2 to match CI — building
# against a newer webkit (>= 2.46) produces an AppImage whose web process
# requires EGL and black-screens GPU-less hosts.
sudo apt-get install -y \
    libwebkit2gtk-4.1-0=2.44.0-2 \
    libwebkit2gtk-4.1-dev=2.44.0-2 \
    gir1.2-webkit2-4.1=2.44.0-2 \
    libjavascriptcoregtk-4.1-0=2.44.0-2 \
    libjavascriptcoregtk-4.1-dev=2.44.0-2 \
    gir1.2-javascriptcoregtk-4.1=2.44.0-2 \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    libxdo-dev \
    libegl1 \
    libgbm1 \
    libwayland-client0 \
    wget \
    file \
    imagemagick
```

The reduced graphics runtime (`libegl1 libgbm1 libwayland-client0` — the
glvnd libEGL dispatcher, GBM and wayland clients the binaries NEED at load
time) is bundled **into** the AppImage (Phase 2d of the build script), so
the build host must have it — the script fails loudly if any piece is
missing. No GL stack (libgl1/mesa/DRI) is needed at build time or bundled:
rendering uses the non-compositing software path.
**Build on the oldest supported glibc (Ubuntu 24.04)** so the bundled libs
stay inside the floor enforced by `scripts/check-glibc-floor.sh`.

### Build Process

#### Quick Build (with connectors auto-downloaded)

```bash
# Build AppImage with connectors included
./scripts/build-appimage-with-connectors.sh 1.0.0 x86_64

# Or use the one-command build and run:
./scripts/build-and-run-appimage.sh
```

#### Manual Build

1. **Build the release binary:**
   ```bash
   cargo build --release --target x86_64-unknown-linux-gnu --no-default-features --features desktop
   ```

2. **Optional: Download connectors to include:**
   ```bash
   mkdir -p dist
   cd dist
   # Download from GitHub releases (adjust versions as needed)
   wget https://github.com/Strike48-public/kubestudio/releases/download/v0.1.0/ks-connector-linux-x86_64.tar.gz
   wget https://github.com/Strike48-public/pick/releases/download/v0.1.0/pentest-agent-linux-x86_64.tar.gz
   tar -xzf ks-connector-linux-x86_64.tar.gz
   tar -xzf pentest-agent-linux-x86_64.tar.gz
   cd ..
   ```

3. **Run the AppImage build script:**
   ```bash
   ./scripts/build-appimage.sh 1.0.0 x86_64
   ```

   This will create `StrikeHub-1.0.0-x86_64.AppImage`

### Environment Configuration

The AppImage automatically sets default environment variables:
- `STRIKE48_API_URL=https://studio.strike48.test` (default Strike48 API server)

You can override these when running:
```bash
STRIKE48_API_URL=https://your.server ./StrikeHub-*.AppImage
MATRIX_TLS_INSECURE=true ./StrikeHub-*.AppImage  # For self-signed certs
```

Or create a `.env` file (see `.env.example`) for persistent configuration during builds.

## CI/CD Integration

The AppImage build is automatically triggered on GitHub Actions when a new tag is pushed:

1. The workflow builds the Rust binary with desktop features
2. Downloads and bundles the connectors (ks-connector, pentest-agent)
3. Uses linuxdeploy with GTK plugin to create the AppImage
4. Uploads the AppImage to the GitHub release

## Running the AppImage

```bash
# Make it executable (first time only)
chmod +x StrikeHub-*.AppImage

# Run the application
./StrikeHub-*.AppImage
```

## Host runtime requirements

The AppImage bundles the GTK3 + WebKitGTK 4.1 runtime (pinned to 2.44.0-2,
the last pre-mandatory-EGL release), including WebKit's out-of-process
helpers, **and the reduced graphics runtime** (wayland client, the glvnd
libEGL dispatcher, libgbm — the load-time deps the binaries NEED). **No GL
stack is bundled**: with `WEBKIT_DISABLE_COMPOSITING_MODE=1` (the AppRun
hook default) the web process renders via the non-compositing shared-memory
path and never touches GL/EGL, so GPU-less hosts need no GL packages. The
bundled libs' `rpath` points inside the bundle, so the bundled runtimes are
always the ones used — no system `libwebkit2gtk-4.1-0` or wayland packages
are required, and differently-versioned system copies are ignored (which is
what previously hung startup on hosts like ubuntu 26.04 where the system
WebKit is newer than the bundled one).

**Support target: the AppImage runs with zero extra packages on stock Ubuntu
24.04/26.04, including minimal/server images** (22.04 and Debian 12 fall
below the 2.39 glibc floor). Verified on a strict zero-GL `ubuntu:24.04`
container (no GL/mesa/webkit packages at all) and an Ubuntu 26.04 VM (bare
launch, all webkit env combinations), plus the NixOS host: the app launches
to the sign-in UI and `ldd` on the extracted binaries reports no missing
libraries. (Historical note: before the wayland/EGL closure was bundled,
stock 22.04 exited 127 on `libwayland-client.so.0` — ldd also missing
`libgbm.so.1` + `libEGL.so.1` — and stock 26.04 failed the same way on
`libEGL.so.1`.) (Building from source instead requires `libegl1 libgbm1
libwayland-client0 ca-certificates libwebkit2gtk-4.1-0` on the desktop.
)`

No host FUSE **library** is required: the AppImage embeds the static FUSE-3
type-2 runtime (see below), which only uses `fusermount3` (present on stock
Ubuntu 24.04+ and most current distros).

- glibc floor: GLIBC 2.39 / GLIBCXX 3.4.32 (enforced by `scripts/check-glibc-floor.sh`).
- A display server is required (X11; Xvfb works for headless testing).
- `ca-certificates` must be present for TLS (native-tls) — preinstalled on
  every normal image, including the minimal ones above.

## Troubleshooting

### FUSE

The shipped AppImage embeds the **static FUSE-3 type-2 runtime**
([github.com/AppImage/type2-runtime](https://github.com/AppImage/type2-runtime),
pinned by release tag + sha256 in `scripts/build-appimage.sh`), so it needs
**no host FUSE library at all** — only `fusermount3`, which stock Ubuntu 24.04+
provides. (Older builds used the legacy AppImageKit runtime, which dlopened
`libfuse.so.2` and failed **silently** on stock Ubuntu 24.04+, which ships FUSE
3 only — Strike48/project-management#377 finding 2, resolved by #382.)

On a system without a working `fusermount3`, the AppImage still runs via
extraction:

```bash
APPIMAGE_EXTRACT_AND_RUN=1 ./StrikeHub-*.AppImage
```

### Extracting AppImage Contents

To inspect or extract the AppImage contents:
```bash
./StrikeHub-*.AppImage --appimage-extract
```

This creates a `squashfs-root` directory with all the bundled files.

## AppImage Structure

```
StrikeHub.AppDir/
├── AppRun                     # Entry point script
├── strikehub.desktop          # Desktop entry file
├── strikehub.svg              # Application icon
└── usr/
    ├── bin/
    │   ├── strikehub          # Main binary
    │   ├── ks-connector       # KubeStudio connector
    │   └── pentest-agent      # Pentest agent
    └── lib/                   # Bundled libraries
```

## Build Scripts

- `scripts/build-appimage.sh` - Main AppImage build script with environment variable support
- `scripts/build-appimage-with-connectors.sh` - Downloads connectors and builds AppImage
- `scripts/build-and-run-appimage.sh` - One-command build and run script
- `scripts/test-env-fix.sh` - Test script to verify environment variables are set

The build system:
- Sets default `STRIKE48_API_URL` and `STRIKE48_URL` environment variables
- Bundles connectors (ks-connector, pentest-agent) when available
- Ensures environment variables are always set via the generated `apprun-hooks/00-strike48-env.sh` AppRun hook, which the build script writes before the linuxdeploy call and the generated AppRun sources before exec'ing the app
- Creates a portable, self-contained AppImage

## Testing

To test the AppImage on different distributions, you can use Docker:

```bash
# Test on Ubuntu 24.04 (static FUSE-3 runtime: only fusermount3 is needed,
# which fuse3 provides; without it, APPIMAGE_EXTRACT_AND_RUN=1 works)
docker run -it --rm -v $(pwd):/app ubuntu:24.04 bash
cd /app
apt-get update && apt-get install -y fuse3
./StrikeHub-*.AppImage --appimage-extract >/dev/null   # no FUSE needed to inspect

# Test on Fedora
docker run -it --rm -v $(pwd):/app fedora:latest bash
cd /app
dnf install -y fuse-libs
./StrikeHub-*.AppImage --appimage-extract >/dev/null
```
