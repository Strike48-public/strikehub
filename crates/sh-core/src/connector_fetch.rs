//! Runtime connector binary fetch from GitHub Releases.
//!
//! Downloads pre-built connector binaries, verifies SHA256 checksums, extracts
//! archives, and caches them in `~/.strike48/strikehub/bin/`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::registry::ConnectorManifest;

/// Monotonic per-process sequence so parallel connector fetches (see
/// `ensure_all_connector_binaries`, which downloads with `join_all`) never
/// stage into the same sibling directory.
static STAGE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Decide whether to download: yes if no binary is cached, or the release is
/// strictly newer than the cached record. Split out so the rule is unit-tested.
pub(crate) fn should_download(binary_exists: bool, cached_ts: &str, release_ts: &str) -> bool {
    if !binary_exists {
        return true;
    }
    crate::connector_version::is_newer(release_ts, cached_ts)
}

/// Result of ensuring a connector binary is available.
#[derive(Debug)]
pub enum EnsureResult {
    /// Binary was already cached and up-to-date.
    AlreadyCurrent(PathBuf),
    /// Binary was downloaded (or updated) successfully.
    Downloaded(PathBuf),
    /// Download failed but a stale cached binary exists.
    FallbackStale(PathBuf, String),
    /// No binary available (download failed, no cache).
    Unavailable(String),
}

impl EnsureResult {
    /// Returns the path to the binary if one is available (current, downloaded, or stale).
    pub fn path(&self) -> Option<&PathBuf> {
        match self {
            Self::AlreadyCurrent(p) | Self::Downloaded(p) | Self::FallbackStale(p, _) => Some(p),
            Self::Unavailable(_) => None,
        }
    }
}

/// Returns the cache directory for connector binaries.
///
/// Resolves to `~/.strike48/strikehub/bin/` (or platform equivalent via `dirs`).
pub fn bin_cache_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".strike48")
        .join("strikehub")
        .join("bin")
}

/// Ensure a single connector binary is available and up-to-date.
///
/// Checks the latest GitHub release, compares with the cached version, downloads
/// if needed, and verifies the SHA256 checksum.
#[tracing::instrument(
    name = "connector.fetch",
    skip(client),
    fields(
        connector.id = %manifest.id,
        outcome = tracing::field::Empty,
    )
)]
pub async fn ensure_connector_binary(
    manifest: &ConnectorManifest,
    client: &reqwest::Client,
) -> EnsureResult {
    let result = ensure_connector_binary_inner(manifest, client).await;
    let outcome = match &result {
        EnsureResult::Downloaded(_) => "downloaded",
        EnsureResult::AlreadyCurrent(_) => "cached",
        EnsureResult::FallbackStale(_, _) => "fallback_stale",
        EnsureResult::Unavailable(_) => "unavailable",
    };
    tracing::Span::current().record("outcome", outcome);
    result
}

async fn ensure_connector_binary_inner(
    manifest: &ConnectorManifest,
    client: &reqwest::Client,
) -> EnsureResult {
    let Some(ref repo) = manifest.github_repo else {
        return EnsureResult::Unavailable("no github_repo configured".into());
    };
    let repo = repo.as_ref();
    let Some(ref binary_hint) = manifest.binary_hint else {
        return EnsureResult::Unavailable("no binary_hint configured".into());
    };
    let binary_name = binary_hint.as_ref();
    let Some(asset_name) = manifest.asset_name() else {
        return EnsureResult::Unavailable("no asset_pattern configured".into());
    };

    // Defense-in-depth: check the allowlist before any network request for
    // non-builtin connectors (builtins are compiled in and always trusted).
    if !manifest.is_builtin {
        let allowlist = crate::allowlist::get_allowlist();
        if !allowlist.is_allowed(repo) {
            return EnsureResult::Unavailable(format!("repo '{}' is not in the allowlist", repo));
        }
    }

    let cache_dir = bin_cache_dir();
    let binary_filename = if cfg!(target_os = "windows") {
        format!("{}.exe", binary_name)
    } else {
        binary_name.to_string()
    };
    let binary_path = cache_dir.join(&binary_filename);
    let version_path = cache_dir.join(format!("{}.version", binary_name));

    // Fetch latest release tag + publish time from GitHub
    let (latest_tag, latest_ts) = match fetch_latest_release(client, repo).await {
        Ok(pair) => pair,
        Err(e) => {
            let msg = format!("failed to fetch latest release for {}: {}", repo, e);
            tracing::warn!("{}", msg);
            if binary_path.exists() {
                return EnsureResult::FallbackStale(binary_path, msg);
            }
            return EnsureResult::Unavailable(msg);
        }
    };

    // Skip the download unless the release is strictly newer than what's cached.
    let cached = crate::connector_version::read_version_file(&version_path);
    if !should_download(binary_path.exists(), &cached.ts, &latest_ts) {
        tracing::debug!(
            "connector '{}' cache is current (cached ts {}, release ts {})",
            manifest.id,
            cached.ts,
            latest_ts
        );
        return EnsureResult::AlreadyCurrent(binary_path);
    }

    // Download the asset
    let download_url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        repo, latest_tag, asset_name
    );
    tracing::info!(
        "downloading connector '{}' {} from {}",
        manifest.id,
        latest_tag,
        download_url
    );

    let asset_bytes = match download_asset(client, &download_url).await {
        Ok(bytes) => bytes,
        Err(e) => {
            let msg = format!("failed to download {}: {}", download_url, e);
            tracing::warn!("{}", msg);
            if binary_path.exists() {
                return EnsureResult::FallbackStale(binary_path, msg);
            }
            return EnsureResult::Unavailable(msg);
        }
    };

    // Verify SHA256 checksum — fail closed for every connector, builtin or
    // dynamic. The cache dir this binary is written into (and later executed
    // from) is user-writable, so a download we cannot positively verify must
    // never reach it. A missing checksum file is therefore a refusal, not a
    // graceful skip: the release must publish SHA256SUMS.txt or an
    // {asset}.sha256 sidecar for this platform's asset.
    let checksum = verify_checksum(client, repo, &latest_tag, &asset_name, &asset_bytes).await;
    if !install_allowed(&checksum) {
        let msg = match &checksum {
            ChecksumResult::Failed(e) => {
                format!("checksum verification failed for {}: {}", asset_name, e)
            }
            ChecksumResult::NotFound => format!(
                "no checksum file found for connector '{}' (asset {}, release {}) — refusing to install unverified binary",
                manifest.id, asset_name, latest_tag
            ),
            ChecksumResult::Verified => unreachable!("install_allowed(Verified) is true"),
        };
        tracing::warn!("{}", msg);
        if binary_path.exists() {
            return EnsureResult::FallbackStale(binary_path, msg);
        }
        return EnsureResult::Unavailable(msg);
    }

    // Ensure cache directory exists
    if let Err(e) = std::fs::create_dir_all(&cache_dir) {
        let msg = format!("failed to create cache dir {}: {}", cache_dir.display(), e);
        tracing::error!("{}", msg);
        return EnsureResult::Unavailable(msg);
    }

    // Extract the binary
    if let Err(e) = extract_archive(&asset_bytes, &cache_dir, binary_name, &asset_name) {
        let msg = format!("failed to extract {}: {}", asset_name, e);
        tracing::error!("{}", msg);
        if binary_path.exists() {
            return EnsureResult::FallbackStale(binary_path, msg);
        }
        return EnsureResult::Unavailable(msg);
    }

    // Set executable permission on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) =
            std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
        {
            tracing::warn!("failed to set executable permission: {}", e);
        }
    }

    // On macOS, remove the quarantine extended attribute and ad-hoc
    // codesign the binary so Gatekeeper/XProtect don't block execution.
    // Without this, macOS treats downloaded binaries as untrusted and may
    // move them to trash with a "Malware Blocked" dialog.
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("xattr")
            .args(["-d", "com.apple.quarantine"])
            .arg(&binary_path)
            .output();
        match std::process::Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(&binary_path)
            .output()
        {
            Ok(output) if output.status.success() => {
                tracing::debug!("ad-hoc codesigned {}", binary_path.display());
            }
            Ok(output) => {
                tracing::warn!(
                    "codesign failed for {}: {}",
                    binary_path.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            Err(e) => {
                tracing::warn!("failed to run codesign: {}", e);
            }
        }
    }

    // Write version record (JSON: ref + source/publish timestamp).
    let record = crate::connector_version::ConnectorVersion::new(latest_tag.clone(), latest_ts);
    if let Err(e) = crate::connector_version::write_version_file(&version_path, &record) {
        tracing::warn!("failed to write version file: {}", e);
    }

    tracing::info!(
        "connector '{}' updated to version {}",
        manifest.id,
        latest_tag
    );

    EnsureResult::Downloaded(binary_path)
}

