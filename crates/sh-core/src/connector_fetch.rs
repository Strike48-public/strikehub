//! Runtime connector binary fetch from GitHub Releases.
//!
//! Downloads pre-built connector binaries, verifies SHA256 checksums, extracts
//! archives, and caches them in `~/.strike48/strikehub/bin/`.

use std::path::PathBuf;

use crate::registry::ConnectorManifest;

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
pub async fn ensure_connector_binary(
    manifest: &ConnectorManifest,
    client: &reqwest::Client,
) -> EnsureResult {
    let Some(repo) = manifest.github_repo else {
        return EnsureResult::Unavailable("no github_repo configured".into());
    };
    let Some(binary_name) = manifest.binary_hint else {
        return EnsureResult::Unavailable("no binary_hint configured".into());
    };
    let Some(asset_name) = manifest.asset_name() else {
        return EnsureResult::Unavailable("no asset_pattern configured".into());
    };

    let cache_dir = bin_cache_dir();
    let binary_filename = if cfg!(target_os = "windows") {
        format!("{}.exe", binary_name)
    } else {
        binary_name.to_string()
    };
    let binary_path = cache_dir.join(&binary_filename);
    let version_path = cache_dir.join(format!("{}.version", binary_name));

    // Fetch latest release tag from GitHub
    let latest_tag = match fetch_latest_release(client, repo).await {
        Ok(tag) => tag,
        Err(e) => {
            let msg = format!("failed to fetch latest release for {}: {}", repo, e);
            tracing::warn!("{}", msg);
            if binary_path.exists() {
                return EnsureResult::FallbackStale(binary_path, msg);
            }
            return EnsureResult::Unavailable(msg);
        }
    };

    // Check if we already have this version
    if binary_path.exists()
        && let Ok(cached_version) = std::fs::read_to_string(&version_path)
        && cached_version.trim() == latest_tag
    {
        tracing::debug!(
            "connector '{}' already at version {}",
            manifest.id,
            latest_tag
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

    // Verify SHA256 if available
    if let Err(e) = verify_checksum(client, repo, &latest_tag, &asset_name, &asset_bytes).await {
        let msg = format!("checksum verification failed for {}: {}", asset_name, e);
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

    // Write version file
    if let Err(e) = std::fs::write(&version_path, &latest_tag) {
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
            let id = manifest.id.to_string();
            async move {
                let result = ensure_connector_binary(manifest, &client).await;
                (id, result)
            }
        })
        .collect();

    futures::future::join_all(futures).await
}

/// Fetch the latest release tag name from a GitHub repo.
async fn fetch_latest_release(client: &reqwest::Client, repo: &str) -> Result<String, String> {
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

    json.get("tag_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "no tag_name in release response".into())
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

/// Verify SHA256 checksum of downloaded asset.
///
/// Attempts two strategies:
/// 1. `SHA256SUMS.txt` in the same release (pick style)
/// 2. `{asset_name}.sha256` sidecar file (kubestudio style)
///
/// If neither is available, verification is skipped (returns Ok).
async fn verify_checksum(
    client: &reqwest::Client,
    repo: &str,
    tag: &str,
    asset_name: &str,
    asset_bytes: &[u8],
) -> Result<(), String> {
    let actual_hash = hex_sha256(asset_bytes);

    // Strategy 1: SHA256SUMS.txt
    let sums_url = format!(
        "https://github.com/{}/releases/download/{}/SHA256SUMS.txt",
        repo, tag
    );
    if let Ok(sums_resp) = client.get(&sums_url).send().await
        && sums_resp.status().is_success()
        && let Ok(sums_text) = sums_resp.text().await
    {
        for line in sums_text.lines() {
            // Format: "hash  filename" or "hash filename"
            let parts: Vec<&str> = line.splitn(2, |c: char| c.is_whitespace()).collect();
            if parts.len() == 2 {
                let expected_hash = parts[0];
                let filename = parts[1].trim().trim_start_matches('*');
                if filename == asset_name {
                    if actual_hash == expected_hash {
                        tracing::debug!("SHA256 verified via SHA256SUMS.txt");
                        return Ok(());
                    }
                    return Err(format!(
                        "SHA256 mismatch: expected {}, got {}",
                        expected_hash, actual_hash
                    ));
                }
            }
        }
    }

    // Strategy 2: .sha256 sidecar
    let sidecar_url = format!(
        "https://github.com/{}/releases/download/{}/{}.sha256",
        repo, tag, asset_name
    );
    if let Ok(sidecar_resp) = client.get(&sidecar_url).send().await
        && sidecar_resp.status().is_success()
        && let Ok(sidecar_text) = sidecar_resp.text().await
    {
        let expected_hash = sidecar_text.split_whitespace().next().unwrap_or("");
        if !expected_hash.is_empty() {
            if actual_hash == expected_hash {
                tracing::debug!("SHA256 verified via .sha256 sidecar");
                return Ok(());
            }
            return Err(format!(
                "SHA256 mismatch: expected {}, got {}",
                expected_hash, actual_hash
            ));
        }
    }

    // No checksum file found — skip verification
    tracing::debug!(
        "no checksum file found for {}, skipping verification",
        asset_name
    );
    Ok(())
}

/// Extract a tar.gz archive, looking for a specific binary inside.
///
/// The binary may be at the top level or nested in a directory.
fn extract_tar_gz(
    archive_bytes: &[u8],
    dest_dir: &std::path::Path,
    binary_name: &str,
) -> Result<(), String> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    let decoder = GzDecoder::new(archive_bytes);
    let mut archive = Archive::new(decoder);

    let entries = archive
        .entries()
        .map_err(|e| format!("failed to read archive entries: {}", e))?;

    for entry in entries {
        let mut entry = entry.map_err(|e| format!("failed to read entry: {}", e))?;
        let path = entry
            .path()
            .map_err(|e| format!("failed to read entry path: {}", e))?
            .to_path_buf();

        // Match the binary by filename (may be nested in a directory)
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        if file_name == binary_name {
            let dest = dest_dir.join(binary_name);
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("failed to read binary from archive: {}", e))?;
            std::fs::write(&dest, &buf)
                .map_err(|e| format!("failed to write binary to {}: {}", dest.display(), e))?;
            return Ok(());
        }
    }

    Err(format!("binary '{}' not found in archive", binary_name))
}

/// Extract a zip archive, looking for a specific binary inside.
///
/// The binary may be at the top level or nested in a directory.
/// On Windows, also matches `{binary_name}.exe`.
fn extract_zip(
    archive_bytes: &[u8],
    dest_dir: &std::path::Path,
    binary_name: &str,
) -> Result<(), String> {
    use std::io::Read;

    let cursor = std::io::Cursor::new(archive_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("failed to read zip archive: {}", e))?;

    let exe_name = format!("{}.exe", binary_name);

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("failed to read zip entry: {}", e))?;

        if file.is_dir() {
            continue;
        }

        let path = std::path::PathBuf::from(file.name());
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        if file_name == binary_name || file_name == exe_name {
            let dest = dest_dir.join(file_name);
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .map_err(|e| format!("failed to read binary from zip: {}", e))?;
            std::fs::write(&dest, &buf)
                .map_err(|e| format!("failed to write binary to {}: {}", dest.display(), e))?;
            return Ok(());
        }
    }

    Err(format!("binary '{}' not found in zip archive", binary_name))
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

        let manifest = ConnectorManifest {
            id: "test",
            name: "Test",
            description: "test",
            icon: "test",
            default_port: 3030,
            default_transport: crate::config::ConnectorTransport::Ipc,
            binary_hint: Some("test-bin"),
            github_repo: Some("org/repo"),
            asset_pattern: Some("test-bin-{os}-{arch}.{ext}"),
        };

        let name = manifest.asset_name().unwrap();
        assert!(name.contains(platform_os()));
        assert!(name.contains(platform_arch()));
        assert!(name.contains(platform_archive_ext()));
    }

    #[test]
    fn test_asset_name_none_without_pattern() {
        let manifest = ConnectorManifest {
            id: "test",
            name: "Test",
            description: "test",
            icon: "test",
            default_port: 3030,
            default_transport: crate::config::ConnectorTransport::Ipc,
            binary_hint: Some("test-bin"),
            github_repo: Some("org/repo"),
            asset_pattern: None,
        };

        assert!(manifest.asset_name().is_none());
    }
}
