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

use axum::{Router, response::Redirect, routing::get};
use dioxus_liveview::LiveviewRouter as _;
use std::net::SocketAddr;
use tokio::signal;

async fn health() -> &'static str {
    "OK"
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

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
