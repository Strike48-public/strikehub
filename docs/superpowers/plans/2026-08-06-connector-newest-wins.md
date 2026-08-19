# Connector "Newest Wins" Resolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the bug where installing StrikeHub fresh over a previous install runs the *old* cached connector (pentest-agent / ks-connector) instead of the one the new install shipped, by making connector resolution pick the newest of {bundled, cached, latest GitHub release} using real source timestamps.

**Architecture:** Three connector-binary sources exist: the binary bundled with the StrikeHub install, the runtime fetch cache in `~/.strike48/strikehub/bin/`, and GitHub releases. Today the cache is preferred over the bundle (precedence) and freshness is decided by *tag-string equality* against GitHub — so a stale home-dir cache survives reinstalls and wins. We switch to **timestamp comparison** (ISO-8601 UTC, which sorts lexicographically = chronologically). The bundle's connector provenance (git commit timestamp + ref of the pinned connector) is **baked into the strikehub binary at compile time** via `build.rs`/`env!` — no editable sidecar file. On startup a **seed step** copies the bundled binary over the cache when the bundle is newer (fixing reinstall staleness); the existing background fetch then still upgrades the cache when a GitHub release is newer still (preserving runtime updates). The mutable cache keeps a small on-disk version record `{ref, ts}`.

**Tech Stack:** Rust (workspace crates `sh-core`, `sh-ui`), `serde_json` (already a dep), GitHub Actions (`release.yml`), git (commit timestamps in CI).

## Global Constraints

