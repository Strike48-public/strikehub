# PRD-IPC: Child-Process IPC for Connector Integration

## 1. Overview

This document specifies the migration from in-process TCP connector hosting to child-process IPC for StrikeHub connector integration. The goal is to decouple connector code from the StrikeHub binary — connectors run as independent child processes and communicate with StrikeHub over Unix domain sockets rather than compiled-in library dependencies and localhost TCP ports.

KubeStudio is the first connector to migrate. The architecture generalizes to all connectors.

### Motivation

Today, StrikeHub compiles KubeStudio's `ks_ui` crate directly into the host binary via `ConnectorRunner::start_kubestudio()`. This creates several problems:

1. **Tight coupling**: Changing KubeStudio requires rebuilding StrikeHub.
2. **Feature-gate sprawl**: Each connector needs a cargo feature and conditional compilation.
3. **Port exposure**: Every connector opens a TCP port on localhost. With N connectors, N+1 ports are open (N connectors + 1 proxy).
4. **No process isolation**: A connector panic or memory leak takes down the entire shell.
5. **Dependency conflicts**: Connector crates can introduce version conflicts in the workspace.

### What Changes

| Aspect | Before (TCP) | After (IPC) |
|--------|-------------|-------------|
| Connector hosting | In-process `tokio::spawn` | Child process via `Command` |
| Transport | TCP on `127.0.0.1:{port}` | Unix domain socket at `/tmp/strikehub-{id}.sock` |
| Content delivery | `http://127.0.0.1:{port}/...` via iframe | `connector://{id}/...` via wry custom protocol |
| WebSocket | Direct TCP connection to connector port | Single shared WS bridge on one ephemeral TCP port |
| Open TCP ports | N+1 (N connectors + 1 proxy) | 1 (WS bridge only) |
| StrikeHub dependency on connector crate | Required (`ks_ui`) | None |

### What Does Not Change

- **User-facing rendering**: Users still see KubeStudio inside StrikeHub's content area via iframe, with full interactivity, chat panel, and Matrix agent features. The transport is invisible.
- **Auth injection**: Same HTML rewriting (Matrix token, API URL, WS URL) — just performed inside the custom protocol handler instead of the TCP proxy.
- **Connector internals**: KubeStudio's Axum router, Dioxus liveview, health/info endpoints all remain identical.
- **Configuration format**: `~/.config/strikehub/connectors.toml` retains the same schema, extended with a `transport` field.

---

## 2. Architecture

### 2.1 Component Diagram

```
StrikeHub Process
├── IpcConnectorRunner          (spawns child process, manages lifecycle)
├── ConnectorBridge             (wry custom protocol: connector://{id}/*)
├── WsRelay                     (single TCP listener, relays WS to Unix sockets)
├── AuthManager                 (unchanged — OIDC flow, token refresh)
├── ConnectorProxy              (retained for TCP-mode connectors during migration)
└── Dioxus Desktop UI
    ├── Sidebar
    ├── Content Panel            (iframe src: connector://kubestudio/liveview)
    ├── Status Bar
    └── Settings

KubeStudio Child Process
├── ks-connector binary
├── Axum server bound to UnixListener
│   ├── /liveview               (Dioxus liveview endpoint)
│   ├── /health                 (health check)
│   ├── /connector/info         (name, icon metadata)
│   └── /assets/*               (CSS, JS, WASM)
└── WebSocket upgrade on /ws    (Dioxus hot-reload & liveview WS)
```

### 2.2 Request Flow (IPC Mode)

```
Dioxus webview
    │
    ├─ HTTP (custom protocol) ──────────────────────────────────────────┐
    │                                                                    │
    │  iframe src="connector://kubestudio/liveview"                      │
    │      ↓                                                             │
    │  ConnectorBridge (wry custom protocol handler)                     │
    │      ↓                                                             │
    │  Unix socket: /tmp/strikehub-kubestudio.sock                       │
    │      ↓                                                             │
    │  GET /liveview → KubeStudio Axum server                            │
    │      ↓                                                             │
    │  HTML response (rewritten: auth token, API URL, WS URL injected)   │
    │      ↓                                                             │
    │  Rendered in iframe                                                 │
    │                                                                    │
    ├─ WebSocket ───────────────────────────────────────────────────────┐
    │                                                                    │
    │  new WebSocket("ws://127.0.0.1:{bridge_port}/ws/kubestudio")       │
    │      ↓                                                             │
    │  WsRelay (single TCP listener on ephemeral port)                   │
    │      ↓                                                             │
    │  Unix socket: /tmp/strikehub-kubestudio.sock                       │
    │      ↓                                                             │
    │  WebSocket upgrade → KubeStudio Dioxus liveview WS                 │
    │                                                                    │
    └─ Matrix GraphQL WS ──────────────────────────────────────────────┐
                                                                         │
       new WebSocket("ws://127.0.0.1:{bridge_port}/ws/graphql?token=…")  │
           ↓                                                             │
       WsRelay                                                           │
           ↓                                                             │
       wss://{matrix_host}/v1alpha/graphql_socket/websocket              │
```

