//! Sentry observability integration for StrikeHub.
//!
//! Provides error reporting, tracing, and context enrichment via Sentry.
//! DSN and other config are set at compile time via `build-defaults.toml`.

use sentry::{ClientInitGuard, SessionMode, TransactionContext};
use std::time::Duration;

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

/// Build the [`sentry::ClientOptions`] used by [`init_sentry`].
///
/// Kept separate from [`init_sentry`] so tests can exercise the exact
/// production options with a capturing transport substituted in.
fn client_options() -> sentry::ClientOptions {
    sentry::ClientOptions {
        release: sentry::release_name!(),
        environment: Some(sentry_environment().into()),
        traces_sampler: Some(std::sync::Arc::new(traces_sampler)),
        // Release Health: emit a session per app run so Sentry can compute
        // DAU/WAU, crash-free session rate, and adoption per release without
        // any custom instrumentation.
        //
        // This only works if the workspace `sentry` dependency enables the
        // `release-health` feature. `default-features = false` strips it, and
        // without it sentry-rust compiles out the session machinery entirely,
        // silently ignoring these settings (no session envelopes are sent).
        auto_session_tracking: true,
        session_mode: SessionMode::Application,
        before_send: Some(std::sync::Arc::new(before_send)),
        attach_stacktrace: true,
        ..Default::default()
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

    let guard = sentry::init((dsn, client_options()));

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

/// End the current Release Health session and flush pending data to Sentry.
///
/// Call this on the graceful shutdown paths (window close, server drain)
/// after the application has finished its work and before the
/// [`ClientInitGuard`] returned by [`init_sentry`] is dropped.
///
/// sentry-rust only sends the final `session` update once the session has
/// been ended and the client flushed; a process that is killed (SIGKILL,
/// `_exit` from a signal handler, forced quit) never gets that far, so this
/// explicit end-and-flush is what guarantees a clean run reports a session.
///
/// The flush is synchronous: `guard.flush(Some(timeout))` blocks the calling
/// thread until the transport queue is drained or `timeout` elapses. The
/// desktop binary calls this on the UI thread, so in the worst case (a hung
/// or very slow transport) the UI thread stalls for up to the timeout
/// (currently 5s at the call sites) before the process exits. That stall is
/// accepted as the cost of guaranteeing the final session update is sent.
///
/// Returns `true` if the transport queue was drained within the timeout,
/// or if Sentry is not enabled; `false` on timeout.
pub fn shutdown_sentry(guard: Option<&ClientInitGuard>, timeout: Duration) -> bool {
    let Some(guard) = guard.filter(|g| g.is_enabled()) else {
        tracing::debug!("Sentry shutdown: not enabled, nothing to flush");
        return true;
    };

    sentry::end_session();
    let drained = guard.flush(Some(timeout));
    tracing::info!("Sentry session ended, transport drained={}", drained);
    drained
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

#[cfg(test)]
mod tests {
    use super::*;
    use sentry::protocol::{Envelope, EnvelopeItem, SessionStatus, SessionUpdate};
    use std::sync::{Arc, Mutex};

    /// Serializes tests that share sentry's global hub.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Synthetic DSN for tests (no real project, no PII).
    const TEST_DSN: &str = "https://key@example.test/1";

    /// Transport that records envelopes instead of sending them to the
    /// network. `Arc<Transport>` is a valid `TransportFactory`, so it can be
    /// placed directly into `ClientOptions.transport`.
    #[derive(Debug, Default)]
    struct CaptureTransport {
        envelopes: Mutex<Vec<Envelope>>,
    }

    impl CaptureTransport {
        fn envelopes(&self) -> Vec<Envelope> {
            self.envelopes.lock().unwrap().clone()
        }

        fn session_updates(&self) -> Vec<SessionUpdate<'static>> {
            self.envelopes
                .lock()
                .unwrap()
                .iter()
                .flat_map(|envelope| envelope.items())
                .filter_map(|item| match item {
                    EnvelopeItem::SessionUpdate(session) => Some(session.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    impl sentry::Transport for CaptureTransport {
        fn send_envelope(&self, envelope: Envelope) {
            self.envelopes.lock().unwrap().push(envelope);
        }
    }

    /// Initialize with the exact production options (see `client_options`),
    /// swapping in the capturing transport and a synthetic DSN.
    fn init_with_capture_transport(transport: Arc<CaptureTransport>) -> ClientInitGuard {
        let options = client_options().dsn(TEST_DSN).transport(transport);
        sentry::init((TEST_DSN, options))
    }

    /// Dropping the guard returned by `init_sentry` (i.e. `main` returning
    /// after a clean exit such as a window close) must send the final
    /// session update for the Application-mode session started by
    /// `sentry::init`. Without the `release-health` feature enabled on the
    /// workspace `sentry` dependency, no session is ever started and this
    /// test fails with zero session envelopes.
    #[test]
    fn session_update_sent_when_guard_dropped() {
        let _lock = TEST_LOCK.lock().unwrap();
        let transport = Arc::new(CaptureTransport::default());
        let guard = init_with_capture_transport(transport.clone());
        assert!(guard.is_enabled());

        // Simulate the end of the run: main returns and the guard is dropped.
        drop(guard);

        let sessions = transport.session_updates();
        assert!(
            sessions
                .iter()
                .any(|s| s.init && s.status == SessionStatus::Exited && s.errors == 0),
            "expected an initial session update with status Exited, got {sessions:?}"
        );
    }

    /// The graceful shutdown path used by both binaries: explicitly end the
    /// session and flush before the guard is dropped. The session update
    /// must be in the captured envelopes immediately after, without waiting
    /// for the 60s session-flusher interval or the guard's drop.
    #[test]
    fn shutdown_sentry_sends_session_update_before_guard_drop() {
        let _lock = TEST_LOCK.lock().unwrap();
        let transport = Arc::new(CaptureTransport::default());
        let guard = init_with_capture_transport(transport.clone());
        assert!(guard.is_enabled());

        assert!(shutdown_sentry(Some(&guard), Duration::from_secs(2)));

        let sessions = transport.session_updates();
        assert!(
            sessions
                .iter()
                .any(|s| s.init && s.status == SessionStatus::Exited && s.errors == 0),
            "expected a session update with status Exited, got {sessions:?}"
        );
        // The session must be attributed to the release so it shows up in
        // per-release release health. `release_name!()` is always Some in
        // this crate (CARGO_PKG_NAME/CARGO_PKG_VERSION are set by cargo).
        let release = sentry::release_name!().expect("release name always set in tests");
        assert!(
            sessions.iter().any(|s| s.attributes.release == release),
            "session update missing release attribute: {sessions:?}"
        );

        drop(guard);
    }

    /// `shutdown_sentry` must be a no-op that does not panic when Sentry was
    /// never initialized (no DSN configured, `init_sentry` returned None).
    #[test]
    fn shutdown_sentry_without_guard_is_noop() {
        let _lock = TEST_LOCK.lock().unwrap();
        assert!(shutdown_sentry(None, Duration::from_secs(1)));
    }

    /// An unhandled panic must be reported as an event, and in Application
    /// mode the (now crashed) session update must ride in the *same*
    /// envelope, so a panicking run counts toward crash-free sessions. This
    /// is the offline half of issue #73 acceptance #2: it proves the
    /// crashed session is attached to the panic event, without a network.
    ///
    /// sentry's panic hook (installed once per process by the default
    /// `PanicIntegration` during `sentry::init`) captures on the currently
    /// bound client and flushes before returning, so by the time
    /// `catch_unwind` returns the envelope is already in the capture
    /// transport (the transport sends synchronously; the hook's
    /// `client.flush(None)` is a no-op for it).
    #[test]
    fn crashed_session_rides_panic_envelope() {
        let _lock = TEST_LOCK.lock().unwrap();
        let transport = Arc::new(CaptureTransport::default());
        let guard = init_with_capture_transport(transport.clone());
        assert!(guard.is_enabled());

        // Trigger an unhandled panic on this thread. The sentry hook runs
        // during unwinding (before `catch_unwind` catches the payload);
        // the previously installed default hook then runs and prints to
        // stderr, which is expected test noise.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            panic!("offline test panic for crashed-session envelope");
        }));

        let envelopes = transport.envelopes();
        assert!(
            envelopes.iter().any(|envelope| {
                let items: Vec<_> = envelope.items().collect();
                items.iter().any(|item| matches!(item, EnvelopeItem::Event(_)))
                    && items.iter().any(|item| {
                        matches!(item, EnvelopeItem::SessionUpdate(s) if s.status == SessionStatus::Crashed)
                    })
            }),
            "expected a panic event envelope carrying a Crashed session update, got {envelopes:?}"
        );

        drop(guard);
    }
}
