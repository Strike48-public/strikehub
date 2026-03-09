use std::process::Command;

use crate::auth::{AuthManager, ConnectorAppInfo, fetch_connector_apps};
use crate::config::ConnectorStatus;
use crate::matrix_ws::MatrixWsClient;

/// Create a `Command` that won't open a visible console window on Windows.
fn hidden_command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Refresh the process PATH from the registry on Windows.
///
/// After `winget install` adds a new entry to the user PATH, the running
/// process still has the old value. This re-reads the system and user PATH
/// from the registry via `reg query` and updates the process environment so
/// that subsequent commands can find newly-installed binaries.
#[cfg(target_os = "windows")]
fn refresh_path() {
    /// Expand `%VAR%` references using the current process environment.
    fn expand_env_vars(s: &str) -> String {
        let mut result = s.to_string();
        while let Some(start) = result.find('%') {
            if let Some(end) = result[start + 1..].find('%') {
                let var_name = &result[start + 1..start + 1 + end];
                if var_name.is_empty() {
                    break;
                }
                let value = std::env::var(var_name)
                    .or_else(|_| std::env::var(var_name.to_uppercase()))
                    .unwrap_or_default();
                result = format!(
                    "{}{}{}",
                    &result[..start],
                    value,
                    &result[start + 2 + end..]
                );
            } else {
                break;
            }
        }
        result
    }

    fn reg_query_path(key: &str) -> Option<String> {
        let output = hidden_command("reg")
            .args(["query", key, "/v", "Path"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        // Output format: "    Path    REG_EXPAND_SZ    C:\...;C:\..."
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Path") || trimmed.starts_with("PATH") {
                if let Some(pos) = trimmed.find("REG_") {
                    let after_type = &trimmed[pos..];
                    if let Some(val_start) = after_type.find("    ") {
                        let val = after_type[val_start..].trim();
                        if !val.is_empty() {
                            return Some(expand_env_vars(val));
                        }
                    }
                }
            }
        }
        None
    }

    let machine =
        reg_query_path(r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment")
            .unwrap_or_default();
    let user = reg_query_path(r"HKCU\Environment").unwrap_or_default();

    let new_path = format!("{};{}", machine, user);
    if new_path.len() > 2 {
        // SAFETY: preflight checks run sequentially on a single blocking
        // thread; no other thread reads PATH concurrently.
        unsafe { std::env::set_var("PATH", &new_path) };
        tracing::debug!("refreshed PATH: {}", new_path);
    }
}

#[cfg(not(target_os = "windows"))]
fn refresh_path() {
    // When launched as a GUI app (e.g. .app bundle from Finder, or a Linux
    // .desktop launcher), the process inherits a minimal PATH from launchd /
    // systemd — typically just /usr/bin:/bin:/usr/sbin:/sbin.  Tools installed
    // via Homebrew, Docker Desktop, snap, or package managers won't be found.
    //
    // On macOS, /usr/libexec/path_helper merges /etc/paths and /etc/paths.d/*
    // (which is how Homebrew, Docker Desktop, etc. register themselves).
    // We run it first, then append a few well-known fallback directories that
    // might not be covered.

    let current = std::env::var("PATH").unwrap_or_default();

    // On macOS, use path_helper to get the system-configured PATH.
    #[cfg(target_os = "macos")]
    let base = {
        std::process::Command::new("/usr/libexec/path_helper")
            .arg("-s")
            .output()
            .ok()
            .and_then(|o| {
                if !o.status.success() {
                    return None;
                }
                let out = String::from_utf8_lossy(&o.stdout).to_string();
                // Output is: PATH="..."; export PATH;
                out.strip_prefix("PATH=\"")
                    .and_then(|s| s.split('"').next())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| current.clone())
    };

    #[cfg(not(target_os = "macos"))]
    let base = current.clone();

    // Well-known directories where kubectl / docker / other tools are commonly
    // installed.  We only append ones that actually exist on disk and are not
    // already present in the PATH string.
    let home = dirs::home_dir().unwrap_or_default();
    let extra_dirs: Vec<std::path::PathBuf> = vec![
        // Homebrew (Apple Silicon + Intel)
        "/opt/homebrew/bin".into(),
        "/opt/homebrew/sbin".into(),
        "/usr/local/bin".into(),
        "/usr/local/sbin".into(),
        // Snap (Linux)
        "/snap/bin".into(),
        // Flatpak (Linux)
        "/var/lib/flatpak/exports/bin".into(),
        // User-local
        home.join("bin"),
        home.join(".local/bin"),
        // Rancher Desktop
        home.join(".rd/bin"),
    ];

    let mut parts: Vec<String> = base.split(':').map(|s| s.to_string()).collect();
    let mut changed = false;
    for dir in &extra_dirs {
        let s = dir.to_string_lossy().into_owned();
        if dir.is_dir() && !parts.contains(&s) {
            parts.push(s);
            changed = true;
        }
    }

    // Also merge back anything from the original PATH that path_helper may
    // have missed (e.g. entries added by the parent shell).
    for entry in current.split(':') {
        if !entry.is_empty() && !parts.iter().any(|p| p == entry) {
            parts.push(entry.to_string());
            changed = true;
        }
    }

    if changed {
        let new_path = parts.join(":");
        tracing::debug!("refreshed PATH: {}", new_path);
        // SAFETY: preflight checks run sequentially on a single blocking
        // thread before any concurrent readers.
        unsafe { std::env::set_var("PATH", &new_path) };
    }
}

/// Detected host operating system for platform-specific install hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    MacOs,
    Linux,
    Windows,
}

impl HostOs {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// Status of a single preflight check.
#[derive(Debug, Clone, PartialEq)]
pub enum CheckStatus {
    Checking,
    Passed,
    Failed,
}

/// A single prerequisite check result.
#[derive(Debug, Clone, PartialEq)]
pub struct PreflightCheck {
    pub name: String,
    pub description: String,
    pub status: CheckStatus,
    /// Shown when the check fails — tells the user how to fix it.
    pub install_hint: String,
    /// Optional shell command the UI can run to install the dependency.
    /// When set, the preflight UI shows an "Install" button.
    pub install_command: Option<String>,
}

/// Result of running all preflight checks for a connector.
#[derive(Debug, Clone, PartialEq)]
pub struct PreflightResult {
    pub connector_id: String,
    pub connector_name: String,
    pub checks: Vec<PreflightCheck>,
}

impl PreflightResult {
    pub fn all_passed(&self) -> bool {
        self.checks.iter().all(|c| c.status == CheckStatus::Passed)
    }
}

/// Aggregate result across all connectors.
#[derive(Debug, Clone, PartialEq)]
pub struct AggregatePreflightResult {
    pub results: Vec<PreflightResult>,
}

impl AggregatePreflightResult {
    pub fn all_passed(&self) -> bool {
        self.results.iter().all(|r| r.all_passed())
    }
}

/// Run preflight checks for a connector by ID (local prerequisites only).
///
/// On Windows the step-1 device-posture checks (Docker, kubectl, etc.) are
/// skipped because bundled connectors handle their own dependencies.
pub async fn run_preflight(connector_id: &str) -> PreflightResult {
    // Skip device-posture checks on Windows — they are not actionable there.
    if cfg!(target_os = "windows") {
        return PreflightResult {
            connector_id: connector_id.to_string(),
            connector_name: connector_id.to_string(),
            checks: vec![],
        };
    }

    // Pick up any PATH changes from installs that happened since launch.
    refresh_path();
    let (name, checks) = match connector_id {
        "kubestudio" => ("KubeStudio", run_kubestudio_checks().await),
        "pick" => ("Pick", run_pick_checks().await),
        _ => {
            return PreflightResult {
                connector_id: connector_id.to_string(),
                connector_name: connector_id.to_string(),
                checks: vec![],
            };
        }
    };
    PreflightResult {
        connector_id: connector_id.to_string(),
        connector_name: name.to_string(),
        checks,
    }
}

/// Run preflight checks for all given connector IDs (local prerequisites only).
pub async fn run_preflight_all(connector_ids: &[String]) -> AggregatePreflightResult {
    let mut results = Vec::new();
    for id in connector_ids {
        let result = run_preflight(id).await;
        if !result.checks.is_empty() {
            results.push(result);
        }
    }
    AggregatePreflightResult { results }
}

/// Info about a connector's runtime state for registration checks.
#[derive(Debug, Clone)]
pub struct ConnectorRuntime {
    pub id: String,
    pub name: String,
    pub status: ConnectorStatus,
}

/// Run the full preflight: local prerequisites + connector registration checks.
///
/// Queries Matrix to verify each connector has registered and is visible.
/// `runners` provides the current health status of managed connectors.
pub async fn run_preflight_full(
    connector_ids: &[String],
    auth: &AuthManager,
    ws_client: Option<&MatrixWsClient>,
    runtimes: &[ConnectorRuntime],
) -> AggregatePreflightResult {
    // Phase 1: local prerequisite checks
    let mut result = run_preflight_all(connector_ids).await;

    // Phase 2: connector process health + Matrix registration
    let apps = fetch_connector_apps(auth, ws_client).await;
    tracing::info!(
        "Preflight: discovered {} connector app(s) from Matrix: {:?}",
        apps.len(),
        apps.iter().map(|a| &a.name).collect::<Vec<_>>()
    );

    // Per-connector registration groups (prefixed with "reg-" to distinguish
    // from device-posture groups in the UI wizard).
    for id in connector_ids {
        let display_name = connector_display_name(id);
        let mut checks = Vec::new();

        // Check 1: connector process is running
        let runtime = runtimes.iter().find(|r| r.id == *id);
        checks.push(match runtime {
            Some(rt) if rt.status == ConnectorStatus::Online => PreflightCheck {
                name: "Process".into(),
                description: format!("{} connector is running", display_name),
                status: CheckStatus::Passed,
                install_hint: String::new(),
                install_command: None,
            },
            Some(_) => PreflightCheck {
                name: "Process".into(),
                description: format!("{} connector is not responding", display_name),
                status: CheckStatus::Failed,
                install_hint: format!(
                    "The {} connector process started but is not healthy.\n\
                     Check the application logs for errors.",
                    display_name
                ),
                install_command: None,
            },
            None => {
                let binary_name = connector_binary_name(id);
                // Check if the binary actually exists on disk before
                // claiming it is missing.
                let binary_found = find_connector_binary(binary_name);
                if binary_found {
                    PreflightCheck {
                        name: "Process".into(),
                        description: format!(
                            "{} connector has not started yet (waiting for sign-in)",
                            display_name
                        ),
                        status: CheckStatus::Checking,
                        install_hint: String::new(),
                        install_command: None,
                    }
                } else {
                    let hint = match HostOs::current() {
                        HostOs::Windows => format!(
                            "The {d} connector binary ({b}.exe) was not found.\n\n\
                             Check that {b}.exe is next to strikehub.exe,\n\
                             or add its location to your PATH.",
                            d = display_name,
                            b = binary_name
                        ),
                        _ => format!(
                            "The {d} connector binary was not found.\n\n\
                             Ensure \"{b}\" is in your PATH or ~/bin/.",
                            d = display_name,
                            b = binary_name
                        ),
                    };
                    PreflightCheck {
                        name: "Process".into(),
                        description: format!("{} connector binary not found", display_name),
                        status: CheckStatus::Failed,
                        install_hint: hint,
                        install_command: None,
                    }
                }
            }
        });

        // Check 2: registered with Matrix
        let registered = is_connector_registered(id, &apps);
        checks.push(if registered {
            PreflightCheck {
                name: "Registration".into(),
                description: format!("{} is registered with Strike48", display_name),
                status: CheckStatus::Passed,
                install_hint: String::new(),
                install_command: None,
            }
        } else {
            PreflightCheck {
                name: "Registration".into(),
                description: format!("{} is not yet registered with Strike48", display_name),
                status: CheckStatus::Failed,
                install_hint: format!(
                    "The {} connector has not registered with the Strike48 platform.\n\
                     This usually means:\n\
                     \u{2022} The connector is still starting up (try Re-check)\n\
                     \u{2022} The connector needs approval in the Strike48 dashboard\n\
                     \u{2022} The STRIKE48_URL or TENANT_ID environment is misconfigured",
                    display_name
                ),
                install_command: None,
            }
        });

        result.results.push(PreflightResult {
            connector_id: format!("reg-{}", id),
            connector_name: display_name.to_string(),
            checks,
        });
    }

    result
}

/// Check if a connector appears in the Matrix connector apps list.
fn is_connector_registered(connector_id: &str, apps: &[ConnectorAppInfo]) -> bool {
    // Multiple patterns to match against — the app name or address may use
    // different casing/formatting (e.g. "KubeStudio" vs "kube-studio").
    let patterns: &[&str] = match connector_id {
        "kubestudio" => &["kubestudio", "kube-studio"],
        "pick" => &["pentest", "pentest-connector"],
        _ => {
            return apps
                .iter()
                .any(|app| app.name.to_lowercase().contains(connector_id));
        }
    };
    apps.iter().any(|app| {
        let name_lower = app.name.to_lowercase();
        let addr_lower = app
            .address
            .as_deref()
            .map(|a| a.to_lowercase())
            .unwrap_or_default();
        patterns
            .iter()
            .any(|p| name_lower.contains(p) || addr_lower.contains(p))
    })
}

fn connector_display_name(id: &str) -> &str {
    match id {
        "kubestudio" => "KubeStudio",
        "pick" => "Pick",
        _ => id,
    }
}

fn connector_binary_name(id: &str) -> &str {
    match id {
        "kubestudio" => "ks-connector",
        "pick" => "pentest-agent",
        _ => id,
    }
}

/// Check if a connector binary can be found on disk (next to the exe or on PATH).
fn find_connector_binary(name: &str) -> bool {
    // Check next to the running executable
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join(name);
        if candidate.exists() {
            return true;
        }
        // On Windows, also check with .exe extension
        #[cfg(target_os = "windows")]
        {
            let exe_candidate = dir.join(format!("{}.exe", name));
            if exe_candidate.exists() {
                return true;
            }
        }
    }
    // Check on PATH
    hidden_command(name)
        .arg("--version")
        .output()
        .map(|_| true)
        .unwrap_or(false)
}

async fn run_kubestudio_checks() -> Vec<PreflightCheck> {
    let kubectl_check = tokio::task::spawn_blocking(check_kube_context).await;
    vec![kubectl_check.unwrap_or_else(|_| PreflightCheck {
        name: "Kubernetes Context".into(),
        description: "A Kubernetes cluster context must be configured".into(),
        status: CheckStatus::Failed,
        install_hint: "Could not verify Kubernetes context.".into(),
        install_command: None,
    })]
}

async fn run_pick_checks() -> Vec<PreflightCheck> {
    let docker_check = tokio::task::spawn_blocking(check_docker_cli).await;
    vec![docker_check.unwrap_or_else(|_| PreflightCheck {
        name: "Docker CLI".into(),
        description: "Docker must be installed and running".into(),
        status: CheckStatus::Failed,
        install_hint: "Could not verify Docker installation.".into(),
        install_command: None,
    })]
}

/// Check if there is at least one Kubernetes context available.
fn check_kube_context() -> PreflightCheck {
    let name = "Kubernetes Context".to_string();

    // First check if kubectl binary exists at all.
    // NOTE: do NOT use --short here — it was removed in kubectl v1.28+ and
    // causes a non-zero exit code, making us think kubectl is missing.
    let kubectl_found = hidden_command("kubectl")
        .args(["version", "--client"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if kubectl_found {
        // kubectl is installed — check for contexts.
        if let Ok(output) = hidden_command("kubectl")
            .args(["config", "get-contexts", "-o", "name"])
            .output()
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let contexts: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
            if !contexts.is_empty() {
                return PreflightCheck {
                    name,
                    description: format!(
                        "Found {} context{}: {}",
                        contexts.len(),
                        if contexts.len() == 1 { "" } else { "s" },
                        contexts.join(", ")
                    ),
                    status: CheckStatus::Passed,
                    install_hint: String::new(),
                    install_command: None,
                };
            }
        }

        // Fall back: check if ~/.kube/config exists with any context
        if let Some(home) = dirs::home_dir() {
            let kubeconfig = home.join(".kube").join("config");
            if kubeconfig.exists()
                && let Ok(content) = std::fs::read_to_string(&kubeconfig)
                && content.contains("contexts:")
                && content.contains("- context:")
            {
                return PreflightCheck {
                    name,
                    description: "Found kubeconfig with cluster contexts".into(),
                    status: CheckStatus::Passed,
                    install_hint: String::new(),
                    install_command: None,
                };
            }
        }

        // kubectl installed but no context configured.
        return PreflightCheck {
            name,
            description: "kubectl is installed but no cluster context is configured".into(),
            status: CheckStatus::Failed,
            install_hint: "\
# Configure a context:
kubectl config set-context my-cluster --cluster=<cluster> --user=<user>

# Or use Docker Desktop, Rancher Desktop, minikube, or kind to create a local cluster."
                .into(),
            install_command: None,
        };
    }

    // kubectl not found — show install instructions.
    let hint = match HostOs::current() {
        HostOs::MacOs => "\
brew install kubectl

# Then configure a context:
kubectl config set-context my-cluster --cluster=<cluster> --user=<user>

# Or use Docker Desktop, Rancher Desktop, minikube, or kind to create a local cluster.",

        HostOs::Linux => "\
curl -LO \"https://dl.k8s.io/release/$(curl -Ls https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl\"
chmod +x kubectl
sudo mv kubectl /usr/local/bin/

# Then configure a context:
kubectl config set-context my-cluster --cluster=<cluster> --user=<user>

# Or use Docker Desktop, Rancher Desktop, minikube, or kind to create a local cluster.",

        HostOs::Windows => "\
winget install Kubernetes.kubectl --source winget

# Then configure a context:
kubectl config set-context my-cluster --cluster=<cluster> --user=<user>

# Or use Docker Desktop, Rancher Desktop, minikube, or kind to create a local cluster.",
    };

    let install_cmd = match HostOs::current() {
        HostOs::MacOs => Some("brew install kubectl".into()),
        HostOs::Windows => {
            Some("winget install Kubernetes.kubectl --source winget --accept-source-agreements --accept-package-agreements".into())
        }
        HostOs::Linux => None,
    };

    PreflightCheck {
        name,
        description: "kubectl not found".into(),
        status: CheckStatus::Failed,
        install_hint: hint.into(),
        install_command: install_cmd,
    }
}

/// Check if the Docker CLI is available and the daemon is responsive.
fn check_docker_cli() -> PreflightCheck {
    let name = "Docker CLI".to_string();

    // Check if docker binary exists
    let docker_exists = hidden_command("docker").arg("--version").output();
    match docker_exists {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();

            // Check if daemon is running
            match hidden_command("docker").arg("info").output() {
                Ok(info_output) if info_output.status.success() => PreflightCheck {
                    name,
                    description: format!("{} (daemon running)", version),
                    status: CheckStatus::Passed,
                    install_hint: String::new(),
                    install_command: None,
                },
                _ => {
                    let (hint, cmd) = match HostOs::current() {
                        HostOs::MacOs => ("open -a Docker", Some("open -a Docker".into())),
                        HostOs::Linux => ("sudo systemctl start docker", None),
                        HostOs::Windows => (
                            "Launch Docker Desktop from the Start menu.",
                            Some("Start-Process 'C:\\Program Files\\Docker\\Docker\\Docker Desktop.exe'".into()),
                        ),
                    };
                    PreflightCheck {
                        name,
                        description: format!("{} (daemon not running)", version),
                        status: CheckStatus::Failed,
                        install_hint: hint.into(),
                        install_command: cmd,
                    }
                }
            }
        }
        _ => {
            let (hint, cmd) = match HostOs::current() {
                HostOs::MacOs => (
                    "brew install --cask docker",
                    Some("brew install --cask docker".into()),
                ),
                HostOs::Linux => (
                    "\
curl -fsSL https://get.docker.com -o get-docker.sh
sudo sh get-docker.sh",
                    None,
                ),
                HostOs::Windows => (
                    "winget install Docker.DockerDesktop --source winget",
                    Some("winget install Docker.DockerDesktop --source winget --accept-source-agreements --accept-package-agreements".into()),
                ),
            };
            PreflightCheck {
                name,
                description: "Docker CLI not found".into(),
                status: CheckStatus::Failed,
                install_hint: hint.into(),
                install_command: cmd,
            }
        }
    }
}
