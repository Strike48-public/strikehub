use std::path::{Path, PathBuf};

use crate::HubError;
use crate::ipc::IpcAddr;

/// A connector running as a child process, communicating over IPC.
pub struct IpcConnectorRunner {
    id: String,
    child: tokio::process::Child,
    ipc_addr: IpcAddr,
}

impl IpcConnectorRunner {
    /// Spawn a connector binary with `STRIKEHUB_SOCKET` set, then poll until
    /// the IPC endpoint is ready (or timeout).
    pub async fn start(
        id: &str,
        binary: &Path,
        env_vars: &[(String, String)],
    ) -> Result<Self, HubError> {
        let ipc_addr = IpcAddr::for_connector(id);

        // Remove stale socket from a previous run (no-op on Windows)
        ipc_addr.cleanup();

        // Resolve the binary: if it's a bare name (no path separator), look
        // for it next to the running executable first, then fall back to PATH.
        let resolved = resolve_binary(binary);

        let mut cmd = tokio::process::Command::new(&resolved);
        cmd.env("STRIKEHUB_SOCKET", ipc_addr.to_env_string());
        for (k, v) in env_vars {
            cmd.env(k, v);
        }

        // Inherit stdout/stderr so connector logs appear in StrikeHub's console
        cmd.stdout(std::process::Stdio::inherit());
        cmd.stderr(std::process::Stdio::inherit());

        let child = cmd.spawn().map_err(|e| {
            HubError::Runner(format!("failed to spawn {}: {}", binary.display(), e))
        })?;

        tracing::info!(
            "spawned connector '{}' (pid={:?}, bin={}) → {}",
            id,
            child.id(),
            resolved.display(),
            ipc_addr
        );

        let runner = Self {
            id: id.to_string(),
            child,
            ipc_addr,
        };

        // Wait for the IPC endpoint to become ready (up to 15 s)
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            // Try a health check — on Unix this also implies the socket file exists,
            // on Windows the named pipe will accept connections once the child is ready.
            if runner.health_check().await {
                tracing::info!("connector '{}' is healthy", id);
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!("connector '{}' did not become ready in time", id);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        Ok(runner)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn ipc_addr(&self) -> &IpcAddr {
        &self.ipc_addr
    }

    /// Backward-compat: return the IPC address as a `PathBuf`.
    pub fn socket_path(&self) -> PathBuf {
        self.ipc_addr.to_path_buf()
    }

    /// HTTP GET `/health` over IPC. Returns `true` on 200 OK.
    pub async fn health_check(&self) -> bool {
        match ipc_http_get(&self.ipc_addr, "/health").await {
            Ok((status, _body)) => status == 200,
            Err(_) => false,
        }
    }

    /// HTTP GET `/connector/info` over IPC.
    pub async fn fetch_info(&self) -> Option<(String, Option<String>)> {
        let (_status, body) = ipc_http_get(&self.ipc_addr, "/connector/info").await.ok()?;
        let json: serde_json::Value = serde_json::from_slice(&body).ok()?;
        let name = json.get("name")?.as_str()?.to_string();
        let icon = json.get("icon").and_then(|v| v.as_str()).map(String::from);
        Some((name, icon))
    }

    /// Kill the child process and clean up the IPC endpoint.
    pub async fn stop(&mut self) -> Result<(), HubError> {
        let _ = self.child.kill().await;
        self.ipc_addr.cleanup();
        tracing::info!("stopped connector '{}'", self.id);
        Ok(())
    }
}

impl Drop for IpcConnectorRunner {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        self.ipc_addr.cleanup();
    }
}

// ── Binary resolution ──────────────────────────────────────────────────

/// Resolve a binary path.
///
/// Search order:
/// 1. If the path is absolute or contains a separator, use as-is (expand `~`).
/// 2. Next to the running executable (same `target/{profile}/` dir).
/// 3. Sibling Cargo workspaces' `target/{profile}/` dirs — this covers the
///    common dev layout where strikehub and connector repos live side by side
///    (e.g. `~/code/strike48/scratch/strikehub/` and `~/code/strike48/studio-kube-desktop/`).
/// 4. Fall back to the bare name (OS PATH lookup).
fn resolve_binary(binary: &Path) -> PathBuf {
    // Expand leading ~ to home directory
    let binary = if let Some(rest) = binary.to_str().and_then(|s| s.strip_prefix("~/")) {
        if let Some(home) = dirs::home_dir() {
            home.join(rest)
        } else {
            binary.to_path_buf()
        }
    } else {
        binary.to_path_buf()
    };

    // If the path already contains a separator it is explicit — use as-is.
    if binary.components().count() > 1 {
        if binary.exists() {
            return binary;
        }
        // Absolute path that doesn't exist — still return it so the caller
        // gets a clear "not found" error from Command::spawn.
        tracing::warn!("binary path does not exist: {}", binary.display());
        return binary;
    }

    if let Ok(exe) = std::env::current_exe() {
        // exe is typically …/target/debug/strikehub or …/target/release/strikehub
        if let Some(target_profile_dir) = exe.parent() {
            // 1. Sibling of current exe (same target dir)
            let sibling = target_profile_dir.join(&binary);
            if sibling.exists() {
                tracing::info!(
                    "resolved '{}' → {} (sibling)",
                    binary.display(),
                    sibling.display()
                );
                return sibling;
            }

            // 2. Sibling workspaces: walk up from the workspace root and
            //    scan nearby directories (up to 3 levels) for
            //    {dir}/target/{profile}/{binary}.  This handles layouts like:
            //      ~/code/strike48/scratch/strikehub/   (this workspace)
            //      ~/code/strike48/studio-kube-desktop/  (connector workspace)
            if let Some(target_dir) = target_profile_dir.parent() {
                let profile = target_profile_dir
                    .file_name()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or("debug");

                if let Some(workspace_root) = target_dir.parent() {
                    // Scan ancestors: parent, grandparent, etc.
                    let mut ancestor = workspace_root.to_path_buf();
                    for _ in 0..3 {
                        ancestor = match ancestor.parent() {
                            Some(p) => p.to_path_buf(),
                            None => break,
                        };
                        if let Some(found) = scan_for_binary(&ancestor, profile, &binary) {
                            return found;
                        }
                    }
                }
            }
        }
    }

    // Fall back to PATH lookup
    binary
}

/// Recursively scan a directory (1 level of subdirs) for `target/{profile}/{binary}`.
fn scan_for_binary(dir: &Path, profile: &str, binary: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let candidate = path.join("target").join(profile).join(binary);
        if candidate.exists() {
            tracing::info!(
                "resolved '{}' → {} (sibling workspace)",
                binary.display(),
                candidate.display()
            );
            return Some(candidate);
        }
    }
    None
}

