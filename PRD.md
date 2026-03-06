# StrikeHub - Product Requirements Document

## 1. Overview

StrikeHub is a Dioxus 0.6 desktop application that acts as a unified host shell for Strike48 connector apps. It discovers running connector processes, communicates with them over IPC, and renders each as an embeddable panel inside a single native window.

### Connector Apps

| App | Path | Purpose |
|-----|------|---------|
| KubeStudio | `~/code/strike48/studio-kube-desktop` | Kubernetes cluster browser |
| JiraStudio | `~/code/strike48/scratch/jirastudio` | Jira Cloud client |
| GitLabStudio | `~/code/strike48/scratch/gitlabstudio` | GitLab client (projects, MRs, pipelines) |
| StrikeOffice | `~/code/strike48/scratch/strikeoffice` | Google Workspace manager |
| StrikeTeam | `~/code/strike48/scratch/striketeam` | Isometric AI agent RPG |

All connectors already use **Dioxus 0.6**, **Axum 0.7**, and the **strike48-connector SDK** (gRPC + protobuf). Each can run a liveview server on `localhost:303X` and register with a broker via `RegisterConnectorRequest`.

---

## 2. Problem Statement

Today each connector app runs as a standalone desktop or liveview binary. There is no unified way to:

- See which connectors are running at a glance
- Switch between connectors without alt-tabbing between windows
- Centralize credential and lifecycle management

StrikeHub solves this by providing a single desktop shell that orchestrates all connectors.

---

## 3. Architecture

### 3.1 Integration Model: IPC + Side Panel

```
┌──────────────────────────────────────────────────────┐
│  StrikeHub (Dioxus Desktop)                          │
│                                                      │
│  ┌──────────┐  ┌──────────────────────────────────┐  │
│  │ Sidebar  │  │  Content Area                    │  │
│  │          │  │                                  │  │
│  │ ● Kube   │  │  ┌──────────────────────────┐   │  │
│  │ ● Jira   │  │  │ Embedded WebView         │   │  │
│  │ ○ GitLab │  │  │ (liveview of active app)  │   │  │
│  │ ○ Office │  │  │                          │   │  │
│  │ ○ Team   │  │  └──────────────────────────┘   │  │
│  │          │  │                                  │  │
│  │ [+] Add  │  │                                  │  │
│  └──────────┘  └──────────────────────────────────┘  │
│                                                      │
│  ┌──────────────────────────────────────────────────┐│
│  │ Status Bar: 3/5 connectors online  │ agent chat  ││
│  └──────────────────────────────────────────────────┘│
└──────────────────────────────────────────────────────┘
```

Each connector runs as a **separate process** with its own liveview server. StrikeHub embeds the connector UI via a webview pointed at the connector's localhost port. Communication for orchestration (health checks, lifecycle) happens over **gRPC** using the existing `strike48-connector` protocol. Each connector continues to manage its own tool registration with Matrix independently.

### 3.2 Component Diagram

```
StrikeHub Process
├── Connector Registry (discovers & tracks running connectors)
├── Process Manager (optional: start/stop connector binaries)
├── Health Monitor (periodic gRPC health checks)
├── WebView Manager (one webview per active connector)
├── Credential Vault (centralized token storage)
└── Dioxus Desktop UI
    ├── Sidebar (connector list + status indicators)
    ├── Content Panel (active connector webview)
    ├── Status Bar (aggregate health)
    └── Settings / Preferences
```

### 3.3 IPC Protocol

Leverage the existing `strike48-connector` gRPC services:

| RPC | Direction | Purpose |
|-----|-----------|---------|
| `RegisterConnector` | Connector → Hub | Connector announces itself on startup |
| `HealthCheck` | Hub → Connector | Periodic liveness probe |
| `ProxyWebSocket` | Hub ↔ Connector | Tunnel liveview WS frames |

StrikeHub runs a lightweight gRPC server that connectors register with on startup (replacing or supplementing the Matrix broker). It also acts as a gRPC client to query each connector.

### 3.4 Connector Discovery

Three mechanisms, in priority order:

1. **Active registration**: Connectors call `RegisterConnector` on StrikeHub's gRPC endpoint at startup.
2. **Config file**: `~/.config/strikehub/connectors.toml` lists known connectors with binary paths and ports.
3. **Port scanning** (fallback): Probe `localhost:3030-3039` for known connector fingerprints.

---

## 4. Functional Requirements

### 4.1 Connector Management

| ID | Requirement | Priority |
|----|-------------|----------|
| CM-1 | Display a sidebar listing all known connectors with online/offline status | P0 |
| CM-2 | Click a connector in the sidebar to load its liveview UI in the content panel | P0 |
| CM-3 | Support multiple connectors visible simultaneously via tabs or split view | P1 |
| CM-4 | Start/stop connector processes from within StrikeHub | P1 |
| CM-5 | Auto-discover connectors via gRPC registration | P0 |
| CM-6 | Persist connector configuration in `~/.config/strikehub/connectors.toml` | P1 |
| CM-7 | Show connector health (green/yellow/red) based on periodic health checks | P0 |

### 4.2 UI Shell

| ID | Requirement | Priority |
|----|-------------|----------|
| UI-1 | Dioxus 0.6 desktop app with native window chrome | P0 |
| UI-2 | Collapsible sidebar with connector icons and labels | P0 |
| UI-3 | Content area renders the selected connector's liveview via embedded webview | P0 |
| UI-4 | Status bar showing aggregate connector count and health | P1 |
| UI-5 | Global keyboard shortcuts to switch between connectors (Cmd+1..5) | P1 |
| UI-6 | Light/dark theme following system preference | P2 |
| UI-7 | Splash/empty state when no connectors are running | P0 |

