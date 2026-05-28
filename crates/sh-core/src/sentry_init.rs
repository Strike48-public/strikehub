//! Sentry observability integration for StrikeHub.
//!
//! Provides error reporting, tracing, and context enrichment via Sentry.
//! DSN and other config are set at compile time via `build-defaults.toml`.

use sentry::{ClientInitGuard, SessionMode, TransactionContext};

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

/// Before-send hook to redact sensitive data from events.
///
/// Redacts:
/// - `authorization` headers
/// - Fields containing `token` in the name
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
