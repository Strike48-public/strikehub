//! Sentry observability integration for StrikeHub.
//!
//! Provides error reporting, tracing, and context enrichment via Sentry.
//! DSN and other config are set at compile time via `build-defaults.toml`.

use sentry::{ClientInitGuard, SessionMode, TransactionContext};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Compile-time Sentry DSN from build-defaults.toml.
/// Returns empty string if not set.
pub fn sentry_dsn() -> &'static str {
    option_env!("STRIKEHUB_SENTRY_DSN").unwrap_or("")
}

/// Compile-time Sentry environment from build-defaults.toml.
/// Defaults to "development" for debug builds, "production" for release builds.
pub fn sentry_environment() -> &'static str {
    option_env!("STRIKEHUB_SENTRY_ENVIRONMENT").unwrap_or(if cfg!(debug_assertions) {
        "development"
    } else {
        "production"
    })
}

/// Baseline trace sample rate for high-volume spans (currently `bridge.request`).
/// Business-event spans (oauth.flow, connector.start, connector.fetch) override this to 1.0
/// via [`traces_sampler`] so they're never sampled away.
fn bridge_sample_rate() -> f32 {
    option_env!("STRIKEHUB_SENTRY_TRACES_SAMPLE_RATE")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.2)
}

/// Sampler: keep 100% of low-volume business-event spans, sample bridge requests at the
/// configured rate. Runs once per root transaction at start.
fn traces_sampler(ctx: &TransactionContext) -> f32 {
    match ctx.name() {
        "oauth.flow" | "connector.start" | "connector.fetch" => 1.0,
        "bridge.request" => bridge_sample_rate(),
        _ => bridge_sample_rate(),
    }
}

/// Application mode for context tagging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Desktop,
    Server,
}

impl std::fmt::Display for AppMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppMode::Desktop => write!(f, "desktop"),
            AppMode::Server => write!(f, "server"),
        }
    }
}

/// Initialize Sentry with the compiled-in DSN.
///
/// Returns `Some(guard)` if Sentry was initialized (DSN is non-empty),
/// or `None` if Sentry is disabled. The guard must be kept alive for
/// the duration of the application to ensure events are flushed on shutdown.
pub fn init_sentry(mode: AppMode) -> Option<ClientInitGuard> {
    let dsn = sentry_dsn();
    if dsn.is_empty() {
        tracing::debug!("Sentry disabled (no DSN configured)");
        return None;
    }

    let environment = sentry_environment();

    let guard = sentry::init((
        dsn,
        sentry::ClientOptions {
            release: sentry::release_name!(),
            environment: Some(environment.into()),
            traces_sampler: Some(std::sync::Arc::new(traces_sampler)),
            // Release Health: emit a session per app run so Sentry can compute
            // DAU/WAU, crash-free session rate, and adoption per release without
            // any custom instrumentation.
            auto_session_tracking: true,
            session_mode: SessionMode::Application,
            before_send: Some(std::sync::Arc::new(before_send)),
            attach_stacktrace: true,
            ..Default::default()
        },
    ));

    // Set initial tags for platform context
    sentry::configure_scope(|scope| {
        scope.set_tag("app.mode", mode.to_string());
        scope.set_tag("app.platform", platform_os());
        scope.set_tag("app.arch", platform_arch());
    });

    tracing::info!(
        "Sentry initialized: env={}, mode={}, bridge_sample_rate={}",
        environment,
        mode,
        bridge_sample_rate()
    );

    Some(guard)
}

/// Set user context after successful OAuth sign-in.
///
/// Call this after authentication completes to associate errors with the user.
pub fn set_user_context(user_id: Option<&str>, email: Option<&str>, username: Option<&str>) {
    sentry::configure_scope(|scope| {
        scope.set_user(Some(sentry::User {
            id: user_id.map(String::from),
            email: email.map(String::from),
            username: username.map(String::from),
            ..Default::default()
        }));
    });
    tracing::debug!(
        "Sentry user context set: id={:?}, email={:?}, username={:?}",
        user_id,
        email,
        username
    );
}

/// Clear user context on sign-out.
pub fn clear_user_context() {
    sentry::configure_scope(|scope| {
        scope.set_user(None);
    });
    tracing::debug!("Sentry user context cleared");
}

/// Set connector context when a connector is selected.
pub fn set_connector_context(connector_id: &str, connector_name: Option<&str>) {
    sentry::configure_scope(|scope| {
        scope.set_tag("connector.id", connector_id);
        if let Some(name) = connector_name {
            scope.set_tag("connector.name", name);
        }
    });
}

/// How long a repeated event message is suppressed after the first
/// occurrence was forwarded.
const EVENT_DEDUPE_WINDOW: Duration = Duration::from_secs(60);

