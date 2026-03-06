use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::HubError;
use crate::registry::ConnectorManifest;

/// Transport used to communicate with a connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConnectorTransport {
    /// Legacy: connector runs in-process, serves on a TCP port.
    #[default]
    Tcp,
    /// New: connector runs as a child process, communicates over a Unix socket.
    Ipc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubConfig {
    #[serde(default)]
    pub setup_complete: bool,
    #[serde(default)]
    pub pick_tos_accepted: bool,
    #[serde(default)]
    pub connectors: BTreeMap<String, ConnectorEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorEntry {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub binary: Option<String>,
    #[serde(default)]
    pub port: u16,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub transport: ConnectorTransport,
    /// Explicit socket path for custom IPC connectors (externally managed).
    #[serde(default)]
    pub socket_path: Option<String>,
}

fn default_enabled() -> bool {
    true
}

fn default_icon() -> String {
    "app".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorStatus {
    Online,
    Offline,
    Checking,
}

#[derive(Debug, Clone)]
pub struct ConnectorConfig {
    pub id: String,
    pub display_name: String,
    pub binary: Option<String>,
    pub port: u16,
    pub icon: String,
    pub auto_start: bool,
    pub status: ConnectorStatus,
    pub transport: ConnectorTransport,
    /// Explicit socket path for externally-managed IPC connectors.
    pub explicit_socket: Option<String>,
    /// Matrix app address for routing content through `/app-content/{address}/`.
    /// Discovered via the `connectorApps` GraphQL query after the connector
    /// registers with Matrix.
    pub matrix_app_address: Option<String>,
}

impl ConnectorConfig {
    /// Create a config for an externally-managed IPC connector at a given socket path.
    pub fn from_socket(name: String, socket_path: String) -> Self {
        // Derive a stable id from the socket path.
        let id = format!("ipc-{}", slug_from_path(&socket_path));
        Self {
            id,
            display_name: name,
            binary: None,
            port: 0,
            icon: "app".to_string(),
            auto_start: false,
            status: ConnectorStatus::Offline,
            transport: ConnectorTransport::Ipc,
            explicit_socket: Some(socket_path),
            matrix_app_address: None,
        }
    }

    /// The IPC address for this connector.
    ///
    /// Custom IPC connectors use an explicit path; builtin connectors use
    /// a well-known platform-specific address.
    pub fn ipc_addr(&self) -> crate::ipc::IpcAddr {
        if let Some(ref p) = self.explicit_socket {
            crate::ipc::IpcAddr::from_string(p)
        } else {
            crate::ipc::IpcAddr::for_connector(&self.id)
        }
    }

    /// Backward-compat: return the IPC address as a `PathBuf`.
    pub fn socket_path(&self) -> PathBuf {
        self.ipc_addr().to_path_buf()
    }

    /// The URL to load in the webview (liveview page).
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/liveview", self.port)
    }

    /// The URL to load in the content area, choosing the right scheme per transport.
    ///
    /// - **TCP**: `http://127.0.0.1:{proxy_port}/c/{port}/liveview`
    /// - **IPC**: `connector://{id}/liveview`
    pub fn content_url(&self, proxy_port: Option<u16>, _ws_bridge_port: Option<u16>) -> String {
        match self.transport {
            ConnectorTransport::Ipc => {
                format!("connector://{}/liveview", self.id)
            }
            ConnectorTransport::Tcp => {
                if let Some(pp) = proxy_port {
                    format!("http://127.0.0.1:{}/c/{}/liveview", pp, self.port)
                } else {
                    self.url()
                }
            }
        }
    }

    /// The URL to load via the auth proxy.
    pub fn proxy_url(&self, proxy_port: u16) -> String {
        format!("http://127.0.0.1:{}/c/{}/liveview", proxy_port, self.port)
    }

    /// The URL to probe for health checks.
    pub fn health_url(&self) -> String {
        format!("http://127.0.0.1:{}/health", self.port)
    }
}

impl HubConfig {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("strikehub")
            .join("connectors.toml")
    }

    pub fn load() -> Result<Self, HubError> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(Self {
                setup_complete: false,
                pick_tos_accepted: false,
                connectors: BTreeMap::new(),
            });
        }
        let contents = std::fs::read_to_string(&path)?;
        let config: HubConfig = toml::from_str(&contents)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<(), HubError> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self).map_err(|e| HubError::Config(e.to_string()))?;
        std::fs::write(&path, contents)?;
        Ok(())
    }

    /// Apply manifest defaults to saved connector entries.
    ///
    /// Builtin connectors may have been saved with an older transport or
    /// without a binary path. This merges the manifest values so that
    /// upgrading the code automatically picks up new defaults.
    pub fn apply_manifest_defaults(&mut self, manifests: &[crate::registry::ConnectorManifest]) {
        for m in manifests {
            if let Some(entry) = self.connectors.get_mut(m.id) {
                entry.transport = m.default_transport;
                if entry.binary.is_none() {
                    entry.binary = m.binary_hint.map(|s| s.to_string());
                }
            }
        }
    }

    pub fn to_connectors(&self) -> Vec<ConnectorConfig> {
        self.connectors
            .iter()
            .filter(|(_, entry)| entry.enabled)
            .map(|(id, entry)| {
                let display_name = entry.display_name.clone().unwrap_or_else(|| id.clone());
                ConnectorConfig {
                    id: id.clone(),
                    display_name,
                    binary: entry.binary.clone(),
                    port: entry.port,
                    icon: entry.icon.clone(),
                    auto_start: entry.auto_start,
                    status: ConnectorStatus::Offline,
                    transport: entry.transport,
                    explicit_socket: entry.socket_path.clone(),
                    matrix_app_address: None,
                }
            })
            .collect()
    }

    /// Create a `ConnectorEntry` from a manifest and insert it into the config.
    pub fn enable_from_manifest(&mut self, manifest: &ConnectorManifest) {
        self.connectors.insert(
            manifest.id.to_string(),
            ConnectorEntry {
                display_name: Some(manifest.name.to_string()),
                binary: manifest.binary_hint.map(|s| s.to_string()),
                port: manifest.default_port,
                icon: manifest.icon.to_string(),
                auto_start: true,
                enabled: true,
                transport: manifest.default_transport,
                socket_path: None,
            },
        );
    }

    /// Add a custom IPC connector by socket path.
    pub fn add_socket(&mut self, name: String, socket_path: String) {
        let id = format!("ipc-{}", slug_from_path(&socket_path));
        self.connectors.insert(
            id,
            ConnectorEntry {
                display_name: Some(name),
                binary: None,
                port: 0,
                icon: "app".to_string(),
                auto_start: false,
                enabled: true,
                transport: ConnectorTransport::Ipc,
                socket_path: Some(socket_path),
            },
        );
    }

    pub fn remove(&mut self, id: &str) {
        self.connectors.remove(id);
    }
}

/// Derive a filesystem-safe slug from a socket path for use as a connector id.
pub fn slug_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("custom")
        .to_string()
}
