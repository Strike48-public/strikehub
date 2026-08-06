//! Connector version records + timestamp ordering for "newest wins" resolution.
//!
//! Timestamps are ISO-8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`); lexicographic string
//! comparison equals chronological order, so no date library is needed. An
//! empty timestamp means "epoch 0 / unknown" and never wins a comparison.

use std::path::Path;

/// A connector binary's provenance: the git ref it was built from and the
/// source commit timestamp (ISO-8601 UTC). Persisted next to a cached binary
/// as `<binary>.version`; also produced (in-memory) for the baked-in bundle.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ConnectorVersion {
    pub r#ref: String,
    pub ts: String,
}

impl ConnectorVersion {
    pub fn new(r#ref: impl Into<String>, ts: impl Into<String>) -> Self {
        Self {
            r#ref: r#ref.into(),
            ts: ts.into(),
        }
    }
}

/// Read a `.version` record. Returns an epoch-0 record (`ts == ""`) for a
/// missing/unreadable file OR a legacy bare-tag file (the pre-JSON format,
/// which had no timestamp) so the first run after upgrading re-seeds from the
/// bundle.
pub fn read_version_file(path: &Path) -> ConnectorVersion {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return ConnectorVersion::new(String::new(), String::new());
    };
    match serde_json::from_str::<ConnectorVersion>(&raw) {
        Ok(v) => v,
        // Legacy bare-tag file: keep the tag, force epoch-0 timestamp.
        Err(_) => ConnectorVersion::new(raw.trim().to_string(), String::new()),
    }
}

/// Write a `.version` record as pretty JSON.
pub fn write_version_file(path: &Path, v: &ConnectorVersion) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(path, json)
}

/// True when `candidate_ts` is strictly newer than `existing_ts`. Empty
/// (epoch-0) candidates never win; a non-empty candidate beats an empty existing.
pub fn is_newer(candidate_ts: &str, existing_ts: &str) -> bool {
    if candidate_ts.is_empty() {
        return false;
    }
    candidate_ts > existing_ts
}

/// The connector provenance baked into this StrikeHub binary at build time.
///
/// CI passes `STRIKEHUB_BUNDLED_<SUFFIX>_REF` / `_TS` to the strikehub build
/// (see build.rs), where `<SUFFIX>` is `PICK` or `KUBESTUDIO`. Local/dev builds
/// don't set them, so this returns an epoch-0 record and the bundle never wins
/// a "newest" comparison — dev sibling-workspace resolution is unaffected.
pub fn bundled_version(connector_id: &str) -> ConnectorVersion {
    let (r#ref, ts) = match connector_id {
        "pick" => (
            option_env!("STRIKEHUB_BUNDLED_PICK_REF").unwrap_or(""),
            option_env!("STRIKEHUB_BUNDLED_PICK_TS").unwrap_or(""),
        ),
        "kubestudio" => (
            option_env!("STRIKEHUB_BUNDLED_KUBESTUDIO_REF").unwrap_or(""),
            option_env!("STRIKEHUB_BUNDLED_KUBESTUDIO_TS").unwrap_or(""),
        ),
        _ => ("", ""),
    };
    ConnectorVersion::new(r#ref.to_string(), ts.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_orders_by_iso8601_string() {
        assert!(is_newer("2026-08-06T14:00:00Z", "2026-08-05T23:59:59Z"));
        assert!(!is_newer("2026-08-05T00:00:00Z", "2026-08-06T00:00:00Z"));
        assert!(!is_newer("2026-08-06T14:00:00Z", "2026-08-06T14:00:00Z")); // equal = not newer
    }

    #[test]
    fn empty_timestamps_are_epoch_zero() {
        // A non-empty candidate beats an empty (epoch-0) existing.
        assert!(is_newer("2020-01-01T00:00:00Z", ""));
        // An empty candidate never wins.
        assert!(!is_newer("", "2020-01-01T00:00:00Z"));
        assert!(!is_newer("", ""));
    }

    #[test]
    fn read_missing_file_is_epoch_zero() {
        let v = read_version_file(std::path::Path::new("/no/such/file.version"));
        assert_eq!(v.ts, "");
    }

    #[test]
    fn read_legacy_bare_tag_is_epoch_zero() {
        let dir = std::env::temp_dir().join(format!("cv-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("legacy.version");
        std::fs::write(&p, "proot-openat2-v1").unwrap(); // old bare-tag format
        let v = read_version_file(&p);
        assert_eq!(v.ts, "", "legacy bare tag must read as epoch-0");
        assert_eq!(v.r#ref, "proot-openat2-v1");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_then_read_roundtrips() {
        let dir = std::env::temp_dir().join(format!("cv-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("rt.version");
        let v = ConnectorVersion::new("josh/catching-up", "2026-08-06T14:03:22Z");
        write_version_file(&p, &v).unwrap();
        let back = read_version_file(&p);
        assert_eq!(back.r#ref, "josh/catching-up");
        assert_eq!(back.ts, "2026-08-06T14:03:22Z");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bundled_version_unknown_id_is_epoch_zero() {
        let v = bundled_version("does-not-exist");
        assert_eq!(v.ts, "");
    }

    #[test]
    fn bundled_version_known_ids_do_not_panic() {
        // In a local build these are epoch-0 (env not baked); in CI they carry
        // real values. Either way the call must be total and never panic.
        let _pick = bundled_version("pick");
        let _ks = bundled_version("kubestudio");
    }
}