/// Cap on distinct messages tracked by the dedupe cache so a workload with
/// unbounded message variety cannot grow it without limit.
const EVENT_DEDUPE_MAX_TRACKED: usize = 1024;

/// Process-wide cache of recently forwarded event messages (dedupe defense).
fn dedupe_cache() -> &'static std::sync::Mutex<HashMap<String, Instant>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Instant>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Decide whether an event carrying the given message key should be forwarded.
///
/// Keeps the first occurrence of a key and suppresses repeats while less than
/// `window` has elapsed since the kept occurrence. The window is anchored at
/// the kept occurrence (duplicates do not refresh it), so a key becomes
/// forwardable again exactly `window` after its last forward.
///
/// The cache is also bounded: expired entries are evicted on every call, and
/// if more than `max_tracked` live entries remain, the oldest are dropped.
///
/// Pure with respect to its arguments (no globals, `now` is injected) so the
/// window behavior is unit-testable.
fn should_forward_message(
    cache: &mut HashMap<String, Instant>,
    key: &str,
    now: Instant,
    window: Duration,
    max_tracked: usize,
) -> bool {
    // Drop entries that fell out of the window.
    cache.retain(|_, last| match now.checked_duration_since(*last) {
        Some(age) => age < window,
        // `last` is in the future (should not happen with a monotonic clock):
        // keep the entry rather than guessing.
        None => true,
    });

    if cache.contains_key(key) {
        // Seen within the window: duplicate, drop.
        return false;
    }

    cache.insert(key.to_string(), now);

    // Bound the cache even when every event carries a unique message.
    while cache.len() > max_tracked {
        match cache
            .iter()
            .min_by_key(|(_, ts)| *ts)
            .map(|(k, _)| k.clone())
        {
            Some(oldest) => {
                cache.remove(&oldest);
            }
            None => break,
        }
    }

    true
}

/// Before-send hook to redact sensitive data from events and to dedupe
/// repeated message events.
///
/// Redacts:
/// - `authorization` headers
/// - Fields containing `token` in the name
///
/// Dedupes: drops an event whose formatted message was already forwarded
/// within [`EVENT_DEDUPE_WINDOW`]. This is defense in depth against hot
/// logging loops (a single `tracing::error!` on a re-entrant path can emit
/// millions of identical events; see strikehub issue #71). Events without a
/// message (panics, captures with exceptions only) are never deduped.
fn before_send(
    mut event: sentry::protocol::Event<'static>,
) -> Option<sentry::protocol::Event<'static>> {
    // Redact request headers
    if let Some(ref mut request) = event.request {
        for (key, value) in request.headers.iter_mut() {
            let key_lower = key.to_lowercase();
            if key_lower == "authorization" || key_lower.contains("token") {
                *value = "[REDACTED]".to_string();
            }
        }
    }

    // Redact extra data containing token fields
    for (key, value) in event.extra.iter_mut() {
        if key.to_lowercase().contains("token") {
            *value = serde_json::Value::String("[REDACTED]".to_string());
        }
    }

    // Dedupe repeated message events (see above).
    if let Some(key) = event.message.as_deref().filter(|m| !m.is_empty()) {
        let key = key.to_string();
        let mut cache = dedupe_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !should_forward_message(
            &mut cache,
            &key,
            Instant::now(),
            EVENT_DEDUPE_WINDOW,
            EVENT_DEDUPE_MAX_TRACKED,
        ) {
            return None;
        }
    }

    Some(event)
}

/// Get the current platform OS name.
fn platform_os() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        "unknown"
    }
}

/// Get the current platform architecture.
fn platform_arch() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64"
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        "unknown"
    }
}

/// The set of span names we explicitly instrument and forward to Sentry.
/// Anything else (Dioxus runtime spans, library spans, etc.) is dropped at the
/// tracing layer so it doesn't pollute traces or dashboards.
const INSTRUMENTED_SPANS: &[&str] = &[
    "oauth.flow",
    "connector.start",
    "connector.fetch",
    "bridge.request",
];

/// Span filter for `sentry_tracing::layer()`. Returns `true` only for spans we
/// explicitly instrument; this keeps Dioxus + library spans out of Sentry.
pub fn instrumented_spans_only(metadata: &tracing::Metadata<'_>) -> bool {
    INSTRUMENTED_SPANS.contains(&metadata.name())
}

// ── User interaction tracking ──────────────────────────────────────────

