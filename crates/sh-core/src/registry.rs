use std::borrow::Cow;

use crate::allowlist::get_allowlist;
use crate::config::{ConnectorTransport, HubConfig};

/// Manifest for a connector (builtin or dynamic).
///
/// Each connector crate pulled into the workspace gets one entry here.
/// Adding a new connector = one new entry in `builtin_manifests()`.
/// Dynamic connectors are loaded from config at runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorManifest {
    pub id: Cow<'static, str>,
    pub name: Cow<'static, str>,
    pub description: Cow<'static, str>,
    pub icon: Cow<'static, str>,
    pub default_port: u16,
    /// Default transport for this connector. Connectors that support IPC can
    /// set this to `Ipc`; others default to `Tcp`.
    pub default_transport: ConnectorTransport,
    /// Optional hint for the binary path (used to pre-populate IPC config).
    pub binary_hint: Option<Cow<'static, str>>,
    /// GitHub repo in `owner/repo` format for fetching pre-built binaries.
    pub github_repo: Option<Cow<'static, str>>,
    /// Asset filename pattern with `{os}`, `{arch}`, `{ext}` placeholders.
    pub asset_pattern: Option<Cow<'static, str>>,
    /// Whether this manifest was compiled into the binary (true) or loaded
    /// from config at runtime (false).
    pub is_builtin: bool,
}

impl ConnectorManifest {
    /// Resolve the asset filename for the current platform using the pattern.
    pub fn asset_name(&self) -> Option<String> {
        let pattern = self.asset_pattern.as_deref()?;
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
            id: Cow::Borrowed("kubestudio"),
            name: Cow::Borrowed("KubeStudio"),
            description: Cow::Borrowed("Kubernetes cluster management dashboard"),
            icon: Cow::Borrowed("hero-server-stack"),
            default_port: 3030,
            default_transport: ConnectorTransport::Ipc,
            binary_hint: Some(Cow::Borrowed("ks-connector")),
            github_repo: Some(Cow::Borrowed("Strike48-public/kubestudio")),
            asset_pattern: Some(Cow::Borrowed("ks-connector-{os}-{arch}.{ext}")),
            is_builtin: true,
        },
        ConnectorManifest {
            id: Cow::Borrowed("pick"),
            name: Cow::Borrowed("Pick"),
            description: Cow::Borrowed("Penetration testing toolkit"),
            icon: Cow::Borrowed("hero-shield-exclamation"),
            default_port: 3030,
            default_transport: ConnectorTransport::Ipc,
            binary_hint: Some(Cow::Borrowed("pentest-agent")),
            github_repo: Some(Cow::Borrowed("Strike48-public/pick")),
            asset_pattern: Some(Cow::Borrowed("pentest-agent-{os}-{arch}.{ext}")),
            is_builtin: true,
        },
    ]
}

/// Merge builtin manifests with dynamic connectors from config, filtering
/// by the allowlist.
///
/// - Builtins always pass the allowlist (they are compiled in).
/// - Dynamic connectors are checked against the global allowlist; those
///   with a non-allowed `github_repo` are excluded with a warning log.
/// - If a dynamic connector ID collides with a builtin, the builtin wins.
pub fn all_manifests(config: &HubConfig) -> Vec<ConnectorManifest> {
    merge_manifests(config, get_allowlist())
}