/// Ensure all connector binaries with GitHub repos are fetched.
///
/// Downloads are performed in parallel using `join_all`.
pub async fn ensure_all_connector_binaries(
    manifests: &[ConnectorManifest],
) -> Vec<(String, EnsureResult)> {
    let client = reqwest::Client::builder()
        .user_agent("strikehub/0.1")
        .build()
        .unwrap_or_default();

    let futures: Vec<_> = manifests
        .iter()
        .filter(|m| m.github_repo.is_some())
        .map(|manifest| {
            let client = client.clone();
            let id = manifest.id.clone().into_owned();
            async move {
                let result = ensure_connector_binary(manifest, &client).await;
                (id, result)
            }
        })
        .collect();

    futures::future::join_all(futures).await
}

/// Fetch the latest release tag name and publish timestamp from a GitHub repo.
async fn fetch_latest_release(
    client: &reqwest::Client,
    repo: &str,
) -> Result<(String, String), String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);

    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API returned {}", resp.status()));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("failed to parse response: {}", e))?;

    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "no tag_name in release response".to_string())?;
    // published_at is ISO-8601 UTC (e.g. 2026-08-06T14:03:22Z). Fall back to the
    // empty string (epoch-0) if absent so a cached copy with a real ts wins.
    let published_at = json
        .get("published_at")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok((tag, published_at))
}

/// Download an asset from a URL, following redirects.
async fn download_asset(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("download returned {}", resp.status()));
    }

    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("failed to read body: {}", e))
}

/// Three-state checksum verification result.
#[derive(Debug)]
pub(crate) enum ChecksumResult {
    /// Checksum matched successfully.
    Verified,
    /// Checksum file was found but the hash did not match.
    Failed(String),
    /// No checksum file was found in the release.
    NotFound,
}

/// Install policy: a connector binary is installed only when its integrity
/// has been **positively verified** against a published checksum.
///
/// A missing checksum (`NotFound`) refuses installation for every connector,
/// builtin or dynamic. The older behaviour let builtins "gracefully skip"
/// missing checksums, which shipped unverified binaries into the user-writable
/// cache dir in production (the kubestudio release publishes no checksum for
/// the `ks-connector-*` archives StrikeHub fetches).
///
/// Pure so the refusal paths are unit-testable without a network.
pub(crate) fn install_allowed(checksum: &ChecksumResult) -> bool {
    matches!(checksum, ChecksumResult::Verified)
}

/// Pure checksum matching: given the checksum-file texts fetched from the
/// release (either may be absent), decide the verification outcome for
/// `asset_name` against its actual SHA256.
///
/// Strategies, in order:
/// 1. `SHA256SUMS.txt` in the same release (pick style) — a line whose
///    filename equals `asset_name` decides; a mismatch is a hard `Failed`.
/// 2. `{asset_name}.sha256` sidecar file (kubestudio style).
///
/// Returns `Verified`, `Failed`, or `NotFound`. Split from
/// [`verify_checksum`] so the refusal paths are unit-testable without a
/// network.
pub(crate) fn match_checksums(
    sums_text: Option<&str>,
    sidecar_text: Option<&str>,
    asset_name: &str,
    actual_hash: &str,
) -> ChecksumResult {
    // Strategy 1: SHA256SUMS.txt
    if let Some(sums_text) = sums_text {
        for line in sums_text.lines() {
            // Format: "hash  filename" or "hash filename"
            let parts: Vec<&str> = line.splitn(2, |c: char| c.is_whitespace()).collect();
            if parts.len() == 2 {
                let expected_hash = parts[0];
                let filename = parts[1].trim().trim_start_matches('*');
                if filename == asset_name {
                    if actual_hash == expected_hash {
                        return ChecksumResult::Verified;
                    }
                    return ChecksumResult::Failed(format!(
                        "SHA256 mismatch: expected {}, got {}",
                        expected_hash, actual_hash
                    ));
                }
            }
        }
    }

    // Strategy 2: .sha256 sidecar
    if let Some(sidecar_text) = sidecar_text {
        let expected_hash = sidecar_text.split_whitespace().next().unwrap_or("");
        if !expected_hash.is_empty() {
            if actual_hash == expected_hash {
                return ChecksumResult::Verified;
            }
            return ChecksumResult::Failed(format!(
                "SHA256 mismatch: expected {}, got {}",
                expected_hash, actual_hash
            ));
        }
    }

    ChecksumResult::NotFound
}

/// Verify SHA256 checksum of downloaded asset.
///
/// Fetches `SHA256SUMS.txt` and/or the `{asset_name}.sha256` sidecar from the
/// same release, then delegates the matching to [`match_checksums`]. Returns
/// `Verified`, `Failed`, or `NotFound`.
async fn verify_checksum(
    client: &reqwest::Client,
    repo: &str,
    tag: &str,
    asset_name: &str,
    asset_bytes: &[u8],
) -> ChecksumResult {
    let actual_hash = hex_sha256(asset_bytes);

    // Strategy 1: SHA256SUMS.txt
    let sums_url = format!(
        "https://github.com/{}/releases/download/{}/SHA256SUMS.txt",
        repo, tag
    );
    let sums_text = if let Ok(sums_resp) = client.get(&sums_url).send().await
        && sums_resp.status().is_success()
        && let Ok(text) = sums_resp.text().await
    {
        Some(text)
    } else {
        None
    };

    // Strategy 2: .sha256 sidecar
    let sidecar_url = format!(
        "https://github.com/{}/releases/download/{}/{}.sha256",
        repo, tag, asset_name
    );
    let sidecar_text = if let Ok(sidecar_resp) = client.get(&sidecar_url).send().await
        && sidecar_resp.status().is_success()
        && let Ok(text) = sidecar_resp.text().await
    {
        Some(text)
    } else {
        None
    };

    let result = match_checksums(
        sums_text.as_deref(),
        sidecar_text.as_deref(),
        asset_name,
        &actual_hash,
    );
    match &result {
        ChecksumResult::Verified => tracing::debug!("SHA256 verified for {}", asset_name),
        ChecksumResult::Failed(e) => tracing::warn!("{}", e),
        ChecksumResult::NotFound => {
            tracing::debug!("no checksum file found for {}", asset_name)
        }
    }
    result
}