/// Track a user action or navigation event.
///
/// Use this to record what parts of the product users interact with.
/// The action name should be a descriptive identifier, e.g.:
/// - `"nav.connector.kubestudio"` — user navigated to KubeStudio
/// - `"nav.settings"` — user opened settings
/// - `"action.sign_out"` — user signed out
/// - `"action.connector.add"` — user added a custom connector
///
/// Actions are recorded as Sentry breadcrumbs for session context,
/// making them visible in error reports to understand user journeys.
pub fn track_action(action: &str) {
    sentry::add_breadcrumb(sentry::Breadcrumb {
        ty: "user".into(),
        category: Some("ui.action".into()),
        message: Some(action.to_string()),
        level: sentry::Level::Info,
        ..Default::default()
    });
    tracing::debug!("Tracked user action: {}", action);
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_secs(60);
    const MAX_TRACKED: usize = 1024;

    fn future_at(base: Instant, offset: Duration) -> Instant {
        base.checked_add(offset)
            .expect("checked_add within test bounds")
    }

    #[test]
    fn first_occurrence_is_kept() {
        let mut cache = HashMap::new();
        let t0 = Instant::now();
        assert!(should_forward_message(
            &mut cache,
            "msg",
            t0,
            WINDOW,
            MAX_TRACKED
        ));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get("msg"), Some(&t0));
    }

    #[test]
    fn duplicate_within_window_is_dropped() {
        let mut cache = HashMap::new();
        let t0 = Instant::now();
        assert!(should_forward_message(
            &mut cache,
            "msg",
            t0,
            WINDOW,
            MAX_TRACKED
        ));

        let t1 = future_at(t0, Duration::from_secs(10));
        assert!(!should_forward_message(
            &mut cache,
            "msg",
            t1,
            WINDOW,
            MAX_TRACKED
        ));

        // Duplicates do not refresh the window: it stays anchored at the
        // kept occurrence.
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get("msg"), Some(&t0));
    }

    #[test]
    fn re_allowed_after_window_elapses() {
        let mut cache = HashMap::new();
        let t0 = Instant::now();
        assert!(should_forward_message(
            &mut cache,
            "msg",
            t0,
            WINDOW,
            MAX_TRACKED
        ));

        let t1 = future_at(t0, Duration::from_secs(61));
        assert!(should_forward_message(
            &mut cache,
            "msg",
            t1,
            WINDOW,
            MAX_TRACKED
        ));
        assert_eq!(cache.get("msg"), Some(&t1));
    }

    #[test]
    fn boundary_inside_window_still_dropped() {
        let mut cache = HashMap::new();
        let t0 = Instant::now();
        assert!(should_forward_message(
            &mut cache,
            "msg",
            t0,
            WINDOW,
            MAX_TRACKED
        ));

        // Exactly `window` after the kept occurrence the entry has expired
        // (age == window is not < window), so the key is forwardable again.
        let t1 = future_at(t0, WINDOW);
        assert!(should_forward_message(
            &mut cache,
            "msg",
            t1,
            WINDOW,
            MAX_TRACKED
        ));
        // A repeat 59s later (age < window) is within the window: dropped.
        let t2 = future_at(t1, Duration::from_secs(59));
        assert!(!should_forward_message(
            &mut cache,
            "msg",
            t2,
            WINDOW,
            MAX_TRACKED
        ));
    }

    #[test]
    fn distinct_messages_are_independent() {
        let mut cache = HashMap::new();
        let t0 = Instant::now();
        assert!(should_forward_message(
            &mut cache,
            "a",
            t0,
            WINDOW,
            MAX_TRACKED
        ));
        let t1 = future_at(t0, Duration::from_secs(1));
        assert!(should_forward_message(
            &mut cache,
            "b",
            t1,
            WINDOW,
            MAX_TRACKED
        ));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn expired_entries_are_evicted_on_new_events() {
        let mut cache = HashMap::new();
        let t0 = Instant::now();
        assert!(should_forward_message(
            &mut cache,
            "old",
            t0,
            WINDOW,
            MAX_TRACKED
        ));

        let t1 = future_at(t0, Duration::from_secs(120));
        assert!(should_forward_message(
            &mut cache,
            "new",
            t1,
            WINDOW,
            MAX_TRACKED
        ));
        assert!(!cache.contains_key("old"));
        assert!(cache.contains_key("new"));
    }

    #[test]
    fn oldest_entries_evicted_beyond_cap() {
        let mut cache = HashMap::new();
        let t0 = Instant::now();
        assert!(should_forward_message(&mut cache, "a", t0, WINDOW, 2));
        let t1 = future_at(t0, Duration::from_secs(1));
        assert!(should_forward_message(&mut cache, "b", t1, WINDOW, 2));
        let t2 = future_at(t1, Duration::from_secs(1));
        assert!(should_forward_message(&mut cache, "c", t2, WINDOW, 2));

        assert_eq!(cache.len(), 2);
        assert!(!cache.contains_key("a"));
        assert!(cache.contains_key("b"));
        assert!(cache.contains_key("c"));
    }
}
