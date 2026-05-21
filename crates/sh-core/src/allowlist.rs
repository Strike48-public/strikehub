//! GitHub repository allowlist for connector binary downloads.
//!
//! Controls which `owner/repo` sources are permitted for downloading connector
//! binaries. Ships with compile-time defaults from `build-defaults.toml` that
//! operators can extend or replace via config or environment variable.
//!
//! ## Precedence (replace semantics)
//!
//! 1. Compile-time defaults from `build-defaults.toml` (baked in via `build.rs`)
//! 2. If `[allowlist] sources` is present in `connectors.toml` → **replaces** defaults
//! 3. If `STRIKEHUB_ALLOWED_SOURCES` env var is set (comma-separated) → **replaces** all

use std::sync::OnceLock;

/// A single allow pattern: either an org wildcard (`MyOrg/*`) or an exact
/// `owner/repo` match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowPattern {
    /// Matches any repo under the given GitHub org/owner.
    /// Pattern: `"SomeOrg/*"`
    OrgWildcard(String),
    /// Matches a specific `owner/repo`.
    Exact(String),
}

impl AllowPattern {
    /// Parse a pattern string into an `AllowPattern`.
    ///
    /// `"Strike48-public/*"` → `OrgWildcard("Strike48-public")`
    /// `"my-corp/my-repo"` → `Exact("my-corp/my-repo")`
    pub fn parse(s: &str) -> Self {
        let trimmed = s.trim();
        if let Some(org) = trimmed.strip_suffix("/*") {
            AllowPattern::OrgWildcard(org.to_string())
        } else {
            AllowPattern::Exact(trimmed.to_string())
        }
    }

    /// Check if a `"owner/repo"` string matches this pattern.
    pub fn matches(&self, repo: &str) -> bool {
        match self {
            AllowPattern::OrgWildcard(org) => {
                // repo must start with "org/" and have something after the slash
                repo.starts_with(org.as_str())
                    && repo.as_bytes().get(org.len()) == Some(&b'/')
                    && repo.len() > org.len() + 1
            }
            AllowPattern::Exact(exact) => repo == exact,
        }
    }
}

/// The set of allowed GitHub repository sources.
#[derive(Debug, Clone)]
pub struct RepoAllowlist {
    patterns: Vec<AllowPattern>,
}

impl RepoAllowlist {
    /// Create an allowlist from a list of pattern strings.
    pub fn from_patterns(patterns: Vec<String>) -> Self {
        Self {
            patterns: patterns.iter().map(|s| AllowPattern::parse(s)).collect(),
        }
    }

    /// Check if a `"owner/repo"` string is permitted by this allowlist.
    pub fn is_allowed(&self, repo: &str) -> bool {
        self.patterns.iter().any(|p| p.matches(repo))
    }

    /// The raw patterns in this allowlist.
    pub fn patterns(&self) -> &[AllowPattern] {
        &self.patterns
    }
}

/// Global singleton allowlist, initialised once at startup.
static ALLOWLIST: OnceLock<RepoAllowlist> = OnceLock::new();

/// Load an allowlist using the three-layer precedence chain:
///
/// 1. Compile-time defaults (from `STRIKEHUB_DEFAULT_ALLOWED_SOURCES`)
/// 2. Config file `[allowlist] sources` (replaces defaults if present)
/// 3. `STRIKEHUB_ALLOWED_SOURCES` env var (replaces all if set)
pub fn load_allowlist(config_sources: Option<&[String]>) -> RepoAllowlist {
    // Layer 3: env var takes highest priority
    if let Ok(val) = std::env::var("STRIKEHUB_ALLOWED_SOURCES") {
        let patterns: Vec<String> = val
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !patterns.is_empty() {
            tracing::info!(
                "[allowlist] using STRIKEHUB_ALLOWED_SOURCES env var: {:?}",
                patterns
            );
            return RepoAllowlist::from_patterns(patterns);
        }
    }

    // Layer 2: config file overrides defaults
    if let Some(sources) = config_sources {
        if !sources.is_empty() {
            tracing::info!("[allowlist] using config file sources: {:?}", sources);
            return RepoAllowlist::from_patterns(sources.to_vec());
        }
    }

    // Layer 1: compile-time defaults
    let defaults = env!("STRIKEHUB_DEFAULT_ALLOWED_SOURCES");
    let patterns: Vec<String> = defaults
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    tracing::info!("[allowlist] using compile-time defaults: {:?}", patterns);
    RepoAllowlist::from_patterns(patterns)
}

/// Initialise the global allowlist singleton.
///
/// Call once at startup. Subsequent calls are no-ops.
pub fn init_allowlist(config_sources: Option<&[String]>) {
    let _ = ALLOWLIST.get_or_init(|| load_allowlist(config_sources));
}

/// Get the global allowlist. Panics if `init_allowlist()` was not called.
pub fn get_allowlist() -> &'static RepoAllowlist {
    ALLOWLIST
        .get()
        .expect("allowlist not initialised — call init_allowlist() first")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_org_wildcard_match() {
        let p = AllowPattern::OrgWildcard("Strike48-public".to_string());
        assert!(p.matches("Strike48-public/kubestudio"));
        assert!(p.matches("Strike48-public/pick"));
        assert!(!p.matches("Strike48-public/"));
        assert!(!p.matches("Strike48-publicX/repo"));
        assert!(!p.matches("other-org/repo"));
    }

    #[test]
    fn test_exact_match() {
        let p = AllowPattern::Exact("my-corp/internal-tool".to_string());
        assert!(p.matches("my-corp/internal-tool"));
        assert!(!p.matches("my-corp/other-tool"));
        assert!(!p.matches("my-corp/internal-tool/extra"));
    }

    #[test]
    fn test_parse_wildcard() {
        assert_eq!(
            AllowPattern::parse("Strike48-public/*"),
            AllowPattern::OrgWildcard("Strike48-public".to_string())
        );
    }

    #[test]
    fn test_parse_exact() {
        assert_eq!(
            AllowPattern::parse("my-corp/my-repo"),
            AllowPattern::Exact("my-corp/my-repo".to_string())
        );
    }

    #[test]
    fn test_parse_trims_whitespace() {
        assert_eq!(
            AllowPattern::parse("  Strike48-public/*  "),
            AllowPattern::OrgWildcard("Strike48-public".to_string())
        );
    }

    #[test]
    fn test_allowlist_is_allowed() {
        let al = RepoAllowlist::from_patterns(vec![
            "Strike48-public/*".to_string(),
            "my-corp/internal-tool".to_string(),
        ]);
        assert!(al.is_allowed("Strike48-public/kubestudio"));
        assert!(al.is_allowed("Strike48-public/pick"));
        assert!(al.is_allowed("my-corp/internal-tool"));
        assert!(!al.is_allowed("my-corp/other-tool"));
        assert!(!al.is_allowed("evil-org/malware"));
    }

    #[test]
    fn test_empty_allowlist_denies_all() {
        let al = RepoAllowlist::from_patterns(vec![]);
        assert!(!al.is_allowed("Strike48-public/kubestudio"));
    }
}