/// Extract a tar.gz archive's WHOLE bundle into `dest_dir`.
///
/// The bundle is the binary's directory inside the archive: everything in
/// the same directory as the entry whose file name matches `binary_name`
/// (the binary may sit at the archive root or be nested in a directory).
/// For the post-pick#553 Linux release shape this is the binary, its
/// sibling `lib/` (the `$ORIGIN/lib` rpath target — dropping it is the bug
/// behind issue #108), and the license notices; the relative layout is
/// preserved, flattened onto `dest_dir`'s root, so the binary lands exactly
/// where the caller execs it and `lib/` lands exactly where `$ORIGIN/lib`
/// resolves.
///
/// Extraction is atomic with respect to `dest_dir`: everything is staged
/// into a sibling temp dir first, then moved into place with per-entry
/// renames ([`install_staged_bundle`]); `dest_dir` never exposes a
/// half-installed bundle.
fn extract_tar_gz(
    archive_bytes: &[u8],
    dest_dir: &std::path::Path,
    binary_name: &str,
) -> Result<(), String> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    // Pass 1: locate the binary entry and capture its directory (the bundle
    // prefix). Only regular files qualify — a directory that merely shares
    // the binary's name is not a binary (the old code died reading it).
    let mut archive = Archive::new(GzDecoder::new(archive_bytes));
    let mut prefix: Option<std::path::PathBuf> = None;
    for entry in archive
        .entries()
        .map_err(|e| format!("failed to read archive entries: {}", e))?
    {
        let entry = entry.map_err(|e| format!("failed to read entry: {}", e))?;
        let path = entry
            .path()
            .map_err(|e| format!("refusing unsafe archive entry: {}", e))?;
        let path = path.as_ref();
        let is_file = entry.header().entry_type() == tar::EntryType::Regular;
        if is_file
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == binary_name)
        {
            prefix = Some(
                path.parent()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_default(),
            );
            break;
        }
    }
    let Some(prefix) = prefix else {
        return Err(format!("binary '{}' not found in archive", binary_name));
    };

    let stage = stage_dir_for(dest_dir)?;
    if let Err(e) = std::fs::create_dir_all(&stage) {
        return Err(format!(
            "failed to create staging dir {}: {}",
            stage.display(),
            e
        ));
    }
    let result = extract_tar_gz_into(&stage, archive_bytes, &prefix)
        .and_then(|()| install_staged_bundle(dest_dir, &stage, binary_name));
    cleanup_stage(&stage, &result);
    result
}

/// Pass 2 of [`extract_tar_gz`]: extract the bundle entries (everything
/// under the binary's directory prefix, relative layout preserved) into a
/// fresh staging dir. Exec bits, symlinks, and hard links are preserved;
/// setuid/setgid/sticky bits are dropped — the bundle lands in a
/// user-writable cache dir, never a setuid farm.
fn extract_tar_gz_into(
    stage: &std::path::Path,
    archive_bytes: &[u8],
    prefix: &std::path::Path,
) -> Result<(), String> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let mut archive = Archive::new(GzDecoder::new(archive_bytes));
    for entry in archive
        .entries()
        .map_err(|e| format!("failed to read archive entries: {}", e))?
    {
        let mut entry = entry.map_err(|e| format!("failed to read entry: {}", e))?;
        let path = entry
            .path()
            .map_err(|e| format!("refusing unsafe archive entry: {}", e))?;
        let path = path.as_ref();
        if !path_within(path, prefix) {
            continue; // outside the binary's bundle — not installed
        }
        let rel = path.strip_prefix(prefix).unwrap_or(path);
        if rel.as_os_str().is_empty() {
            continue; // the bundle prefix directory itself
        }
        let target = stage.join(rel);
        let mode =
            entry.header().mode().map_err(|e| {
                format!("failed to read entry mode for {}: {}", target.display(), e)
            })? & 0o7777;
        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(&target)
                    .map_err(|e| format!("failed to create {}: {}", target.display(), e))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ =
                        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode));
                }
            }
            tar::EntryType::Regular => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        format!(
                            "failed to create {} for {}: {}",
                            parent.display(),
                            target.display(),
                            e
                        )
                    })?;
                }
                let _ = std::fs::remove_file(&target);
                entry
                    .unpack(&target)
                    .map_err(|e| format!("failed to extract {}: {}", target.display(), e))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    // unpack applies the header mode; set it explicitly so the
                    // executable bits the rpath bundle relies on are exact.
                    let _ =
                        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode));
                }
            }
            tar::EntryType::Symlink => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
                }
                let _ = std::fs::remove_file(&target); // remove_file clears symlinks too
                #[cfg(unix)]
                {
                    let link = entry
                        .link_name()
                        .map_err(|e| format!("failed to read link target: {}", e))?
                        .ok_or_else(|| {
                            format!("symlink entry '{}' has no target", path.display())
                        })?;
                    std::os::unix::fs::symlink(&link, &target).map_err(|e| {
                        format!("failed to create symlink {}: {}", target.display(), e)
                    })?;
                }
                #[cfg(not(unix))]
                {
                    return Err(format!(
                        "archive entry '{}' is a symlink, which is not supported on this platform",
                        path.display()
                    ));
                }
            }
            tar::EntryType::Link => {
                // Hard link: the stored target is the referent's archive
                // path. It must be inside the bundle (mapped through the same
                // prefix strip) and staged already, or the bundle is
                // incoherent — refuse rather than stage a dangling link.
                let raw = entry
                    .link_name()
                    .map_err(|e| format!("failed to read link target: {}", e))?
                    .ok_or_else(|| format!("hardlink entry '{}' has no target", path.display()))?;
                let referent = raw.into_owned();
                if !path_within(&referent, prefix) {
                    return Err(format!(
                        "hardlink '{}' targets outside the bundle",
                        path.display()
                    ));
                }
                let link_to = stage.join(
                    referent
                        .strip_prefix(prefix)
                        .unwrap_or_else(|_| referent.as_ref()),
                );
                if !link_to.exists() {
                    return Err(format!(
                        "hardlink '{}' target not staged: {}",
                        path.display(),
                        link_to.display()
                    ));
                }
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
                }
                let _ = std::fs::remove_file(&target);
                #[cfg(unix)]
                std::fs::hard_link(&link_to, &target).map_err(|e| {
                    format!("failed to create hardlink {}: {}", target.display(), e)
                })?;
                #[cfg(not(unix))]
                {
                    return Err(format!(
                        "archive entry '{}' is a hardlink, which is not supported on this platform",
                        path.display()
                    ));
                }
            }
            _ => {
                return Err(format!(
                    "unsupported archive entry type for {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

/// True when `p` is strictly below `prefix` (component-wise, so
/// `foo-bar/x` is not under `foo`). An empty prefix (archive root) contains
/// everything.
fn path_within(p: &std::path::Path, prefix: &std::path::Path) -> bool {
    if prefix.components().count() == 0 {
        return true;
    }
    let p_comps: Vec<_> = p.components().collect();
    let f_comps: Vec<_> = prefix.components().collect();
    p_comps.len() > f_comps.len() && p_comps[..f_comps.len()] == f_comps[..]
}

/// True when the '/'-separated zip entry `name` lies strictly below the
/// '/'-separated `prefix` (see [`path_within`]).
fn zip_str_within(name: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    name.len() > prefix.len()
        && name.starts_with(prefix)
        && name.as_bytes().get(prefix.len()) == Some(&b'/')
}

/// Staging dir for a bundle install: a hidden sibling of `dest_dir` (same
/// filesystem, so the final renames are atomic).
fn stage_dir_for(dest_dir: &std::path::Path) -> Result<PathBuf, String> {
    let parent = dest_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| {
            format!(
                "cannot stage beside {}: no parent directory",
                dest_dir.display()
            )
        })?;
    let seq = STAGE_SEQ.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(".strikehub-stage-{}-{}", std::process::id(), seq)))
}