// ── IPC HTTP helpers ──────────────────────────────────────────────────

/// Perform an HTTP/1.1 GET request over IPC.
/// Returns `(status_code, body_bytes)`.
pub(crate) async fn ipc_http_get(
    addr: &IpcAddr,
    uri_path: &str,
) -> Result<(u16, Vec<u8>), anyhow::Error> {
    use http_body_util::BodyExt;
    use hyper::body::Bytes;
    use hyper_util::rt::TokioIo;

    let stream = crate::ipc::IpcStream::connect(addr).await?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::debug!("IPC http connection error: {}", e);
        }
    });

    let req = hyper::Request::builder()
        .method("GET")
        .uri(uri_path)
        .header("Host", "localhost")
        .body(http_body_util::Empty::<Bytes>::new())?;

    let resp = sender.send_request(req).await?;
    let status = resp.status().as_u16();
    let body = resp.into_body().collect().await?.to_bytes().to_vec();
    Ok((status, body))
}

/// Full HTTP GET over IPC returning status, headers, and body.
/// Used by the bridge to proxy requests to connector processes.
pub async fn ipc_http_get_full(
    addr: &IpcAddr,
    uri_path: &str,
) -> Result<(u16, Vec<(String, String)>, Vec<u8>), anyhow::Error> {
    use http_body_util::BodyExt;
    use hyper::body::Bytes;
    use hyper_util::rt::TokioIo;

    let stream = crate::ipc::IpcStream::connect(addr).await?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::debug!("IPC http connection error: {}", e);
        }
    });

    let req = hyper::Request::builder()
        .method("GET")
        .uri(uri_path)
        .header("Host", "localhost")
        .body(http_body_util::Empty::<Bytes>::new())?;

    let resp = sender.send_request(req).await?;
    let status = resp.status().as_u16();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
        .collect();
    let body = resp.into_body().collect().await?.to_bytes().to_vec();
    Ok((status, headers, body))
}
