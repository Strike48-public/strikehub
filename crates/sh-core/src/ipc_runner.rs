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
    #[tracing::instrument(
        name = "connector.start",
        skip(env_vars),
        fields(
            connector.id = %id,
            outcome = tracing::field::Empty,
            startup_ms = tracing::field::Empty,
        )
    )]
    pub async fn start(
        id: &str,
        binary: &Path,
        env_vars: &[(String, String)],
    ) -> Result<Self, HubError> {
        let span = tracing::Span::current();
        let start = std::time::Instant::now();

        let ipc_addr = IpcAddr::for_connector(id);

        // Remove stale socket from a previous run (no-op on Windows)
        ipc_addr.cleanup();

        // Resolve the binary: a bare name is searched against dev workspace
        // builds, the runtime fetch cache, then the bundled sibling (see
        // `resolve_binary_in`). Explicit paths are used as-is.
        let resolved = resolve_binary(binary);

        let mut cmd = tokio::process::Command::new(&resolved);
        cmd.env("STRIKEHUB_SOCKET", ipc_addr.to_env_string());
        for (k, v) in env_vars {
            cmd.env(k, v);
        }

        // Ensure tokio kills the child process if the Child handle is dropped
        // without an explicit wait/kill. This is the primary defense against
        // orphaned connector processes when StrikeHub exits unexpectedly.
        cmd.kill_on_drop(true);

        // On Windows the desktop app has no console, so inheriting stdio would
        // cause Windows to allocate a visible console window for each connector.
        // Suppress that by sending output to null and setting CREATE_NO_WINDOW.
        #[cfg(windows)]
        {
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        #[cfg(not(windows))]
        {
            cmd.stdout(std::process::Stdio::inherit());
            cmd.stderr(std::process::Stdio::inherit());
        }

        // On Unix, arrange for the child to receive SIGHUP when the parent
        // process dies. This covers abrupt kills (SIGKILL, OOM, crash) where
        // Drop destructors never run.
        #[cfg(unix)]
        {
            // SAFETY: pre_exec runs in the forked child before exec.
            // setsid/setpgid are not called, so the child remains in
            // the parent's process group — no async-signal-safety concern.
            unsafe {
                cmd.pre_exec(|| {
                    // On Linux, PR_SET_PDEATHSIG asks the kernel to send the
                    // specified signal to this process when its parent dies.
                    #[cfg(target_os = "linux")]
                    {
                        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGHUP);
                    }
                    // On macOS there is no PR_SET_PDEATHSIG equivalent, but
                    // kill_on_drop(true) and the Drop reap loop cover this case.
                    Ok(())
                });
            }
        }

        let child = cmd.spawn().map_err(|e| {
            span.record("outcome", "spawn_failed");
            span.record("startup_ms", start.elapsed().as_millis() as u64);
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
        let mut became_ready = false;
        loop {
            // Try a health check — on Unix this also implies the socket file exists,
            // on Windows the named pipe will accept connections once the child is ready.
            if runner.health_check().await {
                tracing::info!("connector '{}' is healthy", id);
                became_ready = true;
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!("connector '{}' did not become ready in time", id);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        span.record(
            "outcome",
            if became_ready {
                "ready"
            } else {
                "ready_timeout"
            },
        );
        span.record("startup_ms", start.elapsed().as_millis() as u64);

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
        // Synchronously reap the child to prevent zombie processes.
        // Drop cannot be async, so spin briefly with try_wait.
        for _ in 0..20 {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                _ => std::thread::sleep(std::time::Duration::from_millis(5)),
            }
        }
        self.ipc_addr.cleanup();
    }
}

// ── Binary resolution ──────────────────────────────────────────────────

/// Resolve a binary path against the dev / fetch-cache / bundle search paths.
fn resolve_binary(binary: &Path) -> PathBuf {
    resolve_binary_in(
        binary,
        std::env::current_exe().ok(),
        &crate::connector_fetch::bin_cache_dir(),
    )
}

/// Resolve a bare binary name. Split from [`resolve_binary`] so the precedence
/// rules can be unit-tested with explicit `exe` / `cache_dir` inputs.
///
/// Precedence (highest first):
/// 1. Explicit path (contains a separator) — used as-is (expand `~`).
/// 2. Sibling Cargo workspace builds — local dev builds win during development.
///    Covers the side-by-side repo layout (e.g. `~/code/strike48/strikehub/`
///    and `~/code/strike48/pick/`).
/// 3. Runtime fetch cache (`~/.strike48/strikehub/bin/`) — the dynamically
///    fetched latest connector release. Preferred over the bundled binary so
///    new connector releases reach users without a StrikeHub rebuild.
/// 4. Sibling of the running executable — the binary bundled in the release
///    tarball. Acts as a first-run seed / offline fallback before the cache
///    has been populated by `connector_fetch`.
/// 5. Fall back to the bare name (OS PATH lookup).
fn resolve_binary_in(binary: &Path, exe: Option<PathBuf>, cache_dir: &Path) -> PathBuf {
    // Expand leading ~ to home directory.
    let binary = if let Some(rest) = binary.to_str().and_then(|s| s.strip_prefix("~/")) {
        if let Some(home) = dirs::home_dir() {
            home.join(rest)
        } else {
            binary.to_path_buf()
        }
    } else {
        binary.to_path_buf()
    };

    // 1. If the path already contains a separator it is explicit — use as-is.
    if binary.components().count() > 1 {
        if path_exists(&binary) {
            return with_exe(binary);
        }
        // Path that doesn't exist — still return it so the caller gets a clear
        // "not found" error from Command::spawn.
        tracing::warn!("binary path does not exist: {}", binary.display());
        return binary;
    }

    // 2. Sibling Cargo workspace builds (dev side-by-side repos).
    if let Some(found) = resolve_in_sibling_workspaces(exe.as_deref(), &binary) {
        return found;
    }

    // 3. Runtime fetch cache — the dynamic catalogue. Preferred over the
    //    bundled sibling so published connector releases reach users at runtime
    //    without a StrikeHub rebuild.
    let cache_candidate = cache_dir.join(&binary);
    if path_exists(&cache_candidate) {
        let resolved = with_exe(cache_candidate);
        tracing::info!(
            "resolved '{}' → {} (fetch cache)",
            binary.display(),
            resolved.display()
        );
        return resolved;
    }

    // 4. Sibling of the running executable — release bundle / first-run seed.
    if let Some(exe) = exe.as_deref()
        && let Some(exe_dir) = exe.parent()
    {
        let sibling = exe_dir.join(&binary);
        if path_exists(&sibling) {
            let resolved = with_exe(sibling);
            tracing::info!(
                "resolved '{}' → {} (bundled sibling)",
                binary.display(),
                resolved.display()
            );
            return resolved;
        }
    }

    // 5. Fall back to PATH lookup.
    binary
}

/// Scan sibling Cargo workspaces (up to 3 ancestor levels) for
/// `{dir}/target/{profile}/{binary}`. Returns the first match, preferring the
/// profile that matches the running executable and falling back to the other.
fn resolve_in_sibling_workspaces(exe: Option<&Path>, binary: &Path) -> Option<PathBuf> {
    // exe is typically …/target/{profile}/strikehub
    let target_profile_dir = exe?.parent()?;
    let workspace_root = target_profile_dir.parent()?.parent()?;

    let profile = target_profile_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("debug");
    let alt_profile = if profile == "debug" {
        "release"
    } else {
        "debug"
    };

    let mut ancestor = workspace_root.to_path_buf();
    for _ in 0..3 {
        ancestor = ancestor.parent()?.to_path_buf();
        if let Some(found) = scan_for_binary(&ancestor, profile, binary) {
            tracing::info!(
                "resolved '{}' using {} profile (matched current)",
                binary.display(),
                profile,
            );
            return Some(found);
        }
        if let Some(found) = scan_for_binary(&ancestor, alt_profile, binary) {
            tracing::warn!(
                "resolved '{}' using {} profile (cross-profile fallback, current is {})",
                binary.display(),
                alt_profile,
                profile,
            );
            return Some(found);
        }
    }
    None
}

/// Check whether a path exists; on Windows also accept the `.exe` variant.
fn path_exists(p: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        p.exists() || (p.extension().is_none() && p.with_extension("exe").exists())
    }
    #[cfg(not(target_os = "windows"))]
    {
        p.exists()
    }
}

/// On Windows, return the `.exe` variant when the bare path doesn't exist.
fn with_exe(p: PathBuf) -> PathBuf {
    #[cfg(target_os = "windows")]
    if !p.exists() && p.extension().is_none() {
        let exe = p.with_extension("exe");
        if exe.exists() {
            return exe;
        }
    }
    p
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
        // Check for the candidate as-is, and with .exe on Windows.
        let found = if candidate.exists() {
            Some(candidate)
        } else if cfg!(target_os = "windows") && candidate.extension().is_none() {
            let exe = candidate.with_extension("exe");
            if exe.exists() { Some(exe) } else { None }
        } else {
            None
        };
        if let Some(resolved) = found {
            tracing::info!(
                "resolved '{}' → {} (sibling workspace)",
                binary.display(),
                resolved.display()
            );
            return Some(resolved);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Create a fresh, unique temp directory for a test.
    fn unique_tmp(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "strikehub-resolve-{}-{}-{}",
            label,
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Create an empty file, making parent directories as needed.
    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"#!/bin/sh\n").unwrap();
    }

    const BIN: &str = "pentest-agent";

    /// The fix: a fetched binary in the cache must win over the binary bundled
    /// next to the executable (the release-tarball layout).
    #[test]
    fn cache_preferred_over_bundled_sibling() {
        let base = unique_tmp("cache-wins");
        let exe = base.join("install").join("strikehub");
        let bundled = base.join("install").join(BIN);
        let cache_dir = base.join("cache");
        touch(&bundled);
        touch(&cache_dir.join(BIN));

        let resolved = resolve_binary_in(Path::new(BIN), Some(exe), &cache_dir);
        assert_eq!(resolved, cache_dir.join(BIN));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// First-run / offline: with no cached binary, fall back to the bundled
    /// sibling next to the executable.
    #[test]
    fn bundled_sibling_used_when_cache_empty() {
        let base = unique_tmp("bundle-seed");
        let exe = base.join("install").join("strikehub");
        let bundled = base.join("install").join(BIN);
        let cache_dir = base.join("cache"); // intentionally empty
        touch(&bundled);
        std::fs::create_dir_all(&cache_dir).unwrap();

        let resolved = resolve_binary_in(Path::new(BIN), Some(exe), &cache_dir);
        assert_eq!(resolved, bundled);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Dev layout: a sibling Cargo-workspace build wins over the fetch cache so
    /// developers always run their locally built connector.
    #[test]
    fn sibling_workspace_beats_cache() {
        let base = unique_tmp("dev-wins");
        let exe = base
            .join("strikehub")
            .join("target")
            .join("debug")
            .join("strikehub");
        let workspace_bin = base.join("pick").join("target").join("debug").join(BIN);
        let cache_dir = base.join("cache");
        touch(&workspace_bin);
        touch(&cache_dir.join(BIN));

        let resolved = resolve_binary_in(Path::new(BIN), Some(exe), &cache_dir);
        assert_eq!(resolved, workspace_bin);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// An explicit path containing a separator is used as-is.
    #[test]
    fn explicit_path_passthrough() {
        let base = unique_tmp("explicit");
        let explicit = base.join("custom").join(BIN);
        touch(&explicit);
        let cache_dir = base.join("cache");

        let resolved = resolve_binary_in(&explicit, None, &cache_dir);
        assert_eq!(resolved, explicit);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Nothing resolves: return the bare name unchanged for an OS PATH lookup.
    #[test]
    fn falls_back_to_bare_name() {
        let base = unique_tmp("path-fallback");
        let exe = base.join("install").join("strikehub");
        let cache_dir = base.join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let resolved = resolve_binary_in(Path::new(BIN), Some(exe), &cache_dir);
        assert_eq!(resolved, Path::new(BIN));

        let _ = std::fs::remove_dir_all(&base);
    }
}
