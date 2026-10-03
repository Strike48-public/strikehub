use std::process::Command;

use crate::auth::{AuthManager, ConnectorAppInfo, fetch_connector_apps};
use crate::config::{ConnectorStatus, StartFailure, log_dir};
use crate::matrix_ws::MatrixWsClient;

/// Create a `Command` that won't open a visible console window on Windows.
pub(crate) fn hidden_command(program: &str) -> Command {
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
    /// Not a pass, but not something the user can fix; they may continue.
    Warning,
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
    /// When set, the preflight UI shows an "Install" button ("Start" when
    /// [`PreflightCheck::is_start_action`] is true).
    pub install_command: Option<String>,
}

/// Description suffix for a dependency that is installed but whose daemon is
/// not running. Its `install_command` starts the daemon rather than installing.
const DAEMON_NOT_RUNNING: &str = "(daemon not running)";

impl PreflightCheck {
    /// True when `install_command` starts an already-installed dependency, so
    /// the UI should offer "Start" rather than "Install".
    pub fn is_start_action(&self) -> bool {
        self.description.ends_with(DAEMON_NOT_RUNNING)
    }
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
pub async fn run_preflight(connector_id: &str) -> PreflightResult {
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
///
/// Checks run in parallel across connectors.
pub async fn run_preflight_all(connector_ids: &[String]) -> AggregatePreflightResult {
    let platform = tokio::task::spawn_blocking(crate::platform::native_arch)
        .await
        .ok()
        .and_then(|arch| platform_result(std::env::consts::OS, arch));
    let futures: Vec<_> = connector_ids.iter().map(|id| run_preflight(id)).collect();
    let all = futures::future::join_all(futures).await;
    let results = platform
        .into_iter()
        .chain(all.into_iter().filter(|r| !r.checks.is_empty()))
        .collect();
    AggregatePreflightResult { results }
}

/// A step-1 "Platform" warning when StrikeHub does not ship a build for this
/// OS and CPU, or `None` on a supported platform so nothing extra is shown.
fn platform_result(os: &str, arch: &str) -> Option<PreflightResult> {
    if crate::platform::is_supported(os, arch) {
        return None;
    }
    let supported = crate::platform::SUPPORTED_PLATFORMS
        .iter()
        .map(|(os, arch)| format!("{os} {arch}"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(PreflightResult {
        connector_id: "platform".into(),
        connector_name: "StrikeHub".into(),
        checks: vec![PreflightCheck {
            name: "Platform".into(),
            description: format!("{os} {arch} is not supported"),
            status: CheckStatus::Warning,
            install_hint: format!(
                "StrikeHub is not built or tested for this platform. You can continue \
                 at your own risk, but connectors may fail with errors that do not \
                 mention the platform.\n\nSupported: {supported}."
            ),
            install_command: None,
        }],
    })
}

/// Info about a connector's runtime state for registration checks.
#[derive(Debug, Clone)]
pub struct ConnectorRuntime {
    pub id: String,
    pub name: String,
    pub status: ConnectorStatus,
    /// True when StrikeHub holds a runner for this connector. A process that
    /// exited keeps its runner until the health check evicts it.
    pub process_running: bool,
    /// Why StrikeHub could not start the connector, when known.
    pub start_failure: Option<StartFailure>,
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
        let logs = log_dir().map(|dir| logs_hint(&dir, id)).unwrap_or_default();
        let mut checks = Vec::new();

        // Check 1: connector process is running
        let runtime = runtimes.iter().find(|r| r.id == *id);
        checks.push(match runtime {
            Some(rt) => process_check(
                display_name,
                rt,
                &failed_prerequisites(&result.results, id),
                &logs,
            ),
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
                        install_hint: with_logs(&hint, &logs),
                        install_command: None,
                    }
                }
            }
        });

        // Check 2: registered with Matrix
        let registered = is_connector_registered(id, &apps);
        let process_status = checks
            .iter()
            .find(|c| c.name == "Process")
            .map_or(CheckStatus::Failed, |c| c.status.clone());
        checks.push(registration_check(
            display_name,
            registered,
            &process_status,
        ));

        result.results.push(PreflightResult {
            connector_id: format!("reg-{}", id),
            connector_name: display_name.to_string(),
            checks,
        });
    }

    result
}

