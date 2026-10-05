# AppImage Build Documentation

## Overview

StrikeHub provides AppImage packages for Linux users, offering a portable, distribution-agnostic way to run the application without installation.

## What is AppImage?

AppImage is a format for distributing portable software on Linux without needing superuser permissions to install. The application runs directly from the AppImage file and includes all necessary dependencies.

## Building AppImage Locally

### Prerequisites

```bash
# Install required dependencies on Ubuntu/Debian
sudo apt-get update
sudo apt-get install -y \
    libwebkit2gtk-4.1-dev \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    libxdo-dev \
    libegl1 \
    libgbm1 \
    libwayland-client0 \
    libgl1 \
    libgl1-mesa-dri \
    wget \
    file \
    imagemagick \
    libfuse2
```

The graphics stack (`libegl1 libgbm1 libwayland-client0 libgl1 libgl1-mesa-dri`)
is bundled **into** the AppImage (Phase 2d of the build script), so the build
host must have it — the script fails loudly if any piece is missing.
**Build on the oldest supported glibc (Ubuntu 22.04)** so the bundled libs
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

The AppImage bundles the GTK3 + WebKitGTK 4.1 runtime, including WebKit's
out-of-process helpers, **and the full graphics stack** (wayland client, EGL,
GL/glvnd, mesa DRI drivers with the swrast/llvmpipe software renderer). Their
`rpath` points inside the bundle, so the bundled runtimes are always the ones
used — no system `libwebkit2gtk-4.1-0`, mesa, or wayland packages are
required, and differently-versioned system copies are ignored (which is what
previously hung startup on hosts like ubuntu 26.04 where the system WebKit is
newer than the bundled one).

**Support target: the AppImage runs with zero extra packages on stock Ubuntu
22.04/26.04, including minimal/server images.** Verified on pristine
`ubuntu:22.04` and `ubuntu:26.04` containers (only Xvfb installed, no
graphics packages): before the graphics stack was bundled, stock 22.04 exited
127 on `libwayland-client.so.0` (ldd also missing `libgbm.so.1` +
`libEGL.so.1`; `ks-connector` missing `libwayland-client.so.0`) and stock
26.04 failed the same way on `libEGL.so.1`; with the bundled closure the app
launches to the sign-in screen and `ldd` on the extracted binary reports no
missing libraries. (Building from source instead requires `libegl1 libgbm1
libwayland-client0 ca-certificates libwebkit2gtk-4.1-0` on the desktop.
)`

- glibc floor: GLIBC 2.35 / GLIBCXX 3.4.30 (enforced by `scripts/check-glibc-floor.sh`).
- A display server is required (X11; Xvfb works for headless testing).
- `ca-certificates` must be present for TLS (native-tls) — preinstalled on
  every normal image, including the minimal ones above.

## Troubleshooting

### FUSE Error

If you see an error about FUSE, install it:
```bash
sudo apt-get install libfuse2  # For Ubuntu 22.04+
# or
sudo apt-get install fuse       # For older distributions
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
# Test on Ubuntu 20.04
docker run -it --rm -v $(pwd):/app ubuntu:20.04 bash
cd /app
apt-get update && apt-get install -y libfuse2
./StrikeHub-*.AppImage --help

# Test on Fedora
docker run -it --rm -v $(pwd):/app fedora:latest bash
cd /app
dnf install -y fuse fuse-libs
./StrikeHub-*.AppImage --help
```