### 4.3 Credential Management

| ID | Requirement | Priority |
|----|-------------|----------|
| CR-1 | Optionally store and inject credentials for connectors | P2 |
| CR-2 | Support per-connector environment variable overrides | P1 |
| CR-3 | Respect existing per-app token stores (no forced migration) | P0 |

---

## 5. Non-Functional Requirements

| ID | Requirement |
|----|-------------|
| NF-1 | Startup to usable shell in under 2 seconds (excluding connector boot) |
| NF-2 | Memory overhead of StrikeHub itself under 100MB (webviews excluded) |
| NF-3 | gRPC health checks at 5-second intervals with 2-second timeout |
| NF-4 | Graceful degradation: if a connector crashes, StrikeHub shows error state, doesn't crash |
| NF-5 | Cross-platform: macOS primary, Linux secondary |

---

## 6. Tech Stack

| Layer | Technology |
|-------|------------|
| Language | Rust (edition 2024) |
| UI Framework | Dioxus 0.6 (desktop feature) |
| IPC | gRPC via `tonic` + `prost` (aligns with strike48-connector) |
| Embedded UI | Dioxus desktop webview (each connector rendered at its localhost URL) |
| Config | TOML (`~/.config/strikehub/connectors.toml`) |
| Build | Cargo workspace |
| Async Runtime | Tokio |

---

## 7. Crate Structure

```
strikehub/
├── Cargo.toml              (workspace root)
├── PRD.md
├── sh-core/                (types, errors, config models)
│   ├── Cargo.toml
│   └── src/lib.rs
├── sh-ipc/                 (gRPC server + client for connector communication)
│   ├── Cargo.toml
│   ├── proto/
│   │   └── strikehub.proto
│   └── src/lib.rs
├── sh-connector/           (connector registry, health monitor, process manager)
│   ├── Cargo.toml
│   └── src/lib.rs
└── sh-ui/                  (Dioxus desktop UI)
    ├── Cargo.toml
    ├── assets/
    └── src/
        ├── main.rs
        ├── sidebar.rs
        ├── content.rs
        ├── status_bar.rs
        └── settings.rs
```

---

## 8. Connector Configuration

### `~/.config/strikehub/connectors.toml`

```toml
[connectors.kubestudio]
display_name = "KubeStudio"
binary = "~/code/strike48/studio-kube-desktop/target/release/ks-ui"
port = 3030
icon = "kubernetes"
auto_start = false

[connectors.jirastudio]
display_name = "JiraStudio"
binary = "~/code/strike48/scratch/jirastudio/target/release/js-ui"
port = 3031
icon = "jira"
auto_start = false

[connectors.gitlabstudio]
display_name = "GitLabStudio"
binary = "~/code/strike48/scratch/gitlabstudio/target/release/gl-ui"
port = 3032
icon = "gitlab"
auto_start = false

[connectors.strikeoffice]
display_name = "StrikeOffice"
binary = "~/code/strike48/scratch/strikeoffice/target/release/so-ui"
port = 3033
icon = "google"
auto_start = false

[connectors.striketeam]
display_name = "StrikeTeam"
binary = "~/code/strike48/scratch/striketeam/target/release/st-ui"
port = 3034
icon = "gamepad"
auto_start = false
```

---

## 9. User Flows

### 9.1 First Launch

1. StrikeHub opens with an empty sidebar and a welcome splash screen.
2. User clicks **[+] Add Connector** or edits `connectors.toml`.
3. StrikeHub attempts to connect to the configured port.
4. If the connector is running, it appears as online; otherwise, offline with a "Start" button (if binary path is configured).

### 9.2 Switching Connectors

1. User clicks a connector in the sidebar (or presses Cmd+N).
2. The content panel loads/switches the webview to that connector's liveview URL.
3. Previous connector's webview is suspended (not destroyed) so state is preserved.

### 9.3 Connector Goes Down

1. Health monitor detects missed heartbeat.
2. Sidebar indicator turns red.
3. Content panel shows "Connector offline - Restart?" overlay.
4. User can click Restart or wait for auto-recovery.

---

## 10. Milestones

### M1 - Shell + Static Sidebar (MVP)
- Dioxus desktop window with hardcoded connector list
- Embed a single connector liveview via webview
- Click to switch between connectors

### M2 - IPC + Discovery
- gRPC server for connector registration
- Health monitoring with status indicators
- Dynamic sidebar updates

### M3 - Process Management
- Start/stop connectors from UI
- Read `connectors.toml` for binary paths
- Auto-start flagged connectors on launch

### M4 - Polish
- Keyboard shortcuts
- Credential management
- Theming
- Multi-view (split/tabs)

---

## 11. Open Questions

1. **Should StrikeHub replace the Matrix broker entirely for local dev, or run alongside it?** Current connectors register with a Matrix gateway -- StrikeHub could act as a local broker alternative.

2. **WebView vs re-rendering**: Embedding liveview via webview is the simplest path, but should StrikeHub eventually import connector UI crates directly as Dioxus components for a tighter integration?

3. **Tab vs split-pane for multiple connectors**: Tabs are simpler; split panes let you see two connectors side by side (e.g., Jira + GitLab).
