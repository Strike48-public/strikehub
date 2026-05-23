//! StrikeHub Server
//!
//! Serves the StrikeHub UI as a web application via Dioxus liveview.
//! Manages connector lifecycle, auth proxy, and WebSocket relay headlessly.
//!
//! Environment variables:
//! - `PORT` — Listen port (default: 8080)
//! - `RUST_LOG` — Log filter (default: info)
//! - `STRIKE48_API_URL` — Strike48 API / Keycloak server URL
//! - `STRIKE48_URL` — Strike48 gRPC connector gateway URL
//! - `TENANT_ID` — Matrix tenant identifier
//! - `INSTANCE_ID` — Stable connector instance name
//! - `MATRIX_TLS_INSECURE` — Accept self-signed certs (true/1)
//! - `KUBESTUDIO_AI` — Enable AI features in connectors
//! - `KUBESTUDIO_MODE` — Permission mode (read/write)

use axum::{
    Router,
    extract::{Path, Query},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use dioxus_liveview::LiveviewRouter as _;
use http::StatusCode;
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::signal;

async fn health() -> &'static str {
    "OK"
}

/// Proxy connector content through the IPC bridge so the browser can load
/// connector liveview pages over plain HTTP instead of the desktop-only
/// `dioxus://` scheme.
///
/// Matches `/connector/{*path}` where path is e.g. `kubestudio/liveview`.
async fn handle_connector(
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    // path = "kubestudio/liveview" or "kubestudio/assets/foo.js" etc.
    // Preserve query string so bridge tokens (__st, etc.) are forwarded.
    let uri = if query.is_empty() {
        format!("connector://{}", path)
    } else {
        let qs: String = query
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");
        format!("connector://{}?{}", path, qs)
    };

    let Some(state) = sh_ui::get_bridge_state() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "bridge state not initialised",
        )
            .into_response();
    };

    let (status, headers, body) = sh_core::bridge::handle_bridge_request(state, &uri).await;

    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut resp = Response::builder().status(status);
    for (k, v) in &headers {
        resp = resp.header(k.as_str(), v.as_str());
    }
    resp.body(body.into()).unwrap_or_else(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to build response",
        )
            .into_response()
    })
}

#[tokio::main]
async fn main() {
    // Initialize Sentry before tracing so panics are captured.
    #[cfg(feature = "sentry")]
    let _sentry_guard = sh_core::sentry_init::init_sentry(sh_core::sentry_init::AppMode::Server);

    // Build the tracing subscriber with optional Sentry layer
    #[cfg(feature = "sentry")]
    {
        use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .with(tracing_subscriber::fmt::layer())
            .with(sentry_tracing::layer())
            .init();
    }

    #[cfg(not(feature = "sentry"))]
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    #[cfg(feature = "sentry")]
    sh_core::sentry_init::incr("app.launches");

    // Load config and initialise the allowlist before fetching binaries.
    let cfg = sh_core::HubConfig::load().unwrap_or_else(|e| {
        tracing::warn!("failed to load config, using defaults: {}", e);
        sh_core::HubConfig {
            setup_complete: false,
            pick_tos_accepted: false,
            connectors: Default::default(),
            instance_ids: Default::default(),
            studio_url: None,
            allowlist: Default::default(),
            dynamic_connectors: Vec::new(),
        }
    });
    let config_sources = if cfg.allowlist.sources.is_empty() {
        None
    } else {
        Some(cfg.allowlist.sources.as_slice())
    };
    sh_core::init_allowlist(config_sources);

    // Fetch/update connector binaries before starting the server.
    // This blocks startup to ensure binaries are available for IPC connectors.
    let manifests = sh_core::all_manifests(&cfg);
    let fetch_results = sh_core::ensure_all_connector_binaries(&manifests).await;
    for (id, result) in &fetch_results {
        match result {
            sh_core::EnsureResult::AlreadyCurrent(p) => {
                tracing::info!("connector '{}': binary up-to-date at {}", id, p.display());
            }
            sh_core::EnsureResult::Downloaded(p) => {
                tracing::info!("connector '{}': downloaded to {}", id, p.display());
            }
            sh_core::EnsureResult::FallbackStale(p, reason) => {
                tracing::warn!(
                    "connector '{}': using stale binary at {} ({})",
                    id,
                    p.display(),
                    reason
                );
            }
            sh_core::EnsureResult::Unavailable(reason) => {
                tracing::warn!("connector '{}': binary unavailable ({})", id, reason);
            }
        }
    }

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // Initialise bridge state so the App component's use_effect hooks can
    // register connector sockets (even though there's no Wry custom protocol
    // handler in server mode, IPC connectors still register here).
    let bridge_state = sh_core::new_bridge_state();
    sh_ui::set_bridge_state(bridge_state);

    let router = Router::new()
        .route("/connector/*path", get(handle_connector))
        .with_app("/", sh_ui::App)
        .route("/", get(|| async { Redirect::temporary("/liveview") }))
        .route("/health", get(health));

    tracing::info!("StrikeHub server listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind to address");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

    // Axum has stopped accepting connections. The Dioxus component tree
    // will be torn down, dropping IpcConnectorRunner handles which kill
    // and reap child connector processes via their Drop impl.
    tracing::info!("Server stopped, connector cleanup via Drop");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to listen for ctrl+c");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received SIGINT, shutting down"),
        _ = terminate => tracing::info!("Received SIGTERM, shutting down"),
    }
}
