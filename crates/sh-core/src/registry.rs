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
    /// GitHub repo in `owner/repo` format for fetching pre-built binaries.
    pub github_repo: Option<&'static str>,
    /// Asset filename pattern with `{os}`, `{arch}`, `{ext}` placeholders.
    pub asset_pattern: Option<&'static str>,
}

impl ConnectorManifest {
    /// Resolve the asset filename for the current platform using the pattern.
    pub fn asset_name(&self) -> Option<String> {
        let pattern = self.asset_pattern?;
        Some(
            pattern
                .replace("{os}", platform_os())
                .replace("{arch}", platform_arch())
                .replace("{ext}", platform_archive_ext()),
        )
    }
}

/// Returns the OS identifier used in release asset names.
pub fn platform_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    }
}

/// Returns the architecture identifier used in release asset names.
pub fn platform_arch() -> &'static str {
    std::env::consts::ARCH
}

/// Returns the archive extension for the current platform.
pub fn platform_archive_ext() -> &'static str {
    if cfg!(target_os = "windows") {
        "zip"
    } else {
        "tar.gz"
    }
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
            github_repo: Some("Strike48-public/kubestudio"),
            asset_pattern: Some("ks-connector-{os}-{arch}.{ext}"),
        },
        ConnectorManifest {
            id: "pick",
            name: "Pick",
            description: "Penetration testing toolkit",
            icon: "hero-shield-exclamation",
            default_port: 3030,
            default_transport: ConnectorTransport::Ipc,
            binary_hint: Some("pentest-agent"),
            github_repo: Some("Strike48-public/pick"),
            asset_pattern: Some("pentest-agent-{os}-{arch}.{ext}"),
        },
    ]
}