/// Move a staged bundle into `dest_dir` with per-entry renames.
///
/// Directories first (removing stale contents, so a newer release that drops
/// a lib file never leaves the old one behind), then the remaining files,
/// and the binary LAST — via [`write_binary_atomic`], keeping its temp-file
/// + read-back verification.
///
/// The binary is therefore never visible in `dest_dir` before its `lib/` is
/// in place, and a crash at any point leaves the version file (written by
/// the caller only after this succeeds) stale, which forces a re-extract on
/// the next run: no half-installed state survives a restart.
fn install_staged_bundle(
    dest_dir: &std::path::Path,
    stage: &std::path::Path,
    binary_name: &str,
) -> Result<(), String> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Kind {
        Dir,
        File,
        Binary,
    }
    let mut items: Vec<(PathBuf, Kind)> = std::fs::read_dir(stage)
        .map_err(|e| format!("failed to read staging dir {}: {}", stage.display(), e))?
        .flatten()
        .map(|e| {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let kind = if e.file_type().is_ok_and(|t| t.is_dir()) {
                Kind::Dir
            } else if name == binary_name || name == format!("{binary_name}.exe") {
                Kind::Binary
            } else {
                Kind::File
            };
            (p, kind)
        })
        .collect();
    if items.is_empty() {
        return Err(format!(
            "staging dir {} is empty — bundle not installed",
            stage.display()
        ));
    }
    // Dir < File < Binary: the binary swaps in last, onto a fully staged bundle.
    items.sort_by_key(|(p, k)| {
        (
            match k {
                Kind::Dir => 0u8,
                Kind::File => 1,
                Kind::Binary => 2,
            },
            p.clone(),
        )
    });
    for (p, kind) in &items {
        let name = p
            .file_name()
            .ok_or_else(|| format!("invalid staging entry: {}", p.display()))?;
        let target = dest_dir.join(name);
        if *kind == Kind::Dir {
            // Replace wholesale: stale files from a previous release of this
            // bundle must not survive an update.
            if target.exists() {
                std::fs::remove_dir_all(&target)
                    .map_err(|e| format!("failed to remove stale {}: {}", target.display(), e))?;
            }
            std::fs::rename(p, &target).map_err(|e| {
                format!(
                    "failed to move {} into place at {}: {}",
                    p.display(),
                    target.display(),
                    e
                )
            })?;
        } else if *kind == Kind::Binary {
            // The executable gets the same temp-file + read-back treatment
            // the single-binary path always had. write_binary_atomic stages
            // fresh bytes (default file mode), so the archive's mode —
            // already masked to 0o7777 during staging — is reapplied after
            // the rename.
            let bytes = std::fs::read(p)
                .map_err(|e| format!("failed to read staged binary {}: {}", p.display(), e))?;
            #[cfg(unix)]
            let staged_perms = std::fs::metadata(p).map(|m| m.permissions()).ok();
            write_binary_atomic(&target, &bytes)?;
            #[cfg(unix)]
            if let Some(perms) = staged_perms {
                let _ = std::fs::set_permissions(&target, perms);
            }
        } else {
            std::fs::rename(p, &target).map_err(|e| {
                format!(
                    "failed to move {} into place at {}: {}",
                    p.display(),
                    target.display(),
                    e
                )
            })?;
        }
    }
    Ok(())
}

/// Remove a staging dir after an install attempt; warn (not fail) if a
/// successful install still left it behind.
fn cleanup_stage(stage: &std::path::Path, result: &Result<(), String>) {
    if let Err(e) = std::fs::remove_dir_all(stage)
        && result.is_ok()
    {
        tracing::warn!("left staging dir {}: {}", stage.display(), e);
    }
}

/// Extract a zip archive's WHOLE bundle into `dest_dir` — the mirror of
/// [`extract_tar_gz`] for the Windows asset path. Same rules: the bundle is
/// the binary's directory (binary matched by base name or `{name}.exe`),
/// layout preserved, staged then atomically swapped in.
///
/// Note on the current assets: today's pick Windows release zip contains
/// just the statically-linked `.exe` (its `check-windows-crt.ps1` gate
/// proves no VC++ runtime side-by-side is needed), so there are no siblings
/// to preserve in practice yet — but the extractor must not silently drop
/// them if a release ever bundles DLLs beside the exe.
fn extract_zip(
    archive_bytes: &[u8],
    dest_dir: &std::path::Path,
    binary_name: &str,
) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(archive_bytes))
        .map_err(|e| format!("failed to read zip archive: {}", e))?;

    let exe_name = format!("{}.exe", binary_name);

    // Pass 1: locate the binary entry and its directory prefix.
    let mut prefix: Option<String> = None;
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| format!("failed to read zip entry: {}", e))?;
        if file.is_dir() || file.is_symlink() {
            continue;
        }
        let name = zip_entry_path(file.name())?;
        let file_name = name.rsplit('/').next().unwrap_or_default();
        if file_name == binary_name || file_name == exe_name {
            prefix = Some(match name.rfind('/') {
                Some(pos) => name[..pos].to_string(),
                None => String::new(),
            });
            break;
        }
    }
    let Some(prefix) = prefix else {
        return Err(format!("binary '{}' not found in zip archive", binary_name));
    };

    let stage = stage_dir_for(dest_dir)?;
    if let Err(e) = std::fs::create_dir_all(&stage) {
        return Err(format!(
            "failed to create staging dir {}: {}",
            stage.display(),
            e
        ));
    }
    let result = extract_zip_into(&stage, &mut archive, &prefix)
        .and_then(|()| install_staged_bundle(dest_dir, &stage, binary_name));
    cleanup_stage(&stage, &result);
    result
}