/// Inner merge logic, takes an explicit allowlist for testability.
pub fn merge_manifests(
    config: &HubConfig,
    allowlist: &crate::allowlist::RepoAllowlist,
) -> Vec<ConnectorManifest> {
    let builtins = builtin_manifests();
    let builtin_ids: std::collections::HashSet<String> =
        builtins.iter().map(|m| m.id.to_string()).collect();

    let mut result = builtins;

    for def in &config.dynamic_connectors {
        // ID collision: builtin wins
        if builtin_ids.contains(&def.id) {
            tracing::warn!(
                "[registry] dynamic connector '{}' skipped: ID collides with a builtin",
                def.id
            );
            continue;
        }

        // Allowlist check for the github repo
        if let Some(ref repo) = def.github_repo {
            if !allowlist.is_allowed(repo) {
                tracing::warn!(
                    "[registry] dynamic connector '{}' excluded: repo '{}' is not in the allowlist",
                    def.id,
                    repo
                );
                continue;
            }
        }

        result.push(def.to_manifest());
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allowlist::RepoAllowlist;
    use crate::config::{AllowlistConfig, DynamicConnectorDef};
    use std::collections::BTreeMap;

    fn make_config(dynamic: Vec<DynamicConnectorDef>) -> HubConfig {
        HubConfig {
            setup_complete: false,
            pick_tos_accepted: false,
            connectors: BTreeMap::new(),
            instance_ids: BTreeMap::new(),
            studio_url: None,
            allowlist: AllowlistConfig::default(),
            dynamic_connectors: dynamic,
        }
    }

    fn make_dynamic(id: &str, repo: &str) -> DynamicConnectorDef {
        DynamicConnectorDef {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            icon: "hero-puzzle-piece".to_string(),
            default_port: 3030,
            github_repo: Some(repo.to_string()),
            binary_hint: Some(format!("{}-bin", id)),
            asset_pattern: Some(format!("{}-bin-{{os}}-{{arch}}.{{ext}}", id)),
        }
    }

    #[test]
    fn test_merge_includes_allowed_dynamic() {
        let allowlist = RepoAllowlist::from_patterns(vec![
            "Strike48-public/*".to_string(),
            "my-corp/my-tool".to_string(),
        ]);
        let config = make_config(vec![make_dynamic("my-tool", "my-corp/my-tool")]);

        let manifests = merge_manifests(&config, &allowlist);

        // 2 builtins + 1 dynamic
        assert_eq!(manifests.len(), 3);
        let ids: Vec<&str> = manifests.iter().map(|m| m.id.as_ref()).collect();
        assert!(ids.contains(&"kubestudio"));
        assert!(ids.contains(&"pick"));
        assert!(ids.contains(&"my-tool"));

        // Dynamic entry should have is_builtin = false
        let my_tool = manifests.iter().find(|m| m.id == "my-tool").unwrap();
        assert!(!my_tool.is_builtin);
        assert_eq!(my_tool.github_repo.as_deref(), Some("my-corp/my-tool"));
    }

    #[test]
    fn test_merge_excludes_disallowed_dynamic() {
        let allowlist = RepoAllowlist::from_patterns(vec!["Strike48-public/*".to_string()]);
        let config = make_config(vec![make_dynamic("evil-tool", "evil-org/evil-tool")]);

        let manifests = merge_manifests(&config, &allowlist);

        // Only 2 builtins; evil-tool excluded
        assert_eq!(manifests.len(), 2);
        let ids: Vec<&str> = manifests.iter().map(|m| m.id.as_ref()).collect();
        assert!(!ids.contains(&"evil-tool"));
    }

    #[test]
    fn test_merge_skips_builtin_id_collision() {
        let allowlist = RepoAllowlist::from_patterns(vec!["Strike48-public/*".to_string()]);
        // Try to override kubestudio (a builtin ID)
        let config = make_config(vec![make_dynamic(
            "kubestudio",
            "Strike48-public/kubestudio",
        )]);

        let manifests = merge_manifests(&config, &allowlist);

        // Still only 2 builtins; the duplicate is skipped
        assert_eq!(manifests.len(), 2);
        // The kubestudio entry should be the builtin (is_builtin = true)
        let ks = manifests.iter().find(|m| m.id == "kubestudio").unwrap();
        assert!(ks.is_builtin);
    }

    #[test]
    fn test_merge_dynamic_without_repo_always_included() {
        // A dynamic connector with no github_repo (local/socket only)
        let allowlist = RepoAllowlist::from_patterns(vec!["Strike48-public/*".to_string()]);
        let mut def = make_dynamic("local-thing", "whatever/repo");
        def.github_repo = None;
        let config = make_config(vec![def]);

        let manifests = merge_manifests(&config, &allowlist);

        // 2 builtins + 1 dynamic (no repo = no allowlist check)
        assert_eq!(manifests.len(), 3);
        assert!(manifests.iter().any(|m| m.id == "local-thing"));
    }

    #[test]
    fn test_merge_empty_config_returns_builtins() {
        let allowlist = RepoAllowlist::from_patterns(vec!["Strike48-public/*".to_string()]);
        let config = make_config(vec![]);

        let manifests = merge_manifests(&config, &allowlist);

        assert_eq!(manifests.len(), 2);
        assert!(manifests.iter().all(|m| m.is_builtin));
    }

    #[test]
    fn test_merge_multiple_dynamic_mixed_allowlist() {
        let allowlist = RepoAllowlist::from_patterns(vec![
            "Strike48-public/*".to_string(),
            "acme-corp/allowed-tool".to_string(),
        ]);
        let config = make_config(vec![
            make_dynamic("allowed-tool", "acme-corp/allowed-tool"),
            make_dynamic("blocked-tool", "evil-org/blocked-tool"),
            make_dynamic("also-allowed", "Strike48-public/also-allowed"),
        ]);

        let manifests = merge_manifests(&config, &allowlist);

        // 2 builtins + 2 allowed dynamic (blocked-tool excluded)
        assert_eq!(manifests.len(), 4);
        let ids: Vec<&str> = manifests.iter().map(|m| m.id.as_ref()).collect();
        assert!(ids.contains(&"allowed-tool"));
        assert!(ids.contains(&"also-allowed"));
        assert!(!ids.contains(&"blocked-tool"));
    }
}