### 2.3 Transport: Unix Domain Sockets

Connectors bind an `axum::serve(unix_listener, router)` at a well-known temp path:

```
/tmp/strikehub-{connector_id}.sock
```

**Why Unix sockets over raw `socketpair()`**: A `socketpair` provides a single bidirectional byte stream between two file descriptors — it supports exactly one connection. Axum's `serve()` expects a listener that can `accept()` multiple concurrent connections. A `UnixListener` provides this naturally and integrates with Axum's existing `tokio::net::UnixListener` support. Multiple concurrent HTTP requests (liveview page + CSS + JS + WASM + health checks) can be served simultaneously without multiplexing.

**Why Unix sockets over TCP**: No port allocation, no port conflicts, no exposure to the network. The socket file is created by the connector and cleaned up on exit. StrikeHub connects as a client.

**Socket lifecycle**:
1. StrikeHub spawns the connector child process with `STRIKEHUB_SOCKET=/tmp/strikehub-{id}.sock`.
2. The connector removes any stale socket file, binds a `UnixListener`, and starts `axum::serve`.
3. StrikeHub polls the socket path until it exists, then issues a health check.
4. On connector exit, StrikeHub removes the socket file if it remains.

---

## 3. Content Delivery: Wry Custom Protocol

### 3.1 Why a Custom Protocol

Wry supports registering custom URI scheme handlers via `WebViewBuilder::with_custom_protocol()`. A handler registered for the scheme `connector` intercepts all requests matching `connector://*` and returns responses synchronously. This eliminates the need for any TCP port to serve connector HTTP content (HTML, CSS, JS, WASM, health, info endpoints).

The Dioxus webview already runs on wry. Registering a custom protocol requires access to the `WebViewBuilder` during window creation, which Dioxus desktop exposes via `with_cfg()`.

### 3.2 ConnectorBridge Handler

```
connector://{connector_id}/{path}
```

For each incoming request:

1. **Route to the correct Unix socket** based on `{connector_id}`.
2. **Forward the HTTP request** (method, path, headers, body) over the Unix socket to the connector's Axum server.
3. **Rewrite the response** if it is the liveview HTML page (same `rewrite_html()` logic as today):
   - Inject `window.__MATRIX_AUTH_TOKEN__`
   - Inject `window.__MATRIX_API_URL__`
   - Inject `window.__MATRIX_WS_URL__` pointing to the WS bridge
   - Rewrite Dioxus `__dioxusGetWsUrl()` to point to the WS bridge
   - Inject height-filling CSS
4. **Return the response** to wry with correct `Content-Type`.

### 3.3 Iframe Source

The content panel's iframe `src` changes from:

```
http://127.0.0.1:{proxy_port}/c/{connector_port}/liveview
```

to:

```
connector://kubestudio/liveview
```

This is the only change in the content rendering component. The iframe still loads a full HTML page with all assets resolved relative to the custom protocol origin.

---

## 4. WebSocket Bridge: WsRelay

### 4.1 Why a Bridge is Needed

Wry custom protocol handlers intercept HTTP requests but **cannot intercept `new WebSocket()` calls**. The browser's WebSocket constructor requires a `ws://` or `wss://` URL and bypasses custom protocol handlers entirely. Two types of WebSocket connections need bridging:

1. **Dioxus liveview WS**: The connector's live UI updates (DOM patches, event handling).
2. **Matrix GraphQL subscriptions**: Real-time data from the Matrix server.

### 4.2 Design

`WsRelay` binds a single `TcpListener` on `127.0.0.1:0` (ephemeral port). This is the **only TCP port** opened by StrikeHub in IPC mode, regardless of how many connectors are active.

**Routing by path prefix**:

| Path | Upstream |
|------|----------|
| `/ws/{connector_id}` | Unix socket `/tmp/strikehub-{id}.sock`, WebSocket upgrade on `/ws` |
| `/ws/{connector_id}/{path}` | Unix socket, WebSocket upgrade on `/ws/{path}` |
| `/ws/graphql` | `wss://{matrix_host}/v1alpha/graphql_socket/websocket` (same as today) |

**Frame relay**: Bidirectional forwarding of WebSocket frames between the client (webview) and the upstream (connector or Matrix). Same approach as the existing `handle_graphql_ws` in `proxy.rs`.