/// Pass 2 of [`extract_zip`]: extract the bundle entries into a fresh
/// staging dir, preserving the stored Unix mode when present and symlink
/// entries (zip stores the target path as the entry's content).
fn extract_zip_into(
    stage: &std::path::Path,
    archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    prefix: &str,
) -> Result<(), String> {
    use std::io::Read;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("failed to read zip entry: {}", e))?;
        if file.is_dir() {
            continue; // parent dirs are created implicitly below
        }
        let name = zip_entry_path(file.name())?;
        if !zip_str_within(&name, prefix) {
            continue; // outside the binary's bundle — not installed
        }
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            name[prefix.len() + 1..].to_string()
        };
        let mut target = PathBuf::new();
        for part in rel.split('/') {
            if !part.is_empty() {
                target.push(part);
            }
        }
        if target.as_os_str().is_empty() {
            continue;
        }
        target = stage.join(target);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
        }
        let _ = std::fs::remove_file(&target);
        if file.is_symlink() {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .map_err(|e| format!("failed to read entry {}: {}", name, e))?;
            let link = std::ffi::OsString::from(String::from_utf8_lossy(&buf).into_owned());
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link, &target)
                .map_err(|e| format!("failed to create symlink {}: {}", target.display(), e))?;
            #[cfg(not(unix))]
            {
                return Err(format!(
                    "zip entry '{}' is a symlink, which is not supported on this platform",
                    name
                ));
            }
        } else {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .map_err(|e| format!("failed to read entry {}: {}", name, e))?;
            std::fs::write(&target, &buf)
                .map_err(|e| format!("failed to write {}: {}", target.display(), e))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // The zip's external attributes carry the Unix mode when the
                // archive was made on Unix; drop setuid/setgid/sticky.
                if let Some(mode) = file.unix_mode() {
                    let _ = std::fs::set_permissions(
                        &target,
                        std::fs::Permissions::from_mode(mode & 0o7777),
                    );
                }
            }
        }
    }
    Ok(())
}

/// Normalize and validate a zip entry name: backslash → '/', then refuse
/// absolute paths and `..` components (fail closed, same policy as the tar
/// path's `entry.path()` rejection).
fn zip_entry_path(name: &str) -> Result<String, String> {
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(format!("refusing absolute zip entry: {name}"));
    }
    for comp in normalized.split('/') {
        if comp == ".." {
            return Err(format!("refusing zip entry with '..' component: {name}"));
        }
    }
    Ok(normalized)
}

/// Dispatch archive extraction based on the asset filename extension.
fn extract_archive(
    archive_bytes: &[u8],
    dest_dir: &std::path::Path,
    binary_name: &str,
    asset_name: &str,
) -> Result<(), String> {
    if asset_name.ends_with(".zip") {
        extract_zip(archive_bytes, dest_dir, binary_name)
    } else {
        extract_tar_gz(archive_bytes, dest_dir, binary_name)
    }
}

