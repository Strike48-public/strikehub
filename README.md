# StrikeHub

A native desktop shell that unifies Strike48 connector applications into a single window. Discover, launch, and manage tools like KubeStudio, JiraStudio, GitLabStudio, StrikeOffice, and StrikeTeam without alt-tabbing between separate apps.

## Features

**Connector Management**
- Discover and display connectors in an icon rail sidebar
- Switch between connectors with click or `Cmd+N` shortcuts
- Start/stop connectors from the UI
- Status indicators (online/offline/checking) with periodic health checks
- Dynamic metadata fetching from `/connector/info` endpoints

**Dual Transport Modes**
- **IPC** - Child-process connectors communicating over Unix domain sockets (`connector://` custom protocol)
- **TCP** (legacy) - In-process or external HTTP servers on localhost ports with auth proxy

**Authentication**
- OIDC authentication with Matrix/Keycloak
- System-browser OAuth flow (opens default browser for login)
- Automatic token injection into connector HTML
- Token refresh loop with session management

**WebSocket Bridging**
- Single-port WsRelay bridges Dioxus liveview and Matrix GraphQL subscriptions
- Bidirectional frame forwarding over Unix sockets

**Configuration**
- TOML config at the platform config dir (`~/Library/Application Support/strikehub/connectors.toml` on macOS, `~/.config/strikehub/connectors.toml` on Linux, `%APPDATA%\strikehub\connectors.toml` on Windows)
- Builtin connector manifests with forward-compatible defaults
- Dynamic connector definitions loaded from config at runtime
- Allowlist controls which GitHub repos may provide connector binaries
- Custom connector registration by socket path
- Per-connector transport and environment overrides

## Installation

### Prerequisites

- **Rust 1.91.1+** (specified in `.tool-versions`)
- **macOS or Linux**
- **Matrix server** (optional, for authentication features)

### Quick Start

#### Linux (AppImage)

```bash
# Build AppImage with all connectors included
./scripts/build-and-run-appimage.sh
```

This creates a portable AppImage that includes:
- StrikeHub desktop application
- KubeStudio connector (`ks-connector`)
- Pick connector (`pentest-agent`)
- Default configuration for Strike48 API

#### Windows

Download the latest `strikehub-windows-x86_64.exe` from releases and double-click to run. Everything is bundled — no installation or configuration required.

### Build from Source

```bash
git clone https://github.com/Strike48/strikehub.git
cd strikehub

# Desktop mode (native window with Wry webview)
cargo run --features desktop

# Server UI mode (web-based liveview, accessible via browser)
cargo run --bin strikehub-server --features server --no-default-features -p sh-ui
```

### Release Build

```bash
cargo build --release --features desktop
./target/release/strikehub
```

