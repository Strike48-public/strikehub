//! Seed the runtime connector cache from the binaries bundled with this
//! install, when the bundle is newer than what's cached.
//!
//! Fixes the "install fresh over an old StrikeHub runs the OLD connector" bug:
//! the cache in ~/.strike48/strikehub/bin/ survives reinstalls and the resolver
//! prefers it over the bundle, so without this a stale cached connector wins.

use std::path::Path;

use crate::connector_fetch::bin_cache_dir;
use crate::connector_version::{
    bundled_version, is_newer, read_version_file, write_version_file, ConnectorVersion,
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

/// For each seedable connector, copy the bundled binary + write its version
/// record into the cache when the bundle is newer. Best-effort; never panics.
pub fn seed_bundled_connectors(exe: Option<&Path>) {
    let Some(exe_dir) = exe.and_then(|e| e.parent()) else {
        return;
    };
    let cache_dir = bin_cache_dir();

    for (id, base) in SEEDABLE {
        let filename = binary_filename(base);
        let bundled_bin = exe_dir.join(&filename);
        let cache_bin = cache_dir.join(&filename);
        let cache_ver = cache_dir.join(format!("{base}.version"));

        let bundle = bundled_version(id);
        let cached = read_version_file(&cache_ver);

        if !decide_seed(&bundle.ts, &cached.ts, bundled_bin.exists()) {
            continue;
        }

        if let Err(e) = std::fs::create_dir_all(&cache_dir) {
            tracing::warn!("seed: cannot create cache dir {}: {}", cache_dir.display(), e);
            continue;
        }
        if let Err(e) = std::fs::copy(&bundled_bin, &cache_bin) {
            tracing::warn!("seed: copy {} -> cache failed: {}", bundled_bin.display(), e);
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
}

#[cfg(test)]
mod tests {
    use super::decide_seed;

    #[test]
    fn seeds_when_bundle_newer_and_present() {
        assert!(decide_seed("2026-08-06T00:00:00Z", "2026-08-05T00:00:00Z", true));
    }

    #[test]
    fn does_not_seed_when_cache_current_or_newer() {
        assert!(!decide_seed("2026-08-05T00:00:00Z", "2026-08-06T00:00:00Z", true));
        assert!(!decide_seed("2026-08-06T00:00:00Z", "2026-08-06T00:00:00Z", true));
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
}