- Timestamps are **ISO-8601 UTC, `Z`-suffixed, second precision** (e.g. `2026-08-06T14:03:22Z`). String comparison is the ordering — do not add `chrono`/`time` deps.
- The cache version record lives at `~/.strike48/strikehub/bin/<binary_name>.version` (unchanged path; format upgraded from a bare tag string to JSON `{"ref": "...", "ts": "..."}`). A legacy bare-tag file (no JSON) MUST be treated as timestamp epoch-0 (always stale) so the first run after this change re-seeds from the bundle.
- Bundled connector provenance is read via `option_env!` (compile-time, immutable). When absent (local dev build with no CI env), treat the bundle as timestamp epoch-0 (never wins) so dev sibling-workspace resolution and the cache are unaffected.
- "Newest wins" compares **connector source timestamps** (the connector repo's git commit date of the pinned ref), NOT StrikeHub's build time — bundling last-week's connector in a today-built StrikeHub must not clobber a newer cached connector.
- Connector ids in scope: `pick` (binary `pentest-agent`, env key suffix `PICK`), `kubestudio` (binary `ks-connector`, env key suffix `KUBESTUDIO`). Env var naming: `STRIKEHUB_BUNDLED_<SUFFIX>_REF` and `STRIKEHUB_BUNDLED_<SUFFIX>_TS`.
- Resolver precedence order (ipc_runner.rs `resolve_binary_in`) is NOT reordered by this plan; the seed step keeps the cache ≥ bundle so the existing cache-before-bundle order stops causing staleness.
- Do not change the dev sibling-workspace path (precedence #2) — local `target/{debug,release}` builds must still win for developers.

---

### Task 1: Version record type + timestamp comparison (sh-core)

Create a small module holding the on-disk cache version record and the ordering primitive both the seed step and the fetch step will use.

**Files:**
- Create: `crates/sh-core/src/connector_version.rs`
- Modify: `crates/sh-core/src/lib.rs` (add `pub mod connector_version;`)

**Interfaces:**
- Produces:
  - `pub struct ConnectorVersion { pub r#ref: String, pub ts: String }`
  - `impl ConnectorVersion`: `pub fn new(r#ref: impl Into<String>, ts: impl Into<String>) -> Self`
  - `pub fn read_version_file(path: &std::path::Path) -> ConnectorVersion` — parses JSON; on missing file, unreadable, or legacy bare-tag content returns `ConnectorVersion { ref: <raw-or-empty>, ts: String::new() }` (empty ts = epoch-0).
  - `pub fn write_version_file(path: &std::path::Path, v: &ConnectorVersion) -> std::io::Result<()>` — writes pretty JSON.
  - `pub fn is_newer(candidate_ts: &str, existing_ts: &str) -> bool` — `candidate_ts > existing_ts` by string comparison; an empty candidate is never newer, a non-empty candidate beats an empty existing.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/sh-core/src/connector_version.rs`:

```rust
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
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sh-core connector_version:: 2>&1 | tail -20`
Expected: FAIL — `connector_version` module / functions do not exist (compile error).

- [ ] **Step 3: Write minimal implementation**

Put this at the TOP of `crates/sh-core/src/connector_version.rs` (above the test module):

```rust
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
```

Add to `crates/sh-core/src/lib.rs` near the other `pub mod` declarations:

```rust
pub mod connector_version;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sh-core connector_version:: 2>&1 | tail -20`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/sh-core/src/connector_version.rs crates/sh-core/src/lib.rs
git commit -m "feat(sh-core): connector version record + iso8601 timestamp ordering"
```

---

### Task 2: Bake bundled connector provenance into the binary (build.rs + accessor)

Emit the pinned connectors' ref + source timestamp as compile-time env vars, and expose them to runtime code via a typed accessor. CI supplies the values; local builds omit them (accessor returns epoch-0).

**Files:**
- Modify: `crates/sh-core/build.rs` (append emission of bundled-connector env vars)
- Modify: `crates/sh-core/src/connector_version.rs` (add `bundled_version(connector_id)` accessor + test)

**Interfaces:**
- Consumes: `ConnectorVersion` (Task 1).
- Produces: `pub fn bundled_version(connector_id: &str) -> ConnectorVersion` — returns the baked ref+ts for `"pick"` / `"kubestudio"`, or an epoch-0 record (`ts == ""`) when unknown id or the env vars weren't baked (local dev).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/sh-core/src/connector_version.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sh-core connector_version::tests::bundled 2>&1 | tail -20`
Expected: FAIL — `bundled_version` not found (compile error).

- [ ] **Step 3: Write minimal implementation**

Append to `crates/sh-core/src/connector_version.rs` (above the test module):

```rust
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
```

Append to `crates/sh-core/build.rs`, at the end of `main()` (before the closing brace):

```rust
    // Bundled connector provenance (baked in so the runtime "newest wins"
    // resolver knows what this install shipped, without an editable sidecar).
    // CI sets these from the pinned connector ref's git commit timestamp; local
    // builds leave them unset (option_env! -> None -> epoch-0, bundle never wins).
    for (suffix, ref_env, ts_env) in [
        ("PICK", "STRIKEHUB_BUNDLED_PICK_REF", "STRIKEHUB_BUNDLED_PICK_TS"),
        (
            "KUBESTUDIO",
            "STRIKEHUB_BUNDLED_KUBESTUDIO_REF",
            "STRIKEHUB_BUNDLED_KUBESTUDIO_TS",
        ),
    ] {
        // Re-run build.rs when the caller changes these (so a new CI value is
        // picked up without a clean rebuild).
        println!("cargo:rerun-if-env-changed={}", ref_env);
        println!("cargo:rerun-if-env-changed={}", ts_env);
        if let Ok(v) = std::env::var(ref_env) {
            println!("cargo:rustc-env={}={}", ref_env, v);
        }
        if let Ok(v) = std::env::var(ts_env) {
            println!("cargo:rustc-env={}={}", ts_env, v);
        }
        let _ = suffix;
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sh-core connector_version:: 2>&1 | tail -20`
Expected: PASS (6 tests total).

- [ ] **Step 5: Commit**

```bash
git add crates/sh-core/build.rs crates/sh-core/src/connector_version.rs
git commit -m "feat(sh-core): bake bundled connector ref+timestamp via build.rs env"
```

---

### Task 3: Switch the runtime fetch to timestamp comparison + write JSON version (connector_fetch)

Read GitHub's release `published_at`, only download when it's newer than the cached record's timestamp, and write the cache `.version` as the new JSON `{ref, ts}` format. Preserves "a genuinely newer GitHub release still wins."

**Files:**
- Modify: `crates/sh-core/src/connector_fetch.rs` (`fetch_latest_release` return type; version compare at ~L118-129; version write at ~L246-249)

**Interfaces:**
- Consumes: `crate::connector_version::{ConnectorVersion, read_version_file, write_version_file, is_newer}` (Tasks 1–2).
- Produces: `fetch_latest_release` now returns `Result<(String /*tag*/, String /*published_at ISO-8601*/), String>`. The `AlreadyCurrent` / `Downloaded` semantics are unchanged for callers.

- [ ] **Step 1: Write the failing test**

Add a test module at the bottom of `crates/sh-core/src/connector_fetch.rs` (the file currently has none for this logic). This tests the pure decision helper we extract in Step 3:

```rust
#[cfg(test)]
mod newest_tests {
    use super::should_download;

    #[test]
    fn downloads_when_release_is_newer_than_cache() {
        assert!(should_download(
            true,                    // binary_exists
            "2026-08-05T00:00:00Z",  // cached ts
            "2026-08-06T00:00:00Z",  // release ts
        ));
    }

    #[test]
    fn skips_when_cache_is_current_or_newer() {
        assert!(!should_download(true, "2026-08-06T00:00:00Z", "2026-08-06T00:00:00Z"));
        assert!(!should_download(true, "2026-08-07T00:00:00Z", "2026-08-06T00:00:00Z"));
    }

    #[test]
    fn downloads_when_no_binary_cached_regardless_of_ts() {
        assert!(should_download(false, "2026-08-09T00:00:00Z", ""));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sh-core newest_tests:: 2>&1 | tail -20`
Expected: FAIL — `should_download` not found.

- [ ] **Step 3: Write minimal implementation**

Add the decision helper near the top of `crates/sh-core/src/connector_fetch.rs` (after the `use` lines):

```rust
/// Decide whether to download: yes if no binary is cached, or the release is
/// strictly newer than the cached record. Split out so the rule is unit-tested.
pub(crate) fn should_download(binary_exists: bool, cached_ts: &str, release_ts: &str) -> bool {
    if !binary_exists {
        return true;
    }
    crate::connector_version::is_newer(release_ts, cached_ts)
}
```

Change `fetch_latest_release` (currently returns `Result<String, String>` and reads only `tag_name`) to also return `published_at`:

```rust
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
```

Update the caller in `ensure_connector_binary_inner`. Replace the `let latest_tag = match fetch_latest_release(...)` block AND the "Check if we already have this version" block (~L106-129) with:

```rust
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
```

Replace the "Write version file" block (~L246-249) with the JSON writer:

```rust
    // Write version record (JSON: ref + source/publish timestamp).
    let record = crate::connector_version::ConnectorVersion::new(latest_tag.clone(), latest_ts);
    if let Err(e) = crate::connector_version::write_version_file(&version_path, &record) {
        tracing::warn!("failed to write version file: {}", e);
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sh-core newest_tests:: 2>&1 | tail -20`
Expected: PASS (3 tests).
Then confirm the crate still builds: `cargo build -p sh-core 2>&1 | tail -5` → `Finished`.

- [ ] **Step 5: Commit**

```bash
git add crates/sh-core/src/connector_fetch.rs
git commit -m "feat(sh-core): fetch connectors by published_at timestamp, write JSON version"
```

---

### Task 4: Seed the cache from the bundle when the bundle is newer (sh-core)

Add the startup seed: for each connector, if the baked bundle timestamp is newer than the cached record, copy the bundled binary + write the bundle's `.version` into the cache. This is the actual fix for reinstall staleness.

**Files:**
- Create: `crates/sh-core/src/connector_seed.rs`
- Modify: `crates/sh-core/src/lib.rs` (`pub mod connector_seed;` + re-export `seed_bundled_connectors`)

**Interfaces:**
- Consumes: `connector_version::{bundled_version, read_version_file, write_version_file, is_newer, ConnectorVersion}`; `connector_fetch::bin_cache_dir`.
- Produces:
  - `pub fn decide_seed(bundle_ts: &str, cached_ts: &str, bundle_binary_exists: bool) -> bool` — pure rule: seed only if the bundle binary exists AND the bundle ts is strictly newer than the cached ts.
  - `pub fn seed_bundled_connectors(exe: Option<&std::path::Path>)` — for `pick`/`kubestudio`, locates the bundled binary next to `exe`, and if `decide_seed` is true, copies it (and writes the JSON `.version`) into `bin_cache_dir()`. Best-effort: logs and continues on any IO error. No-op when `exe` is `None`.

- [ ] **Step 1: Write the failing test**

Create `crates/sh-core/src/connector_seed.rs` with the pure-rule test:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sh-core connector_seed:: 2>&1 | tail -20`
Expected: FAIL — module/`decide_seed` missing.

- [ ] **Step 3: Write minimal implementation**

Put at the top of `crates/sh-core/src/connector_seed.rs`:

```rust
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
```

Add to `crates/sh-core/src/lib.rs`:

```rust
pub mod connector_seed;
pub use connector_seed::seed_bundled_connectors;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sh-core connector_seed:: 2>&1 | tail -20`
Expected: PASS (4 tests).
Then: `cargo build -p sh-core 2>&1 | tail -3` → `Finished`.

- [ ] **Step 5: Commit**

```bash
git add crates/sh-core/src/connector_seed.rs crates/sh-core/src/lib.rs
git commit -m "feat(sh-core): seed connector cache from bundle when bundle is newer"
```

---

### Task 5: Call the seed at startup, before connectors resolve (sh-ui)

Wire `seed_bundled_connectors` into StrikeHub startup so it runs before any connector is launched and before the background fetch.

**Files:**
- Modify: `crates/sh-ui/src/main.rs:56` (right after `extract_bundled_binaries()`)

**Interfaces:**
- Consumes: `sh_core::seed_bundled_connectors` (Task 4).

- [ ] **Step 1: Add the seed call**

In `crates/sh-ui/src/main.rs`, the current line 56 is:

```rust
    sh_core::embedded::extract_bundled_binaries();
```

Replace it with:

```rust
    sh_core::embedded::extract_bundled_binaries();
    // "Newest wins": if the connectors bundled with THIS install are newer than
    // whatever is in the per-user cache (~/.strike48/strikehub/bin), refresh the
    // cache from the bundle now — before anything resolves or launches a
    // connector. Fixes installing fresh over an old StrikeHub running the old
    // connector. No-op on dev builds (bundle timestamp is epoch-0).
    sh_core::seed_bundled_connectors(std::env::current_exe().ok().as_deref());
```

- [ ] **Step 2: Build to verify it compiles**

Run: `cargo build --features desktop -p sh-ui 2>&1 | tail -5`
Expected: `Finished` (no errors).

- [ ] **Step 3: Manual smoke check of the seed decision (documented, not automated)**

Run the desktop build and confirm the log line appears/absents correctly:
- With a stale cache and a newer bundle → log `seed: refreshed cached 'pick' from bundle`.
- On a dev build (no baked ts) → no seed line (bundle epoch-0).

Run: `cargo build --features desktop -p sh-ui 2>&1 | tail -2`
Expected: `Finished`. (Runtime behavior verified in Task 7 integration check.)

- [ ] **Step 4: Commit**

```bash
git add crates/sh-ui/src/main.rs
git commit -m "feat(sh-ui): seed connector cache from bundle at startup"
```

---

### Task 6: CI — compute connector source timestamps and bake them into the strikehub build

Make `release.yml` resolve each pinned connector ref's git commit timestamp and pass `STRIKEHUB_BUNDLED_*_REF/_TS` env to the strikehub `cargo build` in all three build jobs (`build`, `installers`, `dmg`).

**Files:**
- Modify: `.github/workflows/release.yml` — the "Load connector refs" step in each of the 3 build jobs (currently `cat connector-versions.env >> "$GITHUB_ENV"` at lines ~131, ~321, ~502) AND the connector-checkout steps already clone pick/kubestudio at `PICK_REF`/`KUBESTUDIO_REF`.

**Interfaces:**
- Consumes: `PICK_REF` / `KUBESTUDIO_REF` (already in `connector-versions.env`), the cloned `/tmp/pick` and `/tmp/kubestudio` checkouts (already produced by the "Checkout and build" steps).
- Produces: `STRIKEHUB_BUNDLED_PICK_REF`, `STRIKEHUB_BUNDLED_PICK_TS`, `STRIKEHUB_BUNDLED_KUBESTUDIO_REF`, `STRIKEHUB_BUNDLED_KUBESTUDIO_TS` in `$GITHUB_ENV`, consumed by `build.rs` (Task 2).

**Ordering constraint:** the strikehub `cargo build` step must run AFTER the timestamps are in `$GITHUB_ENV`. In the `build` and `dmg` jobs the strikehub build (`- name: Build`) runs BEFORE the connector checkouts today, so the timestamp resolution must be added as its own step BEFORE the strikehub build (it clones shallowly just to read the commit date; the later full "Checkout and build" steps remain).

- [ ] **Step 1: Add a timestamp-resolution step before the strikehub build (build job)**

In `.github/workflows/release.yml`, in the **`build`** job, immediately AFTER the `- name: Load connector refs` step (~L131-132) and BEFORE `- uses: dtolnay/rust-toolchain@stable`, insert:

```yaml
      - name: Resolve bundled connector timestamps
        shell: bash
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          # Bake each pinned connector's git commit timestamp (ISO-8601 UTC) into
          # the strikehub binary so the runtime "newest wins" resolver knows what
          # this install shipped. Uses the GitHub API (no full clone needed here;
          # the connectors are cloned+built in later steps).
          resolve_ts () {
            local repo="$1" ref="$2"
            # Commit date of the ref; committer date in strict ISO-8601 (…Z).
            gh api "repos/$repo/commits/$ref" --jq '.commit.committer.date' 2>/dev/null
          }
          PICK_TS="$(resolve_ts Strike48-public/pick "$PICK_REF")"
          KS_TS="$(resolve_ts Strike48-public/kubestudio "$KUBESTUDIO_REF")"
          echo "STRIKEHUB_BUNDLED_PICK_REF=$PICK_REF" >> "$GITHUB_ENV"
          echo "STRIKEHUB_BUNDLED_PICK_TS=$PICK_TS" >> "$GITHUB_ENV"
          echo "STRIKEHUB_BUNDLED_KUBESTUDIO_REF=$KUBESTUDIO_REF" >> "$GITHUB_ENV"
          echo "STRIKEHUB_BUNDLED_KUBESTUDIO_TS=$KS_TS" >> "$GITHUB_ENV"
          echo "Bundled: pick=$PICK_REF@$PICK_TS  kubestudio=$KUBESTUDIO_REF@$KS_TS"
```

- [ ] **Step 2: Repeat the step for the `installers` and `dmg` jobs**

Insert the **same** `Resolve bundled connector timestamps` step (identical YAML from Step 1) into:
- the **`installers`** job, after its `- name: Load connector refs` (~L321-323) and before `- name: Install system dependencies (Linux)`.
- the **`dmg`** job, after its `- name: Load connector refs` (~L502-503) and before `- uses: dtolnay/rust-toolchain@stable`.

(The GitHub API returns the committer date as strict ISO-8601 `Z` already — no reformatting needed. `is_newer` compares it against `published_at` from Task 3, which is the same format.)

- [ ] **Step 3: Validate the workflow YAML parses**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('release.yml OK')"`
Expected: `release.yml OK`.

Grep to confirm the step appears 3× (once per build job):

Run: `grep -c "Resolve bundled connector timestamps" .github/workflows/release.yml`
Expected: `3`.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: bake pinned connector git-commit timestamps into strikehub build"
```

---

### Task 7: Integration verification (documented manual check)

Confirm end-to-end on a real machine that a newer bundle re-seeds a stale cache. This is a documented verification (no automated harness — it needs two builds and the home-dir cache).

**Files:** none (verification only).

- [ ] **Step 1: Simulate a stale cache**

On a test box (Linux is fine), with the current dev build:

```bash
# Fake an OLD cached pentest-agent with an old timestamp.
mkdir -p ~/.strike48/strikehub/bin
cp "$(command -v true)" ~/.strike48/strikehub/bin/pentest-agent  # any placeholder binary
printf '{"ref":"old-tag","ts":"2000-01-01T00:00:00Z"}' > ~/.strike48/strikehub/bin/pentest-agent.version
```

- [ ] **Step 2: Build strikehub with a baked-newer bundle timestamp**

Build with the bake envs set to a recent timestamp and a bundled binary present next to the exe:

```bash
STRIKEHUB_BUNDLED_PICK_REF=test-newer \
STRIKEHUB_BUNDLED_PICK_TS=2026-08-06T00:00:00Z \
  cargo build --features desktop -p sh-ui 2>&1 | tail -3
# Place a bundled pentest-agent next to the built strikehub so the seed can copy it:
cp "$(command -v true)" target/debug/pentest-agent
```

- [ ] **Step 3: Run and confirm the seed fires**

Run the built strikehub (headless is fine for the seed; it runs at startup before UI):

```bash
RUST_LOG=info ./target/debug/strikehub 2>&1 | grep -m1 "seed: refreshed cached 'pick'"
```
Expected: a line `seed: refreshed cached 'pick' from bundle (ref test-newer, ts 2026-08-06T00:00:00Z)`.
Then confirm the cache was overwritten:

```bash
cat ~/.strike48/strikehub/bin/pentest-agent.version
```
Expected: `{"ref":"test-newer","ts":"2026-08-06T00:00:00Z"}` (the old `2000-01-01` record is gone).

- [ ] **Step 4: Confirm the negative case (dev build does NOT seed)**

Rebuild WITHOUT the bake envs and re-run against a fresh stale cache:

```bash
cargo build --features desktop -p sh-ui 2>&1 | tail -2
printf '{"ref":"keep","ts":"2026-01-01T00:00:00Z"}' > ~/.strike48/strikehub/bin/pentest-agent.version
RUST_LOG=info ./target/debug/strikehub 2>&1 | grep "seed: refreshed" || echo "NO SEED (correct for dev build)"
cat ~/.strike48/strikehub/bin/pentest-agent.version   # must still say "keep"
```
Expected: `NO SEED (correct for dev build)` and the `keep` record intact — because the local build's baked timestamp is epoch-0.

- [ ] **Step 5: Clean up the test cache**

```bash
rm -f ~/.strike48/strikehub/bin/pentest-agent ~/.strike48/strikehub/bin/pentest-agent.version
```

No commit (verification only).

---

## Self-Review

**1. Spec coverage:**
- "Newest wins by real source timestamp, not tag equality" → Tasks 1 (`is_newer`), 3 (fetch by `published_at`), 4 (seed by commit ts). ✓
- "Bake bundle provenance into the binary, no editable file" → Task 2 (`build.rs` + `option_env!`). ✓
- "Fix reinstall staleness (the reported bug)" → Task 4 seed + Task 5 startup wiring. ✓
- "Preserve runtime updates without rebuild" → Task 3 keeps fetch, now timestamp-gated. ✓
- "Don't downgrade a newer cached connector under a today-built StrikeHub" → Task 4 compares connector *source* ts (commit date), not StrikeHub build time. ✓
- "Legacy bare-tag cache migration" → Task 1 `read_version_file` treats it as epoch-0 (test covers it). ✓
- "Dev builds unaffected (sibling workspace still wins)" → epoch-0 bundle never seeds (Task 4 test) + precedence #2 untouched. ✓
- "All 3 packagers get the bundled timestamp" → Task 6 adds the step to `build`, `installers`, `dmg`. ✓

**2. Placeholder scan:** No TBD/TODO/"handle errors" placeholders; every code step has complete code. ✓

**3. Type consistency:** `ConnectorVersion { ref, ts }`, `is_newer(candidate, existing)`, `read_version_file`/`write_version_file`, `bundled_version(id)`, `should_download(exists, cached_ts, release_ts)`, `decide_seed(bundle_ts, cached_ts, exists)`, `seed_bundled_connectors(Option<&Path>)`, `fetch_latest_release -> (tag, published_at)` — names/signatures match across Tasks 1–6. Env var names (`STRIKEHUB_BUNDLED_PICK_REF/_TS`, `..._KUBESTUDIO_...`) are identical in build.rs (Task 2), the accessor (Task 2), and CI (Task 6). ✓

**Note for implementer:** the MSI job (`build-msi.ps1`) currently *downloads* connectors from fixed release URLs rather than using the `PICK_REF`/`KUBESTUDIO_REF` checkout the other jobs use (observed at scripts/build-msi.ps1:56-71). That is a pre-existing inconsistency outside this plan's scope, but it means the Windows MSI's *bundled* connector may not match `PICK_REF`. Flag it to the human; do not fix it here unless directed.