### 4.3 Port Reduction

| Mode | Open TCP ports |
|------|---------------|
| TCP (current, N connectors) | N connector ports + 1 proxy port = N+1 |
| IPC (new, N connectors) | 1 WS bridge port |

---

## 5. IpcConnectorRunner

### 5.1 Responsibilities

1. **Spawn** the connector binary as a child process via `tokio::process::Command`.
2. **Set environment**: `STRIKEHUB_SOCKET=/tmp/strikehub-{id}.sock` plus any connector-specific env vars (e.g., `KUBECONFIG`).
3. **Wait for readiness**: Poll the socket path until it exists, then `GET /health` over the Unix socket.
4. **Monitor**: Periodic health checks over the Unix socket (same 3-second interval).
5. **Restart** (optional): If the child exits unexpectedly, restart with backoff.
6. **Shutdown**: Send `SIGTERM` to the child on StrikeHub exit or connector disable. Clean up the socket file.

### 5.2 Struct Sketch

```rust
pub struct IpcConnectorRunner {
    id: String,
    child: tokio::process::Child,
    socket_path: PathBuf,
}

impl IpcConnectorRunner {
    pub async fn start(id: &str, binary: &Path, env: &[(String, String)]) -> Result<Self, HubError>;
    pub fn socket_path(&self) -> &Path;
    pub async fn health_check(&self) -> bool;
    pub async fn stop(&mut self) -> Result<(), HubError>;
}

impl Drop for IpcConnectorRunner {
    fn drop(&mut self) {
        // SIGTERM child, remove socket file
    }
}
```

### 5.3 Coexistence with TCP ConnectorRunner

During migration, both runner types exist. The active runner is selected per-connector based on configuration:

```rust
pub enum ConnectorTransport {
    /// Legacy: connector runs in-process, serves on TCP port.
    Tcp { port: u16 },
    /// New: connector runs as child process, communicates over Unix socket.
    Ipc { binary: PathBuf },
}
```

The `ConnectorConfig` in `config.rs` gains an optional `transport` field. Connectors without an explicit transport default to `Tcp` for backward compatibility.

---

## 6. KubeStudio Changes

### 6.1 Scope

Minimal. The only change is in `ks-connector`'s `start_dioxus_server()` function (or equivalent server startup path).

### 6.2 Behavior

```rust
// In ks-connector's server startup:

let listener = if let Ok(sock_path) = std::env::var("STRIKEHUB_SOCKET") {
    // IPC mode: bind Unix socket at the path StrikeHub specified
    let path = PathBuf::from(&sock_path);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let unix_listener = tokio::net::UnixListener::bind(&path)?;
    ServerListener::Unix(unix_listener)
} else {
    // Standalone mode: bind TCP as before
    let tcp_listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    ServerListener::Tcp(tcp_listener)
};

axum::serve(listener, router).await?;
```

Everything else — the Axum router, Dioxus liveview setup, health endpoint, info endpoint, static assets — remains identical. The connector does not need to know about custom protocols, WS bridges, or auth injection. It just serves HTTP on whatever listener it was given.

### 6.3 Standalone Operation

KubeStudio continues to work as a standalone desktop or liveview app when `STRIKEHUB_SOCKET` is not set. No behavior change for developers running KubeStudio outside of StrikeHub.

---

## 7. StrikeHub Changes

### 7.1 New Components

| Component | Location | Purpose |
|-----------|----------|---------|
| `IpcConnectorRunner` | `sh-core/src/ipc_runner.rs` | Spawns and manages connector child processes |
| `ConnectorBridge` | `sh-core/src/bridge.rs` | Wry custom protocol handler (`connector://*`) |
| `WsRelay` | `sh-core/src/ws_relay.rs` | Single-port WebSocket bridge to Unix sockets |
| `ConnectorTransport` | `sh-core/src/config.rs` | Enum: `Tcp { port }` or `Ipc { binary }` |

### 7.2 Modified Components

| Component | Change |
|-----------|--------|
| `config.rs` | Add `transport` field to `ConnectorEntry` |
| `app.rs` | Select runner type based on transport config; register custom protocol; start WsRelay |
| `content.rs` | Use `connector://{id}/liveview` URL for IPC connectors |
| `main.rs` | Register wry custom protocol via Dioxus desktop config |
| `proxy.rs` | Retained for TCP-mode connectors; no changes |

### 7.3 Removed Dependencies (Final Phase)

| Dependency | Reason |
|------------|--------|
| `ks_ui` (ks-ui crate) | No longer compiled into StrikeHub |
| `kubestudio` cargo feature | No longer needed |
| `dioxus-liveview` (from sh-core) | Only connectors need liveview; StrikeHub is just a host |