/// One line telling the user where to find the logs for a connector.
fn logs_hint(dir: &std::path::Path, connector_id: &str) -> String {
    if cfg!(target_os = "windows") {
        format!(
            "Logs: {} (strikehub.log.*, connector-{}.log)",
            dir.display(),
            connector_id
        )
    } else {
        format!("Logs: {} (strikehub.log.*)", dir.display())
    }
}

/// A failing check's hint followed by the logs line, if there is one.
fn with_logs(hint: &str, logs: &str) -> String {
    format!("{hint}\n{logs}").trim_end().to_string()
}

/// The step-2 "Process" check for a connector StrikeHub manages.
///
/// Each way a connector can fail to be Online gets its own message, so the
/// hint never claims the process started when StrikeHub never launched it.
fn process_check(
    display_name: &str,
    rt: &ConnectorRuntime,
    failed_prereqs: &[String],
    logs: &str,
) -> PreflightCheck {
    let failed = |description: String, hint: String| PreflightCheck {
        name: "Process".into(),
        description,
        status: CheckStatus::Failed,
        install_hint: with_logs(&hint, logs),
        install_command: None,
    };
    if rt.status == ConnectorStatus::Online {
        return PreflightCheck {
            name: "Process".into(),
            description: format!("{} connector is running", display_name),
            status: CheckStatus::Passed,
            install_hint: String::new(),
            install_command: None,
        };
    }
    match &rt.start_failure {
        Some(StartFailure::NoCredentials) => failed(
            format!("{} connector was not started", display_name),
            format!(
                "StrikeHub could not get credentials for the {} connector: there are no \
                 saved credentials and a registration token could not be created, so it \
                 was not launched.\nSign out, sign back in, then Re-check. If it persists, \
                 share the logs below.",
                display_name
            ),
        ),
        Some(StartFailure::SpawnFailed(err)) => failed(
            format!("{} connector failed to launch", display_name),
            format!(
                "StrikeHub could not launch the {} connector: {}",
                display_name, err
            ),
        ),
        None if rt.process_running => {
            let check = unhealthy_process_check(display_name, failed_prereqs);
            failed(check.description, check.install_hint)
        }
        None => PreflightCheck {
            name: "Process".into(),
            description: format!("{} connector is starting", display_name),
            status: CheckStatus::Checking,
            install_hint: String::new(),
            install_command: None,
        },
    }
}

/// The step-2 "Registration" check.
///
/// A connector can only register once it is running, so the approval and
/// configuration advice is shown only when the Process check passed.
/// Otherwise the hint points back at the Process check.
fn registration_check(
    display_name: &str,
    registered: bool,
    process_status: &CheckStatus,
) -> PreflightCheck {
    let check = |description: String, status: CheckStatus, install_hint: String| PreflightCheck {
        name: "Registration".into(),
        description,
        status,
        install_hint,
        install_command: None,
    };
    if registered {
        return check(
            format!("{} is registered with Strike48", display_name),
            CheckStatus::Passed,
            String::new(),
        );
    }
    let not_registered = format!("{} is not yet registered with Strike48", display_name);
    match process_status {
        CheckStatus::Passed => check(
            not_registered,
            CheckStatus::Failed,
            format!(
                "The {} connector is running but has not registered with the Strike48 platform.\n\
                 This usually means:\n\
                 \u{2022} The connector is still connecting (try Re-check)\n\
                 \u{2022} The connector needs approval in the Strike48 dashboard\n\
                 \u{2022} The STRIKE48_URL or TENANT_ID environment is misconfigured",
                display_name
            ),
        ),
        CheckStatus::Checking => check(not_registered, CheckStatus::Checking, String::new()),
        CheckStatus::Failed | CheckStatus::Warning => check(
            not_registered,
            CheckStatus::Failed,
            format!(
                "The {} connector cannot register until it is running.\n\
                 Fix the Process check above first.",
                display_name
            ),
        ),
    }
}

/// Names of the failed step-1 (device-posture) checks for a connector.
fn failed_prerequisites(results: &[PreflightResult], connector_id: &str) -> Vec<String> {
    results
        .iter()
        .filter(|r| r.connector_id == connector_id)
        .flat_map(|r| &r.checks)
        .filter(|c| c.status == CheckStatus::Failed)
        .map(|c| c.name.clone())
        .collect()
}

