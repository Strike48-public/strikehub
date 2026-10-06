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

### VC++ runtime (app-local deployment)

`strikehub.exe` and the connector binaries are built with the default MSVC
**dynamic** CRT, so they import `VCRUNTIME140.dll` (all) and
`VCRUNTIME140_1.dll` (strikehub). A clean Windows 10 machine ships no VC++
redist, so the MSI carries exactly those DLLs **next to every executable it
installs** (app-local deployment; see `wix\main.wxs`). Verify any binary's
imports with:

```powershell
& "C:\Program Files (x86)\Microsoft Visual Studio\<year>\BuildTools\VC\Tools\MSVC\<ver>\bin\Hostx64\x64\dumpbin.exe" /dependents strikehub.exe
```

`scripts\build-msi.ps1` stages the DLLs into `redist\` **at build time**
(never vendored in git). Provenance, in search order:

1. `$env:SHVCREDIST_DIR` (explicit override: a directory containing the DLLs)
2. `<VS year>\<flavor>\VC\Redist\MSVC\<ver>\<arch>\Microsoft.VC143.CRT\` — the VC++ redistributable folder (newest `MSVC\<ver>` wins). This is the canonical location on the GitHub-hosted Windows runners, where VS is installed under `C:\Program Files (x86)\Microsoft Visual Studio\<year>\<flavor>\`. The year directory moves with runner re-bakes (2022, 2025, ...), so the script probes a range of years under both Program Files bases.
3. `<VS year>\<flavor>\VC\Tools\MSVC\<ver>\bin\Hostx64\<arch>\` (Build Tools / Community / Professional / Enterprise)
4. `C:\Program Files (x86)\Windows Kits\10\bin\<sdkver>\<arch>\`
5. `C:\Windows\System32\` — where the VC++ redist MSI installs the identical redistributable bits (license-equivalent copies)

All of these hold the same redistributable CRT the toolchain itself uses to
run its host tools. Each staged DLL is PE-machine-verified
(`0x8664` x64 / `0xAA64` arm64) and its SHA256 is printed to the build log.
If the DLLs cannot be found, the MSI build fails loudly — do not work around
it by deleting the check.

CI additionally runs `scripts\verify-crt-coverage.ps1`, which locates
`dumpbin` at runtime (vswhere first, then VS install globs across the
supported year directories under both Program Files bases, then PATH) and
scans every shipped exe, failing the build if any import is neither
OS-provided nor staged in `redist\`. If a future binary starts importing a
new non-OS DLL (e.g. `MSVCP140.dll`), extend `VC_RUNTIME_DLLS` in
`crates/sh-core/src/connector_seed.rs`, the staging list in
`build-msi.ps1`, and the `File` elements in `wix\main.wxs` together — the
three lists must stay in lockstep.

At first run, the app also copies the app-local DLLs into the per-user
connector cache (`%USERPROFILE%\.strike48\strikehub\bin\`), because cached /dynamically-fetched connector binaries run from there and the
installer's copies in `Program Files` are not on that process's DLL search
path (`seed_vc_runtime` in `crates/sh-core/src/connector_seed.rs`).

Known gap: the SFX build (`scripts\build-windows-sfx.sh`, not part of CI
releases) does not stage the DLLs yet — the SFX `StrikeHub-*.exe` will hit
the same clean-machine failure until it is updated.

### Binary layout

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
