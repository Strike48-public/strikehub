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
    wget \
    file \
    imagemagick \
    libfuse2
```

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
out-of-process helpers. Their `rpath` points inside the bundle, so the
bundled WebKit is always the one used — a system `libwebkit2gtk-4.1-0` is
**not** required (and a differently-versioned one is ignored, which is what
previously hung startup on hosts like ubuntu 26.04 where the system WebKit
is newer than the bundled one).

Remaining host dependencies (verified in containers against ubuntu 22.04 and
ubuntu 26.04):

| Package | Why |
|---|---|
| `libegl1` | EGL dispatch for WebKit's GL path |
| `libgl1` | Host OpenGL/GLES (`libGLESv2.so.2`) — without it the app aborts during WebView init |
| `libgbm1` | Buffer management (Mesa), needed by the bundled WebKitGTK |
| `libx11-6` | X11 client libs (the app runs `GDK_BACKEND=x11`) |
| `libwayland-client0` | Wayland client libs needed by GTK3 (inert under `GDK_BACKEND=x11`; the host's copy must be used — bundling an older one breaks WebKit EGL init on newer hosts) |
| `ca-certificates` | TLS verification for the studio connection (native-tls) |

These are preinstalled on ordinary desktop systems; a minimal container needs
`apt-get install libegl1 libgl1 libgbm1 libx11-6 libwayland-client0 ca-certificates`
plus any font package (e.g. `fonts-dejavu-core`).

- glibc floor: GLIBC 2.35 / GLIBCXX 3.4.30 (enforced by `scripts/check-glibc-floor.sh`).
- A display server is required (X11; Xvfb works for headless testing).

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
- Uses a wrapper script to ensure environment variables are always set
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