/// The "Process" check for a connector that is running but not healthy.
///
/// When a step-1 prerequisite failed (for Pick, Docker), that is the likely
/// cause, so name it instead of sending the user to the application logs.
fn unhealthy_process_check(display_name: &str, failed_prereqs: &[String]) -> PreflightCheck {
    let install_hint = if failed_prereqs.is_empty() {
        format!(
            "The {} connector process started but is not healthy.\n\
             Check the application logs for errors.",
            display_name
        )
    } else {
        format!(
            "The {} connector process started but is not healthy.\n\
             Likely cause: a required prerequisite failed ({}).\n\
             Go back to step 1 (Device Posture), fix it, then Re-check.",
            display_name,
            failed_prereqs.join(", ")
        )
    };
    PreflightCheck {
        name: "Process".into(),
        description: format!("{} connector is not responding", display_name),
        status: CheckStatus::Failed,
        install_hint,
        install_command: None,
    }
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
                        description: format!("{} {}", version, DAEMON_NOT_RUNNING),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn check(name: &str, status: CheckStatus) -> PreflightCheck {
        PreflightCheck {
            name: name.into(),
            description: String::new(),
            status,
            install_hint: String::new(),
            install_command: None,
        }
    }

    fn check_with_description(description: &str) -> PreflightCheck {
        PreflightCheck {
            name: "Docker CLI".into(),
            description: description.into(),
            status: CheckStatus::Failed,
            install_hint: String::new(),
            install_command: Some("cmd".into()),
        }
    }

    fn group(connector_id: &str, checks: Vec<PreflightCheck>) -> PreflightResult {
        PreflightResult {
            connector_id: connector_id.into(),
            connector_name: connector_id.into(),
            checks,
        }
    }

    const LOGS: &str = "Logs: /tmp/StrikeHub/logs";

    fn runtime(
        status: ConnectorStatus,
        process_running: bool,
        start_failure: Option<StartFailure>,
    ) -> ConnectorRuntime {
        ConnectorRuntime {
            id: "pick".into(),
            name: "Pick".into(),
            status,
            process_running,
            start_failure,
        }
    }

    #[test]
    fn process_check_passes_when_online() {
        let c = process_check(
            "Pick",
            &runtime(ConnectorStatus::Online, true, None),
            &[],
            LOGS,
        );
        assert_eq!(c.status, CheckStatus::Passed);
    }

    #[test]
    fn process_check_says_not_started_when_credentials_missing() {
        let rt = runtime(
            ConnectorStatus::Offline,
            false,
            Some(StartFailure::NoCredentials),
        );
        let c = process_check("Pick", &rt, &[], LOGS);
        assert_eq!(c.status, CheckStatus::Failed);
        assert_eq!(c.description, "Pick connector was not started");
        assert!(c.install_hint.contains("credentials"), "{}", c.install_hint);
        assert!(
            !c.install_hint.contains("process started"),
            "{}",
            c.install_hint
        );
        assert!(c.install_hint.contains(LOGS), "{}", c.install_hint);
    }

    #[test]
    fn process_check_reports_spawn_error() {
        let rt = runtime(
            ConnectorStatus::Offline,
            false,
            Some(StartFailure::SpawnFailed("access denied".into())),
        );
        let c = process_check("Pick", &rt, &[], LOGS);
        assert_eq!(c.status, CheckStatus::Failed);
        assert_eq!(c.description, "Pick connector failed to launch");
        assert!(
            c.install_hint.contains("access denied"),
            "{}",
            c.install_hint
        );
        assert!(c.install_hint.contains(LOGS), "{}", c.install_hint);
    }

    #[test]
    fn process_check_running_but_unhealthy_keeps_health_hint_and_adds_logs() {
        let rt = runtime(ConnectorStatus::Offline, true, None);
        let c = process_check("Pick", &rt, &["Docker CLI".to_string()], LOGS);
        assert_eq!(c.status, CheckStatus::Failed);
        assert_eq!(c.description, "Pick connector is not responding");
        assert!(c.install_hint.contains("Docker CLI"), "{}", c.install_hint);
        assert!(c.install_hint.contains(LOGS), "{}", c.install_hint);
    }

    #[test]
    fn process_check_not_running_without_failure_is_still_starting() {
        let rt = runtime(ConnectorStatus::Offline, false, None);
        let c = process_check("Pick", &rt, &[], LOGS);
        assert_eq!(c.status, CheckStatus::Checking);
        assert_eq!(c.description, "Pick connector is starting");
    }

    #[test]
    fn logs_hint_names_the_log_directory() {
        let hint = logs_hint(std::path::Path::new("/x/StrikeHub/logs"), "pick");
        assert!(hint.contains("/x/StrikeHub/logs"), "{hint}");
        assert!(hint.contains("strikehub.log"), "{hint}");
    }

    #[test]
    fn registration_check_passes_when_registered() {
        let c = registration_check("Pick", true, &CheckStatus::Passed);
        assert_eq!(c.status, CheckStatus::Passed);
    }

    #[test]
    fn registration_check_suggests_approval_only_when_process_is_running() {
        let c = registration_check("Pick", false, &CheckStatus::Passed);
        assert_eq!(c.status, CheckStatus::Failed);
        assert!(c.install_hint.contains("approval"), "{}", c.install_hint);
    }

    #[test]
    fn registration_check_points_at_process_when_process_failed() {
        let c = registration_check("Pick", false, &CheckStatus::Failed);
        assert_eq!(c.status, CheckStatus::Failed);
        assert!(!c.install_hint.contains("approval"), "{}", c.install_hint);
        assert!(c.install_hint.contains("Process"), "{}", c.install_hint);
    }

    #[test]
    fn registration_check_waits_while_process_is_starting() {
        let c = registration_check("Pick", false, &CheckStatus::Checking);
        assert_eq!(c.status, CheckStatus::Checking);
        assert!(!c.install_hint.contains("approval"), "{}", c.install_hint);
    }

    #[test]
    fn is_start_action_true_when_daemon_not_running() {
        let desc = format!(
            "Docker version 29.8.1, build 4a63305 {}",
            DAEMON_NOT_RUNNING
        );
        assert!(check_with_description(&desc).is_start_action());
    }

    #[test]
    fn is_start_action_false_when_dependency_missing() {
        assert!(!check_with_description("Docker CLI not found").is_start_action());
    }

    #[test]
    fn failed_prerequisites_lists_only_failed_checks_for_that_connector() {
        let results = [
            group(
                "pick",
                vec![
                    check("Docker CLI", CheckStatus::Failed),
                    check("Other", CheckStatus::Passed),
                ],
            ),
            group("kubestudio", vec![check("kubectl", CheckStatus::Failed)]),
        ];
        assert_eq!(failed_prerequisites(&results, "pick"), vec!["Docker CLI"]);
    }

    #[test]
    fn failed_prerequisites_ignores_registration_groups() {
        let results = [group(
            "reg-pick",
            vec![check("Process", CheckStatus::Failed)],
        )];
        assert!(failed_prerequisites(&results, "pick").is_empty());
    }

    #[test]
    fn unhealthy_process_check_names_failed_prerequisite_as_cause() {
        let c = unhealthy_process_check("Pick", &["Docker CLI".to_string()]);
        assert_eq!(c.status, CheckStatus::Failed);
        assert!(c.install_hint.contains("Docker CLI"), "{}", c.install_hint);
        assert!(c.install_hint.contains("step 1"), "{}", c.install_hint);
        assert!(
            !c.install_hint.contains("application logs"),
            "{}",
            c.install_hint
        );
    }

    #[test]
    fn unhealthy_process_check_keeps_generic_hint_when_prerequisites_pass() {
        let c = unhealthy_process_check("Pick", &[]);
        assert_eq!(c.status, CheckStatus::Failed);
        assert_eq!(c.description, "Pick connector is not responding");
        assert_eq!(
            c.install_hint,
            "The Pick connector process started but is not healthy.\n\
             Check the application logs for errors."
        );
    }

    #[test]
    fn platform_result_is_none_on_a_supported_platform() {
        assert_eq!(platform_result("windows", "x86_64"), None);
        assert_eq!(platform_result("macos", "aarch64"), None);
    }

    #[test]
    fn platform_result_warns_on_windows_arm64() {
        let r = platform_result("windows", "aarch64").expect("unsupported platform");
        assert!(!r.connector_id.starts_with("reg-"));
        let c = &r.checks[0];
        assert_eq!(c.name, "Platform");
        assert_eq!(c.status, CheckStatus::Warning);
        assert_eq!(c.description, "windows aarch64 is not supported");
        assert!(
            c.install_hint.contains("windows x86_64"),
            "{}",
            c.install_hint
        );
        assert_eq!(c.install_command, None);
    }
}
