use std::process::Command;

use crate::auth::{AuthManager, ConnectorAppInfo, fetch_connector_apps};
use crate::config::ConnectorStatus;
use crate::matrix_ws::MatrixWsClient;

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
    let (name, checks) = match connector_id {
        "kubestudio" => ("KubeStudio", run_kubestudio_checks().await),
        "pick" => ("Pick", run_pick_checks().await),
        _ => return PreflightResult {
            connector_id: connector_id.to_string(),
            connector_name: connector_id.to_string(),
            checks: vec![],
        },
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
            },
            None => PreflightCheck {
                name: "Process".into(),
                description: format!("{} connector has not started", display_name),
                status: CheckStatus::Failed,
                install_hint: format!(
                    "The {} connector binary may not be installed.\n\
                     Ensure the binary is available in your PATH or ~/bin/.",
                    display_name
                ),
            },
        });

        // Check 2: registered with Matrix
        let registered = is_connector_registered(id, &apps);
        checks.push(if registered {
            PreflightCheck {
                name: "Registration".into(),
                description: format!("{} is registered with Strike48", display_name),
                status: CheckStatus::Passed,
                install_hint: String::new(),
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
            }
        });

        result.results.push(PreflightResult {
            connector_id: format!("reg-{}", id),
            connector_name: format!("{}", display_name),
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
        _ => return apps.iter().any(|app| {
            app.name.to_lowercase().contains(connector_id)
        }),
    };
    apps.iter().any(|app| {
        let name_lower = app.name.to_lowercase();
        let addr_lower = app
            .address
            .as_deref()
            .map(|a| a.to_lowercase())
            .unwrap_or_default();
        patterns.iter().any(|p| name_lower.contains(p) || addr_lower.contains(p))
    })
}

fn connector_display_name(id: &str) -> &str {
    match id {
        "kubestudio" => "KubeStudio",
        "pick" => "Pick",
        _ => id,
    }
}

async fn run_kubestudio_checks() -> Vec<PreflightCheck> {
    let kubectl_check = tokio::task::spawn_blocking(check_kube_context).await;
    vec![kubectl_check.unwrap_or_else(|_| PreflightCheck {
        name: "Kubernetes Context".into(),
        description: "A Kubernetes cluster context must be configured".into(),
        status: CheckStatus::Failed,
        install_hint: "Could not verify Kubernetes context.".into(),
    })]
}

async fn run_pick_checks() -> Vec<PreflightCheck> {
    let docker_check = tokio::task::spawn_blocking(check_docker_cli).await;
    vec![docker_check.unwrap_or_else(|_| PreflightCheck {
        name: "Docker CLI".into(),
        description: "Docker must be installed and running".into(),
        status: CheckStatus::Failed,
        install_hint: "Could not verify Docker installation.".into(),
    })]
}

/// Check if there is at least one Kubernetes context available.
fn check_kube_context() -> PreflightCheck {
    let name = "Kubernetes Context".to_string();

    // Try kubectl first
    if let Ok(output) = Command::new("kubectl")
        .args(["config", "get-contexts", "-o", "name"])
        .output()
    {
        if output.status.success() {
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
                };
            }
        }
    }

    // Fall back: check if ~/.kube/config exists with any context
    if let Some(home) = dirs::home_dir() {
        let kubeconfig = home.join(".kube").join("config");
        if kubeconfig.exists() {
            if let Ok(content) = std::fs::read_to_string(&kubeconfig) {
                if content.contains("contexts:") && content.contains("- context:") {
                    return PreflightCheck {
                        name,
                        description: "Found kubeconfig with cluster contexts".into(),
                        status: CheckStatus::Passed,
                        install_hint: String::new(),
                    };
                }
            }
        }
    }

    PreflightCheck {
        name,
        description: "No Kubernetes cluster context found".into(),
        status: CheckStatus::Failed,
        install_hint: "Install kubectl and configure a cluster context:\n\n\
            macOS:  brew install kubectl\n\
            Linux:  curl -LO \"https://dl.k8s.io/release/$(curl -Ls https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl\" && chmod +x kubectl && sudo mv kubectl /usr/local/bin/\n\n\
            Then configure a context:\n  kubectl config set-context my-cluster --cluster=<cluster> --user=<user>\n\n\
            Or use Docker Desktop, Rancher Desktop, minikube, or kind to create a local cluster."
            .into(),
    }
}

/// Check if the Docker CLI is available and the daemon is responsive.
fn check_docker_cli() -> PreflightCheck {
    let name = "Docker CLI".to_string();

    // Check if docker binary exists
    let docker_exists = Command::new("docker").arg("--version").output();
    match docker_exists {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .trim()
                .to_string();

            // Check if daemon is running
            match Command::new("docker").arg("info").output() {
                Ok(info_output) if info_output.status.success() => PreflightCheck {
                    name,
                    description: format!("{} (daemon running)", version),
                    status: CheckStatus::Passed,
                    install_hint: String::new(),
                },
                _ => PreflightCheck {
                    name,
                    description: format!("{} (daemon not running)", version),
                    status: CheckStatus::Failed,
                    install_hint:
                        "Docker is installed but the daemon is not running.\n\n\
                        Start Docker Desktop, or run:\n  sudo systemctl start docker\n\n\
                        On macOS you can also run:\n  open -a Docker"
                            .into(),
                },
            }
        }
        _ => PreflightCheck {
            name,
            description: "Docker CLI not found".into(),
            status: CheckStatus::Failed,
            install_hint:
                "Install Docker Desktop:\n\n\
                macOS:   https://docs.docker.com/desktop/install/mac-install/\n\
                Linux:   https://docs.docker.com/engine/install/\n\
                Windows: https://docs.docker.com/desktop/install/windows-install/\n\n\
                Or install via Homebrew (macOS):\n  brew install --cask docker"
                    .into(),
        },
    }
}
