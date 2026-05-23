//! Sentry observability integration for StrikeHub.
//!
//! Provides error reporting, tracing, and context enrichment via Sentry.
//! DSN and other config are set at compile time via `build-defaults.toml`.

use sentry::ClientInitGuard;

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

/// Compile-time traces sample rate from build-defaults.toml.
fn sentry_traces_sample_rate() -> f32 {
    option_env!("STRIKEHUB_SENTRY_TRACES_SAMPLE_RATE")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.2)
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

    let traces_sample_rate = sentry_traces_sample_rate();
    let environment = sentry_environment();

    let guard = sentry::init((
        dsn,
        sentry::ClientOptions {
            release: sentry::release_name!(),
            environment: Some(environment.into()),
            traces_sample_rate,
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
        "Sentry initialized: env={}, mode={}, sample_rate={}",
        environment,
        mode,
        traces_sample_rate
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

// ── Metrics helpers ────────────────────────────────────────────────────

/// Increment a counter metric by 1.
pub fn incr(name: &'static str) {
    sentry::Hub::current().capture_metric(sentry::metrics::counter(name, 1.0));
}

/// Record a distribution (histogram) value.
pub fn distribution(name: &'static str, value: f64) {
    sentry::Hub::current().capture_metric(sentry::metrics::distribution(name, value));
}

/// Set a gauge value.
pub fn gauge(name: &'static str, value: f64) {
    sentry::Hub::current().capture_metric(sentry::metrics::gauge(name, value));
}
