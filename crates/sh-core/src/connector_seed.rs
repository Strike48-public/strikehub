//! Seed the runtime connector cache from the binaries bundled with this
//! install, when the bundle is newer than what's cached.
//!
//! Fixes the "install fresh over an old StrikeHub runs the OLD connector" bug:
//! the cache in ~/.strike48/strikehub/bin/ survives reinstalls and the resolver
//! prefers it over the bundle, so without this a stale cached connector wins.

use std::path::{Path, PathBuf};

use crate::connector_fetch::bin_cache_dir;
use crate::connector_version::{
    ConnectorVersion, bundled_version, is_newer, read_version_file, write_version_file,
};

/// (connector id, binary base name) pairs whose bundled copies we seed.
const SEEDABLE: &[(&str, &str)] = &[("pick", "pentest-agent"), ("kubestudio", "ks-connector")];

/// Pure rule: seed only when the bundled binary exists and its timestamp is
/// strictly newer than the cached record's.
pub fn decide_seed(bundle_ts: &str, cached_ts: &str, bundle_binary_exists: bool) -> bool {
    bundle_binary_exists && is_newer(bundle_ts, cached_ts)
}

fn binary_filename(base: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

/// Locate the bundled connector binary relative to the StrikeHub executable's
/// directory. Returns the first candidate that exists, or `None`.
///
/// Layout differs by packaging: mac/Linux bundle the connector as a direct
/// sibling of the exe (`.app/Contents/MacOS/`, AppImage `usr/bin/`), but the
/// Windows MSI installs connectors into an `connectors\` subdirectory (surfaced
/// at runtime via a PATH entry, not as an exe sibling). Probe the sibling first,
/// then the `connectors/` subdir, so the seed works on all three platforms.
fn bundled_binary_path(exe_dir: &Path, filename: &str) -> Option<std::path::PathBuf> {
    let sibling = exe_dir.join(filename);
    if sibling.exists() {
        return Some(sibling);
    }
    let sub = exe_dir.join("connectors").join(filename);
    if sub.exists() {
        return Some(sub);
    }
    None
}

/// For each seedable connector, copy the bundled binary + write its version
/// record into the cache when the bundle is newer. Best-effort; never panics.
pub fn seed_bundled_connectors(exe: Option<&Path>) {
    let Some(exe_dir) = exe.and_then(|e| e.parent()) else {
        return;
    };
    let cache_dir = bin_cache_dir();

    for (id, base) in SEEDABLE {
        let filename = binary_filename(base);
        let bundled_bin = bundled_binary_path(exe_dir, &filename);
        let cache_bin = cache_dir.join(&filename);
        let cache_ver = cache_dir.join(format!("{base}.version"));

        let bundle = bundled_version(id);
        let cached = read_version_file(&cache_ver);

        if !decide_seed(&bundle.ts, &cached.ts, bundled_bin.is_some()) {
            continue;
        }
        let Some(bundled_bin) = bundled_bin else {
            continue;
        };

        if let Err(e) = std::fs::create_dir_all(&cache_dir) {
            tracing::warn!(
                "seed: cannot create cache dir {}: {}",
                cache_dir.display(),
                e
            );
            continue;
        }
        if let Err(e) = std::fs::copy(&bundled_bin, &cache_bin) {
            tracing::warn!(
                "seed: copy {} -> cache failed: {}",
                bundled_bin.display(),
                e
            );
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&cache_bin, std::fs::Permissions::from_mode(0o755));
        }
        let record = ConnectorVersion::new(bundle.r#ref.clone(), bundle.ts.clone());
        if let Err(e) = write_version_file(&cache_ver, &record) {
            tracing::warn!("seed: write version {} failed: {}", cache_ver.display(), e);
            continue;
        }
        tracing::info!(
            "seed: refreshed cached '{}' from bundle (ref {}, ts {})",
            id,
            bundle.r#ref,
            bundle.ts
        );
    }

    // The connector cache is the directory the connector processes actually
    // run from (see resolve_binary precedence in ipc_runner.rs), so the
    // app-local VC++ runtime DLLs must live there too: a clean Windows 10
    // machine has no VC++ redist, and the DLLs the installer placed next to
    // strikehub.exe are not on the search path of a process whose directory
    // is the cache.
    seed_vc_runtime(&cache_dir, exe_dir);
}

/// VC++ 2015-2022 runtime DLLs required by the MSVC-built executables the
/// cache hosts (strikehub.exe and the connector binaries — the release MSI's
/// import tables were verified with `dumpbin /dependents`; they import
/// VCRUNTIME140.dll, and strikehub.exe additionally VCRUNTIME140_1.dll).
///
/// These are staged app-locally by the MSI (wix/main.wxs) and copied here at
/// first run, rather than requiring a VC++ redist install on the host.
pub const VC_RUNTIME_DLLS: &[&str] = &["vcruntime140.dll", "vcruntime140_1.dll"];

/// Pure rule: stage the app-local copy when a source copy exists and the
/// cache copy does not.
pub fn decide_crt_stage(source_exists: bool, cached_exists: bool) -> bool {
    source_exists && !cached_exists
}

/// Locate the app-local copy of a VC++ runtime DLL: sibling of the running
/// executable first (SFX/dev layout), then the MSI `connectors\` subdirectory
/// — the same layout probing as [`bundled_binary_path`].
fn crt_source_path(exe_dir: &Path, name: &str) -> Option<PathBuf> {
    let sibling = exe_dir.join(name);
    if sibling.exists() {
        return Some(sibling);
    }
    let sub = exe_dir.join("connectors").join(name);
    if sub.exists() {
        return Some(sub);
    }
    None
}

/// Copy the app-local VC++ runtime DLLs into the connector cache directory
/// so cached/fetched connector binaries can resolve the CRT on hosts without
/// a VC++ redist. Windows-only by design (the dynamic CRT is an MSVC
/// phenomenon); best-effort and never panics — a staging failure only means
/// the host has the redist or will get the original field error.
pub fn seed_vc_runtime(cache_dir: &Path, exe_dir: &Path) {
    if !cfg!(target_os = "windows") {
        return;
    }
    for name in VC_RUNTIME_DLLS {
        let Some(src) = crt_source_path(exe_dir, name) else {
            continue;
        };
        let dst = cache_dir.join(name);
        if !decide_crt_stage(true, dst.exists()) {
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(cache_dir) {
            tracing::warn!(
                "seed: cannot create cache dir {}: {}",
                cache_dir.display(),
                e
            );
            continue;
        }
        if let Err(e) = std::fs::copy(&src, &dst) {
            tracing::warn!("seed: copy {} -> cache failed: {}", src.display(), e);
            continue;
        }
        tracing::info!(
            "seed: staged VC++ runtime {} into {}",
            name,
            cache_dir.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        VC_RUNTIME_DLLS, bundled_binary_path, crt_source_path, decide_crt_stage, decide_seed,
    };

    #[test]
    fn finds_bundled_binary_as_exe_sibling() {
        let dir = std::env::temp_dir().join(format!("seed-sib-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("pentest-agent");
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(bundled_binary_path(&dir, "pentest-agent"), Some(f));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_bundled_binary_in_connectors_subdir() {
        // Windows MSI layout: connector lives in <exe_dir>\connectors\, not next
        // to the exe. The seed must still locate it.
        let dir = std::env::temp_dir().join(format!("seed-sub-{}", std::process::id()));
        let sub = dir.join("connectors");
        std::fs::create_dir_all(&sub).unwrap();
        let f = sub.join("pentest-agent");
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(bundled_binary_path(&dir, "pentest-agent"), Some(f));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn returns_none_when_bundle_absent_everywhere() {
        let dir = std::env::temp_dir().join(format!("seed-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(bundled_binary_path(&dir, "pentest-agent"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn seeds_when_bundle_newer_and_present() {
        assert!(decide_seed(
            "2026-08-06T00:00:00Z",
            "2026-08-05T00:00:00Z",
            true
        ));
    }

    #[test]
    fn does_not_seed_when_cache_current_or_newer() {
        assert!(!decide_seed(
            "2026-08-05T00:00:00Z",
            "2026-08-06T00:00:00Z",
            true
        ));
        assert!(!decide_seed(
            "2026-08-06T00:00:00Z",
            "2026-08-06T00:00:00Z",
            true
        ));
    }

    #[test]
    fn does_not_seed_when_bundle_missing() {
        assert!(!decide_seed("2026-08-09T00:00:00Z", "", false));
    }

    #[test]
    fn does_not_seed_when_bundle_ts_unknown() {
        // Local dev build: bundle ts is empty (epoch-0) -> never seeds, so the
        // sibling-workspace / existing cache path is untouched.
        assert!(!decide_seed("", "", true));
    }

    #[test]
    fn crt_stage_only_when_source_present_and_cache_absent() {
        assert!(decide_crt_stage(true, false));
        assert!(!decide_crt_stage(false, false));
        assert!(!decide_crt_stage(true, true));
        assert!(!decide_crt_stage(false, true));
    }

    #[test]
    fn crt_dll_list_matches_msi_imports() {
        // Keep in lockstep with the redist\ staging in scripts/build-msi.ps1
        // and the File elements in wix/main.wxs: exactly the non-OS DLLs the
        // release binaries import (verified via dumpbin /dependents).
        assert_eq!(VC_RUNTIME_DLLS, &["vcruntime140.dll", "vcruntime140_1.dll"]);
    }

    #[test]
    fn crt_source_found_as_exe_sibling_or_in_connectors_subdir() {
        let dir = std::env::temp_dir().join(format!("crt-src-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("connectors")).unwrap();
        // Sibling wins when both exist.
        std::fs::write(dir.join("vcruntime140.dll"), b"sib").unwrap();
        std::fs::write(dir.join("connectors").join("vcruntime140.dll"), b"sub").unwrap();
        assert_eq!(
            crt_source_path(&dir, "vcruntime140.dll"),
            Some(dir.join("vcruntime140.dll"))
        );
        std::fs::remove_file(dir.join("vcruntime140.dll")).unwrap();
        // Fall back to the MSI connectors\ layout.
        assert_eq!(
            crt_source_path(&dir, "vcruntime140.dll"),
            Some(dir.join("connectors").join("vcruntime140.dll"))
        );
        // Absent everywhere -> None (dev build without the staging).
        assert_eq!(crt_source_path(&dir, "vcruntime140_1.dll"), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