Connector binaries are downloaded automatically on first launch. See the [Connector Setup](#connector-setup) section for details.

## Connector Setup

### Runtime Download (default)

On first launch, StrikeHub automatically downloads connector binaries from
GitHub Releases and caches them in `~/.strike48/strikehub/bin/`. Subsequent
launches use the cached version and check for updates in the background.

- SHA256 checksums are verified when available (`SHA256SUMS.txt` or `.sha256` sidecar).
- On macOS, downloaded binaries are ad-hoc codesigned to satisfy Gatekeeper.
- If GitHub is unreachable, the last cached version is used as a fallback.

### CI / AppImage Bundling

When you push a tag to trigger a release, CI downloads connectors and bundles
them into the release artifact. The `build-appimage-with-connectors.sh` script
does the same for local AppImage builds.

### Manual Override

StrikeHub resolves connector binaries in this order:
1. Same directory as the `strikehub` binary
2. Sibling Cargo workspace `target/` dirs (development)
3. Cache dir (`~/.strike48/strikehub/bin/`)
4. System `PATH`

Placing a binary in location 1 or 2 takes precedence over the cached download.

## Configuration

### Environment Variables

| Variable | Purpose | Required |
|----------|---------|----------|
| `STRIKE48_API_URL` | Strike48 API / Keycloak server URL | For auth features |
| `MATRIX_TLS_INSECURE` | Skip TLS verification (`true` / `1`) | No |
| `STRIKEHUB_ALLOWED_SOURCES` | Comma-separated allowlist overriding config and compile-time defaults | No |
| `RUST_LOG` | Logging level (`info`, `debug`) | No |

### Connector Config

Connectors are configured in `connectors.toml` inside the platform config directory (auto-created on first run):

| Platform | Path |
|----------|------|
| macOS | `~/Library/Application Support/strikehub/connectors.toml` |
| Linux | `~/.config/strikehub/connectors.toml` |
| Windows | `%APPDATA%\strikehub\connectors.toml` |

```toml
[connectors.kubestudio]
display_name = "KubeStudio"
binary = "ks-connector"
port = 3030
icon = "hero-server-stack"
auto_start = true
transport = "ipc"

[connectors.custom-external]
display_name = "External Service"
icon = "app"
enabled = true
transport = "ipc"
socket_path = "/tmp/my-app.sock"
```

### Dynamic Connectors

Third-party or internal connectors can be added at runtime without recompiling StrikeHub. Define them in the `[[dynamic_connectors]]` array in `connectors.toml`:

```toml
[[dynamic_connectors]]
id = "internal-tool"
name = "Internal Tool"
description = "Custom internal dashboard"
icon = "hero-puzzle-piece"
github_repo = "my-corp/internal-tool"
binary_hint = "internal-tool-agent"
asset_pattern = "internal-tool-agent-{os}-{arch}.{ext}"
```

Dynamic connectors are fetched from GitHub Releases using the same download/cache/checksum pipeline as builtins, with two additional restrictions:

- The `github_repo` must be permitted by the **allowlist** (see below).
- A SHA256 checksum file (`SHA256SUMS.txt` or `{asset}.sha256`) **must** be present in the release. Dynamic connectors without checksums are refused.

If a dynamic connector's `id` collides with a builtin, the builtin always wins and the dynamic entry is skipped. Dynamic connectors without a `github_repo` (local socket-only) skip the allowlist check.

### Allowlist

The allowlist controls which GitHub `owner/repo` sources are permitted for connector binary downloads. It supports org-level wildcards and exact matches:

```toml
[allowlist]
sources = [
    "Strike48-public/*",
    "my-corp/internal-tool",
]
```

**Precedence (replace semantics):**

1. **Compile-time defaults** from `build-defaults.toml` (baked into the binary via `build.rs`)
2. **Config file** `[allowlist] sources` in `connectors.toml` — replaces compile-time defaults
3. **Environment variable** `STRIKEHUB_ALLOWED_SOURCES` (comma-separated) — replaces all

Builtin connectors (compiled into StrikeHub) always bypass the allowlist.

## Architecture

```
strikehub/
├── crates/
│   ├── sh-core/        # Core library: config, IPC, auth, proxy, WebSocket relay,
│   │                   # connector binary fetch, dynamic registry, allowlist
│   └── sh-ui/          # UI components, desktop app (Wry), and server app (Axum liveview)
```

### Tech Stack

| Component | Technology |
|-----------|------------|
| Language | Rust 2024 edition |
| UI Framework | Dioxus 0.6 (desktop) |
| WebView | Wry |
| HTTP Server | Axum 0.7 |
| Async Runtime | Tokio |
| IPC | Unix domain sockets + Hyper |
| WebSocket | Tokio-tungstenite |
| Auth | OIDC / OAuth (Keycloak) |

### IPC Request Flow

1. Iframe loads `connector://kubestudio/liveview`
2. Wry custom protocol handler intercepts the request
3. HTTP request forwarded to the connector's Unix socket
4. HTML response rewritten (auth token, API URL, WebSocket URL injected)
5. Response returned to the webview
6. WebSocket traffic routed through the WsRelay bridge

## Development

```bash
# Run with debug logging
RUST_LOG=debug cargo run --features desktop

# Run with Matrix auth
STRIKE48_API_URL=https://studio.strike48.test cargo run --features desktop

# Run tests
cargo test --workspace
```

See `PRD.md` and `PRD-IPC.md` for detailed requirements and IPC architecture.

## License

This project is licensed under the **Mozilla Public License 2.0** (MPL-2.0). See [LICENSE](LICENSE) for the full text.

- You are free to use, modify, and distribute this software
- Modifications to MPL-licensed files must be shared under MPL-2.0
- MPL-2.0 is compatible with integration into larger works under other licenses

For the full Strike48 platform Terms of Service, see [strike48.com/terms-of-service](https://www.strike48.com/terms-of-service).

---

**Built with Rust and Dioxus**
