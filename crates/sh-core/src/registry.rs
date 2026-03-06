use crate::config::ConnectorTransport;

/// Static manifest for a builtin connector.
///
/// Each connector crate pulled into the workspace gets one entry here.
/// Adding a new connector = one new entry in `builtin_manifests()`.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorManifest {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
    pub default_port: u16,
    /// Default transport for this connector. Connectors that support IPC can
    /// set this to `Ipc`; others default to `Tcp`.
    pub default_transport: ConnectorTransport,
    /// Optional hint for the binary path (used to pre-populate IPC config).
    pub binary_hint: Option<&'static str>,
}

/// All connectors compiled into this binary.
pub fn builtin_manifests() -> Vec<ConnectorManifest> {
    vec![
        ConnectorManifest {
            id: "kubestudio",
            name: "KubeStudio",
            description: "Kubernetes cluster management dashboard",
            icon: "hero-server-stack",
            default_port: 3030,
            default_transport: ConnectorTransport::Ipc,
            binary_hint: Some("ks-connector"),
        },
        ConnectorManifest {
            id: "pick",
            name: "Pick",
            description: "Penetration testing toolkit",
            icon: "hero-shield-exclamation",
            default_port: 3030,
            default_transport: ConnectorTransport::Ipc,
            binary_hint: Some("pentest-agent"),
        },
    ]
}
