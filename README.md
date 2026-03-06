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
- TOML config at `~/.config/strikehub/connectors.toml`
- Builtin connector manifests with forward-compatible defaults
- Custom connector registration by socket path
- Per-connector transport and environment overrides

## Installation

### Prerequisites

- **Rust 1.91.1+** (specified in `.tool-versions`)
- **macOS or Linux**
- **Matrix server** (optional, for authentication features)

### Quick Start (Linux AppImage)

```bash
# Build AppImage with all connectors included
./scripts/build-and-run-appimage.sh
```

This creates a portable AppImage that includes:
- StrikeHub desktop application
- KubeStudio connector (`ks-connector`)
- Pick connector (`pentest-agent`)
- Default configuration for Strike48 API

### Build from Source

```bash
git clone https://github.com/Strike48/strikehub.git
cd strikehub
cargo run --features desktop
```

### Release Build

```bash
cargo build --release --features desktop
./target/release/strikehub
```

Note: When running from source, you'll need to install the connectors separately or build them in sibling directories. See the [Connector Setup](#connector-setup) section.

## Connector Setup

### Automatic (CI/AppImage)

**YES, the CI automatically downloads and bundles the connectors!** When you:
- Push a tag to trigger a release → CI downloads connectors and includes them
- Build the AppImage locally with our scripts → It downloads them for you

The `build-appimage-with-connectors.sh` script **automatically downloads** the connectors from GitHub releases.

### Manual Installation (if running from source)

If you're running the raw binary (not AppImage), you need the connectors in one of these locations:
1. Same directory as `strikehub` binary
2. In your PATH
3. Built in sibling workspace directories (for development)

## Configuration

### Environment Variables

| Variable | Purpose | Required |
|----------|---------|----------|
| `STRIKE48_API_URL` | Strike48 API / Keycloak server URL | For auth features |
| `MATRIX_TLS_INSECURE` | Skip TLS verification (`true` / `1`) | No |
| `RUST_LOG` | Logging level (`info`, `debug`) | No |

### Connector Config

Connectors are configured at `~/.config/strikehub/connectors.toml` (auto-created on first run):

```toml
[connectors.kubestudio]
display_name = "KubeStudio"
binary = "/path/to/ks-connector"
port = 3030
icon = "hero-server-stack"
auto_start = false
transport = "ipc"
socket_path = "/tmp/strikehub-kubestudio.sock"

[connectors.custom-external]
display_name = "External Service"
icon = "app"
enabled = true
transport = "ipc"
socket_path = "/tmp/my-app.sock"
```

## Architecture

```
strikehub/
├── crates/
│   ├── sh-core/        # Core library: config, IPC, auth, proxy, WebSocket relay
│   └── sh-ui/          # Dioxus desktop application and UI components
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
