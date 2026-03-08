# Building StrikeHub on Windows ARM64

## Prerequisites

### Required for StrikeHub (sh-ui)

1. **Rust toolchain** (aarch64-pc-windows-msvc)
   ```powershell
   winget install Rustlang.Rustup --source winget --accept-source-agreements --accept-package-agreements
   rustup default stable
   ```

2. **Visual Studio Build Tools** with C++ ARM64 workload
   ```powershell
   winget install Microsoft.VisualStudio.2022.BuildTools --source winget --accept-source-agreements --accept-package-agreements
   # Then install the C++ ARM64 workload via the VS Installer
   ```

### Additional requirements for connector binaries

3. **LLVM/Clang** — required by the `ring` crate (used by both connectors)
   ```powershell
   winget install LLVM.LLVM --source winget --accept-source-agreements --accept-package-agreements
   ```
   Installs to `C:\Program Files\LLVM\bin\clang.exe` — must be on PATH.

4. **Protocol Buffers (protoc)** — required by `strike48-proto`
   ```powershell
   winget install Google.Protobuf --source winget --accept-source-agreements --accept-package-agreements
   ```

5. **Npcap SDK** — required by Pick's `pcap` dependency
   - Download the Npcap SDK from https://npcap.com/#download
   - Extract and add the lib path to `LIB` environment variable

## Build Environment

The network share (Z: drive) does not support temp file operations needed
by cargo. Use a local `CARGO_TARGET_DIR`:

```powershell
$env:CARGO_TARGET_DIR = 'C:\build\strikehub\target'
```

MSVC environment variables must be set for connector builds. Use the
`build_connectors.ps1` script or set manually:

```powershell
$msvcBase = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.44.35207'
$sdkBase = 'C:\Program Files (x86)\Windows Kits\10'
$sdkVer = '10.0.26100.0'

$env:VCINSTALLDIR = "$msvcBase\..\.."
$env:VCToolsVersion = '14.44.35207'
$env:INCLUDE = "$msvcBase\include;$sdkBase\Include\$sdkVer\ucrt;$sdkBase\Include\$sdkVer\um;$sdkBase\Include\$sdkVer\shared"
$env:LIB = "$msvcBase\lib\arm64;$sdkBase\Lib\$sdkVer\ucrt\arm64;$sdkBase\Lib\$sdkVer\um\arm64"
$env:PATH = "C:\Program Files\LLVM\bin;$msvcBase\bin\Hostarm64\arm64;$env:USERPROFILE\.cargo\bin;$env:PATH"
```

## Building

### StrikeHub only

```powershell
$env:CARGO_TARGET_DIR = 'C:\build\strikehub\target'
cd Z:\strikehub
cargo build --features desktop
```

### All binaries (strikehub + connectors)

```powershell
# Set MSVC env (see above), then:
$env:CARGO_TARGET_DIR = 'C:\build\strikehub\target'
cd Z:\strikehub
cargo build --features desktop

$env:CARGO_TARGET_DIR = 'C:\build\kubestudio\target'
cd Z:\kubestudio
cargo build --bin ks-connector --features connector

$env:CARGO_TARGET_DIR = 'C:\build\pick\target'
cd Z:\pick
cargo build --bin pentest-agent
```

## Packaging

After building, connector binaries must be placed next to `strikehub.exe`
so the app finds them via `resolve_binary()`:

```
dist/
  strikehub.exe
  ks-connector.exe
  pentest-agent.exe
```

Use `just package` from the strikehub repo (macOS/Linux), or manually copy:

```powershell
$dest = 'C:\dist\strikehub'
mkdir $dest -Force
copy C:\build\strikehub\target\debug\strikehub.exe $dest\
copy C:\build\kubestudio\target\debug\ks-connector.exe $dest\
copy C:\build\pick\target\debug\pentest-agent.exe $dest\
```

## Known Issues

- **Network share caching**: Files edited on macOS may not be immediately
  visible to cargo on the Windows side. Use `cargo clean -p <crate>` to
  force recompilation, or SCP files directly to C: and copy to Z:.

- **Console window**: `strikehub.exe` uses `#![windows_subsystem = "windows"]`
  to suppress the console. Debug builds also suppress it.

- **PATH refresh**: After installing tools via winget, the app refreshes
  PATH from the registry before running preflight checks, so newly
  installed tools are detected without restarting the app.