/// Compute the hex-encoded SHA256 hash of a byte slice.
pub fn hex_sha256(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(data);
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Write `buf` to `dest` atomically and verify the staged bytes.
///
/// Stages to a sibling temp file in the same directory, reads it back and
/// compares it byte-for-byte against `buf`, then renames it over `dest`.
/// The rename is atomic on NTFS/ext4/APFS, so the cache dir never exposes a
/// torn or partially-written binary: a crash mid-install leaves either the
/// previous binary or the new one, never a truncated file that a later
/// "cache is current" check would happily execute.
fn write_binary_atomic(dest: &std::path::Path, buf: &[u8]) -> Result<(), String> {
    let file_name = dest
        .file_name()
        .ok_or_else(|| format!("invalid destination path: {}", dest.display()))?;
    let tmp = dest.with_file_name(format!(
        ".{}.strikehub-tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));

    if let Err(e) = std::fs::write(&tmp, buf) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "failed to write temp binary {}: {}",
            tmp.display(),
            e
        ));
    }

    // Read-back verification: the staged bytes must be exactly what was
    // extracted, or the install is refused and the temp file removed.
    let staged = std::fs::read(&tmp).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("failed to read back {}: {}", tmp.display(), e)
    })?;
    if staged != buf {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "read-back verification failed for {} (staged bytes differ from extracted archive)",
            tmp.display()
        ));
    }

    if let Err(e) = std::fs::rename(&tmp, dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "failed to move {} into place at {}: {}",
            tmp.display(),
            dest.display(),
            e
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_sha256() {
        let hash = hex_sha256(b"hello world");
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_bin_cache_dir() {
        let dir = bin_cache_dir();
        assert!(dir.ends_with("bin"));
        assert!(dir.to_string_lossy().contains(".strike48"));
    }

    #[test]
    fn test_extract_tar_gz() {
        use flate2::Compression;
        use flate2::write::GzEncoder;

        // Create a tar.gz with a fake binary
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = tar::Builder::new(&mut encoder);
            let content = b"#!/bin/sh\necho hello";
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "test-binary", &content[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let archive_bytes = encoder.finish().unwrap();

        let tmp_dir = std::env::temp_dir().join("strikehub-test-extract");
        let _ = std::fs::create_dir_all(&tmp_dir);

        let result = extract_tar_gz(&archive_bytes, &tmp_dir, "test-binary");
        assert!(result.is_ok(), "extract failed: {:?}", result);

        let extracted = tmp_dir.join("test-binary");
        assert!(extracted.exists());
        let content = std::fs::read(&extracted).unwrap();
        assert_eq!(content, b"#!/bin/sh\necho hello");

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_zip() {
        use std::io::Write;

        let content = b"#!/bin/sh\necho hello";

        // Create a zip archive in memory
        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut zip_writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip_writer.start_file("test-binary", options).unwrap();
        zip_writer.write_all(content).unwrap();
        let cursor = zip_writer.finish().unwrap();
        let archive_bytes = cursor.into_inner();

        let tmp_dir = std::env::temp_dir().join("strikehub-test-extract-zip");
        let _ = std::fs::create_dir_all(&tmp_dir);

        let result = extract_zip(&archive_bytes, &tmp_dir, "test-binary");
        assert!(result.is_ok(), "extract_zip failed: {:?}", result);

        let extracted = tmp_dir.join("test-binary");
        assert!(extracted.exists());
        let extracted_content = std::fs::read(&extracted).unwrap();
        assert_eq!(extracted_content, content);

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_zip_nested() {
        use std::io::Write;

        let content = b"nested binary content";

        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut zip_writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip_writer
            .start_file("subdir/test-binary", options)
            .unwrap();
        zip_writer.write_all(content).unwrap();
        let cursor = zip_writer.finish().unwrap();
        let archive_bytes = cursor.into_inner();

        let tmp_dir = std::env::temp_dir().join("strikehub-test-extract-zip-nested");
        let _ = std::fs::create_dir_all(&tmp_dir);

        let result = extract_zip(&archive_bytes, &tmp_dir, "test-binary");
        assert!(result.is_ok(), "extract_zip nested failed: {:?}", result);

        let extracted = tmp_dir.join("test-binary");
        assert!(extracted.exists());
        let extracted_content = std::fs::read(&extracted).unwrap();
        assert_eq!(extracted_content, content);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_archive_dispatches_zip() {
        use std::io::Write;

        let content = b"zip dispatch test";

        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut zip_writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip_writer.start_file("my-binary", options).unwrap();
        zip_writer.write_all(content).unwrap();
        let cursor = zip_writer.finish().unwrap();
        let archive_bytes = cursor.into_inner();

        let tmp_dir = std::env::temp_dir().join("strikehub-test-extract-archive-zip");
        let _ = std::fs::create_dir_all(&tmp_dir);

        let result = extract_archive(&archive_bytes, &tmp_dir, "my-binary", "my-binary.zip");
        assert!(result.is_ok(), "extract_archive zip failed: {:?}", result);

        let extracted = tmp_dir.join("my-binary");
        assert!(extracted.exists());
        assert_eq!(std::fs::read(&extracted).unwrap(), content);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_archive_dispatches_tar_gz() {
        use flate2::Compression;
        use flate2::write::GzEncoder;

        let content = b"tar dispatch test";

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = tar::Builder::new(&mut encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "my-binary", &content[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let archive_bytes = encoder.finish().unwrap();

        let tmp_dir = std::env::temp_dir().join("strikehub-test-extract-archive-targz");
        let _ = std::fs::create_dir_all(&tmp_dir);

        let result = extract_archive(&archive_bytes, &tmp_dir, "my-binary", "my-binary.tar.gz");
        assert!(
            result.is_ok(),
            "extract_archive tar.gz failed: {:?}",
            result
        );

        let extracted = tmp_dir.join("my-binary");
        assert!(extracted.exists());
        assert_eq!(std::fs::read(&extracted).unwrap(), content);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_platform_helpers() {
        use crate::registry::{platform_arch, platform_archive_ext, platform_os};

        let os = platform_os();
        let arch = platform_arch();
        let ext = platform_archive_ext();

        // Just verify they return non-empty strings
        assert!(!os.is_empty());
        assert!(!arch.is_empty());
        assert!(!ext.is_empty());

        // On macOS, should be "darwin"
        #[cfg(target_os = "macos")]
        assert_eq!(os, "darwin");

        #[cfg(target_os = "linux")]
        assert_eq!(os, "linux");

        #[cfg(not(target_os = "windows"))]
        assert_eq!(ext, "tar.gz");

        #[cfg(target_os = "windows")]
        assert_eq!(ext, "zip");
    }

    #[test]
    fn test_asset_name_generation() {
        use crate::registry::{platform_arch, platform_archive_ext, platform_os};
        use std::borrow::Cow;

        let manifest = ConnectorManifest {
            id: Cow::Borrowed("test"),
            name: Cow::Borrowed("Test"),
            description: Cow::Borrowed("test"),
            icon: Cow::Borrowed("test"),
            default_port: 3030,
            default_transport: crate::config::ConnectorTransport::Ipc,
            binary_hint: Some(Cow::Borrowed("test-bin")),
            github_repo: Some(Cow::Borrowed("org/repo")),
            asset_pattern: Some(Cow::Borrowed("test-bin-{os}-{arch}.{ext}")),
            is_builtin: true,
        };

        let name = manifest.asset_name().unwrap();
        assert!(name.contains(platform_os()));
        assert!(name.contains(platform_arch()));
        assert!(name.contains(platform_archive_ext()));
    }

    #[test]
    fn test_asset_name_none_without_pattern() {
        use std::borrow::Cow;

        let manifest = ConnectorManifest {
            id: Cow::Borrowed("test"),
            name: Cow::Borrowed("Test"),
            description: Cow::Borrowed("test"),
            icon: Cow::Borrowed("test"),
            default_port: 3030,
            default_transport: crate::config::ConnectorTransport::Ipc,
            binary_hint: Some(Cow::Borrowed("test-bin")),
            github_repo: Some(Cow::Borrowed("org/repo")),
            asset_pattern: None,
            is_builtin: true,
        };

        assert!(manifest.asset_name().is_none());
    }
}

#[cfg(test)]
mod bundle_extract_tests {
    // Regression tests for issue #108: `extract_tar_gz` / `extract_zip`
    // must install the connector's WHOLE bundle — the binary plus its
    // sibling `lib/` (and friends) — not just the named binary. Since
    // pick#553 the Linux release tarball carries an `$ORIGIN/lib` rpath,
    // so the staged layout must keep `lib/` beside the binary for the
    // rpath to resolve on minimal hosts (NixOS, containers).
    //
    // Fixture shapes mirror the post-pick#553 release pipeline
    // (pick `.github/workflows/release.yml`, build-headless-linux):
    //
    //     pentest-agent              (binary, 0755)
    //     THIRD-PARTY-NOTICES.md     (license attribution)
    //     lib/libpcap.so.0.8         (bundled libs, flat under lib/)
    //     lib/libssl.so.3
    //     lib/libcrypto.so.3

    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// (path, content, mode, symlink-target) — a symlink entry has `None`
    /// content and `Some(target)`.
    type Fixture = (&'static str, &'static [u8], u32, Option<&'static str>);

    fn make_tar_gz(entries: &[Fixture]) -> Vec<u8> {
        use flate2::Compression;
        use flate2::write::GzEncoder;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = tar::Builder::new(&mut encoder);
            for (path, content, mode, link) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_mode(*mode);
                match link {
                    Some(target) => {
                        // GNU encoding: the link target lives in the header's
                        // linkname field; the entry carries no data.
                        header.set_entry_type(tar::EntryType::Symlink);
                        header.set_link_name(std::path::Path::new(target)).unwrap();
                        header.set_size(0);
                        header.set_cksum();
                        builder.append_data(&mut header, path, &b""[..]).unwrap();
                    }
                    None => {
                        header.set_size(content.len() as u64);
                        header.set_cksum();
                        builder
                            .append_data(&mut header, path, &content[..])
                            .unwrap();
                    }
                }
            }
            builder.finish().unwrap();
        }
        encoder.finish().unwrap()
    }

    /// Fresh unique temp dir with its OWN private parent, so the staging-
    /// leftover assertion can't see other (parallel, same-process) tests'
    /// stages: returns (dest, private_parent).
    fn fresh_dest(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let parent = std::env::temp_dir().join(format!(
            "strikehub-108-{}-{}-parent",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&parent);
        let dir = parent.join("dest");
        std::fs::create_dir_all(&dir).unwrap();
        (dir, parent)
    }

    fn assert_no_stage_leftovers(parent: &std::path::Path) {
        let leftovers: Vec<String> = std::fs::read_dir(parent)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("strikehub-stage"))
            .collect();
        assert!(leftovers.is_empty(), "staging leftovers: {leftovers:?}");
    }

    /// The post-pick#553 shape: binary + THIRD-PARTY-NOTICES.md + lib/ at the
    /// tarball root (flat), plus one nested dir under lib/ to prove the
    /// relative layout survives extraction.
    fn pick_bundle_fixture() -> Vec<Fixture> {
        vec![
            ("pentest-agent", b"#!/bin/true\nfake elf v1", 0o755, None),
            ("THIRD-PARTY-NOTICES.md", b"license notices", 0o644, None),
            ("lib/libpcap.so.0.8", b"fake libpcap bytes", 0o644, None),
            ("lib/libssl.so.3", b"fake openssl ssl", 0o644, None),
            ("lib/libcrypto.so.3", b"fake openssl crypto", 0o644, None),
            ("lib/extra/libfoo.so.1", b"fake nested lib", 0o644, None),
        ]
    }

    #[test]
    fn extract_tar_gz_keeps_lib_dir_beside_binary() {
        let archive_bytes = make_tar_gz(&pick_bundle_fixture());
        let (dest, parent) = fresh_dest("bundle-flat");

        let result = extract_tar_gz(&archive_bytes, &dest, "pentest-agent");
        assert!(result.is_ok(), "extract failed: {:?}", result);

        // The binary, at the cache-dir root as before (callers exec exactly
        // `dest/pentest-agent` and write `dest/pentest-agent.version`).
        let bin = dest.join("pentest-agent");
        assert!(bin.exists(), "binary not staged at {}", bin.display());
        assert_eq!(std::fs::read(&bin).unwrap(), b"#!/bin/true\nfake elf v1");

        // ISSUE #108: the sibling lib/ must survive, with the same relative
        // layout the binary's $ORIGIN/lib rpath expects — binary beside lib/.
        assert!(
            dest.join("lib/libpcap.so.0.8").is_file(),
            "lib/libpcap.so.0.8 missing: the rpath $ORIGIN/lib is inert without it"
        );
        assert!(dest.join("lib/libssl.so.3").is_file());
        assert!(dest.join("lib/libcrypto.so.3").is_file());
        // Nested layout under lib/ preserved, not flattened.
        assert!(dest.join("lib/extra/libfoo.so.1").is_file());
        assert_eq!(
            std::fs::read(dest.join("lib/libpcap.so.0.8")).unwrap(),
            b"fake libpcap bytes"
        );

        // The rest of the bundle (license attribution) is part of the tarball
        // and stays beside the binary too.
        assert!(dest.join("THIRD-PARTY-NOTICES.md").is_file());

        assert_no_stage_leftovers(&parent);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_tar_gz_preserves_modes_and_symlinks() {
        let entries: Vec<Fixture> = vec![
            ("pentest-agent", b"fake elf", 0o755, None),
            ("lib/libpcap.so.0.8", b"real soname file", 0o644, None),
            ("lib/libpcap.so", b"", 0o777, Some("libpcap.so.0.8")),
        ];
        let archive_bytes = make_tar_gz(&entries);
        let (dest, parent) = fresh_dest("modes");

        extract_tar_gz(&archive_bytes, &dest, "pentest-agent").unwrap();

        // Exec bit preserved from the archive header (previously the staged
        // binary only got 0o755 because the caller chmod'd it afterwards;
        // bundled files keep their own modes).
        let mode = std::fs::metadata(dest.join("pentest-agent"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "binary exec bits not preserved");
        let lib_mode = std::fs::metadata(dest.join("lib/libpcap.so.0.8"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(lib_mode & 0o777, 0o644, "lib file mode not preserved");

        // Symlink preserved as a symlink (not a copy of the target's bytes).
        let link = dest.join("lib/libpcap.so");
        assert!(link.is_symlink(), "lib/libpcap.so must stay a symlink");
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            std::path::Path::new("libpcap.so.0.8")
        );

        assert_no_stage_leftovers(&parent);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_tar_gz_wrapped_bundle_flattens_to_cache_root() {
        // Some archives wrap the bundle in a top-level directory. The binary
        // must still land at the cache-dir root (callers exec that exact
        // path), and its lib/ must come along, BESIDE it — that is what
        // $ORIGIN/lib resolves to. Unrelated root-level files stay out of the
        // shared cache dir.
        let entries: Vec<Fixture> = vec![
            (
                "pentest-agent-linux-x86_64/pentest-agent",
                b"fake elf wrapped",
                0o755,
                None,
            ),
            (
                "pentest-agent-linux-x86_64/lib/libpcap.so.0.8",
                b"fake libpcap bytes",
                0o644,
                None,
            ),
            ("README.md", b"unrelated root file", 0o644, None),
        ];
        let archive_bytes = make_tar_gz(&entries);
        let (dest, parent) = fresh_dest("wrapped");

        extract_tar_gz(&archive_bytes, &dest, "pentest-agent").unwrap();

        assert!(dest.join("pentest-agent").is_file());
        assert_eq!(
            std::fs::read(dest.join("pentest-agent")).unwrap(),
            b"fake elf wrapped"
        );
        assert!(
            dest.join("lib/libpcap.so.0.8").is_file(),
            "wrapped bundle must flatten lib/ beside the binary"
        );
        assert!(
            !dest.join("README.md").exists(),
            "files outside the binary's directory must not leak into the shared cache dir"
        );

        assert_no_stage_leftovers(&parent);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_tar_gz_update_removes_stale_bundle_files() {
        // Re-extraction (a new release) must replace the whole bundle: a lib
        // file the new release no longer ships must not linger, or the staged
        // dir drifts between versions.
        let v1 = make_tar_gz(&[
            ("pentest-agent", b"fake elf v1", 0o755, None),
            ("lib/old-lib.so.1", b"old lib", 0o644, None),
        ]);
        let (dest, parent) = fresh_dest("update");
        extract_tar_gz(&v1, &dest, "pentest-agent").unwrap();
        assert!(dest.join("lib/old-lib.so.1").is_file());

        let v2 = make_tar_gz(&[
            ("pentest-agent", b"fake elf v2", 0o755, None),
            ("lib/new-lib.so.2", b"new lib", 0o644, None),
        ]);
        extract_tar_gz(&v2, &dest, "pentest-agent").unwrap();

        assert_eq!(
            std::fs::read(dest.join("pentest-agent")).unwrap(),
            b"fake elf v2"
        );
        assert!(dest.join("lib/new-lib.so.2").is_file());
        assert!(
            !dest.join("lib/old-lib.so.1").exists(),
            "stale lib file from the previous release must be removed"
        );

        assert_no_stage_leftovers(&parent);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_tar_gz_missing_binary_refuses_and_leaves_nothing() {
        let archive_bytes = make_tar_gz(&[("some-other-tool", b"x", 0o755, None)]);
        let (dest, parent) = fresh_dest("missing");

        let err = extract_tar_gz(&archive_bytes, &dest, "pentest-agent").unwrap_err();
        assert!(
            err.contains("not found in archive"),
            "unexpected error: {err}"
        );

        // Nothing may reach the (shared, user-writable, executed-from) cache
        // dir, and no staging dir may leak in its parent.
        let staged: Vec<String> = std::fs::read_dir(&dest)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(staged.is_empty(), "nothing may be staged, got: {staged:?}");
        assert_no_stage_leftovers(&parent);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_zip_keeps_siblings_beside_binary() {
        // Mirror of the tarball rule for the Windows zip path: if a release
        // zip ever bundles the DLLs a .exe needs beside it (today's pick
        // windows asset is a single statically-linked exe, but the extractor
        // must not silently drop siblings if that changes), they must land
        // beside the binary.
        use std::io::Write;

        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut zip_writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip_writer.start_file("pentest-agent.exe", options).unwrap();
        zip_writer.write_all(b"fake exe").unwrap();
        zip_writer.start_file("wpcap.dll", options).unwrap();
        zip_writer.write_all(b"fake wpcap").unwrap();
        zip_writer.start_file("Packet.dll", options).unwrap();
        zip_writer.write_all(b"fake packet").unwrap();
        let archive_bytes = zip_writer.finish().unwrap().into_inner();

        let (dest, parent) = fresh_dest("zip-siblings");
        extract_zip(&archive_bytes, &dest, "pentest-agent").unwrap();

        assert!(dest.join("pentest-agent.exe").is_file());
        assert_eq!(
            std::fs::read(dest.join("pentest-agent.exe")).unwrap(),
            b"fake exe"
        );
        assert!(
            dest.join("wpcap.dll").is_file(),
            "zip siblings must be extracted beside the binary"
        );
        assert!(dest.join("Packet.dll").is_file());

        assert_no_stage_leftovers(&parent);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_zip_missing_binary_refuses_and_leaves_nothing() {
        use std::io::Write;

        let buf = Vec::new();
        let cursor = std::io::Cursor::new(buf);
        let mut zip_writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip_writer.start_file("other-tool.exe", options).unwrap();
        zip_writer.write_all(b"x").unwrap();
        let archive_bytes = zip_writer.finish().unwrap().into_inner();

        let (dest, parent) = fresh_dest("zip-missing");
        let err = extract_zip(&archive_bytes, &dest, "pentest-agent").unwrap_err();
        assert!(err.contains("not found in zip"), "unexpected error: {err}");

        let staged: Vec<String> = std::fs::read_dir(&dest)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(staged.is_empty(), "nothing may be staged, got: {staged:?}");
        assert_no_stage_leftovers(&parent);
        let _ = std::fs::remove_dir_all(&dest);
    }
}

#[cfg(test)]
mod newest_tests {
    use super::should_download;

    #[test]
    fn downloads_when_release_is_newer_than_cache() {
        assert!(should_download(
            true,                   // binary_exists
            "2026-08-05T00:00:00Z", // cached ts
            "2026-08-06T00:00:00Z", // release ts
        ));
    }

    #[test]
    fn skips_when_cache_is_current_or_newer() {
        assert!(!should_download(
            true,
            "2026-08-06T00:00:00Z",
            "2026-08-06T00:00:00Z"
        ));
        assert!(!should_download(
            true,
            "2026-08-07T00:00:00Z",
            "2026-08-06T00:00:00Z"
        ));
    }

    #[test]
    fn downloads_when_no_binary_cached_regardless_of_ts() {
        assert!(should_download(false, "2026-08-09T00:00:00Z", ""));
    }
}

#[cfg(test)]
mod verification_policy_tests {
    use super::{
        ChecksumResult, hex_sha256, install_allowed, match_checksums, write_binary_atomic,
    };

    /// Known SHA256 of the fixture bytes below, computed out-of-band
    /// (`printf 'pentest-agent release payload' | sha256sum`) so the expected
    /// value is independent of the code under test.
    const GOOD_HASH: &str = "d9ed2e522e219e1fdaab86305cce3650927d48b0915f3db8dabd8a4dc5cf79d1";
    const ASSET: &str = "ks-connector-windows-x86_64.zip";

    fn sums_for(hash: &str) -> String {
        format!("{}  {}\n", hash, ASSET)
    }

    #[test]
    fn tampered_bytes_are_refused() {
        // Fixture: known-good release payload bytes, then a single flipped
        // bit — the tampered-bytes case.
        let good = b"pentest-agent release payload";
        assert_eq!(hex_sha256(good), GOOD_HASH);

        let mut tampered = good.to_vec();
        tampered[0] ^= 0x01;
        let tampered_hash = hex_sha256(&tampered);
        assert_ne!(tampered_hash, GOOD_HASH);

        // The release publishes the good hash; the tampered download must be
        // a hard Failed — never Verified — and must fail the install gate.
        let sums = sums_for(GOOD_HASH);
        let verdict = match_checksums(Some(&sums), None, ASSET, &tampered_hash);
        assert!(
            matches!(verdict, ChecksumResult::Failed(_)),
            "tampered bytes must yield Failed, got {:?}",
            verdict
        );
        assert!(
            !install_allowed(&verdict),
            "tampered bytes must not be installed"
        );
    }

    #[test]
    fn good_bytes_verify_via_sums_file() {
        let sums = sums_for(GOOD_HASH);
        let verdict = match_checksums(Some(&sums), None, ASSET, GOOD_HASH);
        assert!(matches!(verdict, ChecksumResult::Verified));
        assert!(install_allowed(&verdict));
    }

    #[test]
    fn good_bytes_verify_via_sidecar() {
        let sidecar = format!("{}  {}\n", GOOD_HASH, ASSET);
        let verdict = match_checksums(None, Some(&sidecar), ASSET, GOOD_HASH);
        assert!(matches!(verdict, ChecksumResult::Verified));
        assert!(install_allowed(&verdict));
    }

    #[test]
    fn no_checksum_files_is_not_found() {
        let verdict = match_checksums(None, None, ASSET, GOOD_HASH);
        assert!(matches!(verdict, ChecksumResult::NotFound));
    }

    #[test]
    fn missing_checksum_refuses_install_for_every_connector() {
        // Regression guard for the old builtin "graceful skip": a release
        // without a checksum file (the kubestudio production case) must refuse
        // installation for builtin and dynamic connectors alike — the binary
        // is destined for a user-writable dir it will later be executed from.
        assert!(
            !install_allowed(&ChecksumResult::NotFound),
            "missing checksums must refuse installation (builtin or dynamic)"
        );
        assert!(!install_allowed(&ChecksumResult::Failed("mismatch".into())));
        assert!(install_allowed(&ChecksumResult::Verified));
    }

    #[test]
    fn atomic_write_installs_exact_bytes_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!(
            "strikehub-atomic-{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("t")
                .replace(' ', "_")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let payload = b"#!/bin/sh\necho atomic";
        let dest = dir.join("atomic-bin");
        write_binary_atomic(&dest, payload).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), payload);

        // Reinstall over the existing binary (the update path).
        write_binary_atomic(&dest, b"v2").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"v2");

        // No .strikehub-tmp-* leftovers may remain.
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("strikehub-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp leftovers: {leftovers:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Negative: an unwritable target dir must be refused (temp file removed,
    /// no partial binary left at the destination). Linux-only: the root-uid
    /// guard and the `libc` dependency are Linux-scoped in this crate.
    #[cfg(target_os = "linux")]
    #[test]
    fn atomic_write_refuses_unwritable_dir() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::getuid() } == 0 {
            return; // root bypasses mode bits; nothing to prove
        }
        let dir = std::env::temp_dir().join(format!("strikehub-atomic-ro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let err = write_binary_atomic(&dir.join("ro-bin"), b"x").unwrap_err();

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        // The destination must not exist; the temp file must be cleaned up.
        assert!(!dir.join("ro-bin").exists());
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("strikehub-tmp"))
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(leftovers.is_empty(), "temp leftovers: {leftovers:?}");
        assert!(
            err.contains("failed to write temp binary"),
            "expected a refusal error, got: {err}"
        );
    }
}