---

## 8. Configuration

### 8.1 Updated `connectors.toml`

```toml
[connectors.kubestudio]
display_name = "KubeStudio"
binary = "~/code/strike48/studio-kube-desktop/target/release/ks-connector"
icon = "kubernetes"
auto_start = true
transport = "ipc"   # "ipc" or "tcp"
port = 3030         # used only when transport = "tcp"

[connectors.jirastudio]
display_name = "JiraStudio"
binary = "~/code/strike48/scratch/jirastudio/target/release/js-connector"
port = 3031
icon = "jira"
auto_start = false
transport = "tcp"   # default during migration
```

### 8.2 Environment Variables Passed to Child Processes

| Variable | Value | Purpose |
|----------|-------|---------|
| `STRIKEHUB_SOCKET` | `/tmp/strikehub-{id}.sock` | Tells connector to bind Unix socket |
| `KUBECONFIG` | Inherited or overridden | Connector-specific config |
| `STRIKE48_API_URL` | From StrikeHub's auth config | If connector needs direct API access |

---

## 9. Migration Plan

Both TCP and IPC modes coexist throughout migration. No big-bang cutover.

### Phase 1: KubeStudio Adds Unix Socket Support

**Scope**: `studio-kube-desktop` repo only.

- Add `STRIKEHUB_SOCKET` env var check to `ks-connector` server startup.
- When set, bind `UnixListener` instead of `TcpListener`.
- When unset, behavior unchanged (standalone TCP mode).
- Test: Run `ks-connector` with `STRIKEHUB_SOCKET=/tmp/test.sock`, verify HTTP over Unix socket works with `curl --unix-socket`.

### Phase 2: StrikeHub Gains IPC Runner and Bridge

**Scope**: `strikehub` repo.

- Implement `IpcConnectorRunner` (spawn child, manage socket lifecycle).
- Implement `ConnectorBridge` (wry custom protocol handler).
- Implement `WsRelay` (WebSocket bridge).
- Add `ConnectorTransport` enum to config.
- Register `connector://` custom protocol in Dioxus desktop config.
- Wire up content panel to use `connector://{id}/liveview` for IPC connectors.
- Test: Configure KubeStudio with `transport = "ipc"` in `connectors.toml`, verify full functionality.

### Phase 3: Remove Library Dependency

**Scope**: `strikehub` repo.

- Remove `ks_ui` workspace dependency from `Cargo.toml`.
- Remove `kubestudio` feature flag from `sh-core`.
- Remove `ConnectorRunner::start_kubestudio()` and in-process runner code.
- Default new connectors to `transport = "ipc"`.
- TCP mode retained for any connector that hasn't adopted Unix socket support yet.

### Phase 4: Other Connectors

**Scope**: Each connector repo.

- Add the same `STRIKEHUB_SOCKET` env var check to each connector's server startup.
- Pattern is identical to KubeStudio — check env, bind Unix or TCP accordingly.
- StrikeHub config updated to `transport = "ipc"` per connector as they're ready.

---

## 10. Security Considerations

- **Socket file permissions**: The Unix socket is created by the child process with default permissions (user-only on most systems). StrikeHub and the connector run as the same user.
- **No network exposure**: Unix sockets are not accessible over the network. The only TCP port (WsRelay) binds to `127.0.0.1` and is not exposed externally.
- **Auth token handling**: Auth tokens are injected into HTML by the `ConnectorBridge`, same as the current TCP proxy. Tokens are never written to disk or passed as command-line arguments.
- **Socket cleanup**: Stale socket files are removed before bind and after child exit. A crash handler (`Drop` impl) ensures cleanup on unexpected shutdown.
- **Child process isolation**: Connector crashes do not affect StrikeHub. The runner detects child exit and updates status to offline.

---

## 11. Open Questions

1. **Socket path convention**: `/tmp/strikehub-{id}.sock` is simple but `/tmp` is world-readable on some systems. Consider `$XDG_RUNTIME_DIR/strikehub/{id}.sock` for better isolation, falling back to `/tmp` when `XDG_RUNTIME_DIR` is not set.

2. **Auto-restart policy**: Should `IpcConnectorRunner` automatically restart a crashed connector? If so, what backoff strategy (linear, exponential, max retries)?

3. **Stdio forwarding**: Should connector stdout/stderr be captured and displayed in StrikeHub's UI (e.g., a log panel), or just forwarded to StrikeHub's own stdout?

4. **Multiple instances**: Can two StrikeHub instances run simultaneously? Socket paths would conflict. Consider including a session ID in the path.
