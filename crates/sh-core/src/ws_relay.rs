use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, Query, State, WebSocketUpgrade, ws},
    response::Response,
    routing::get,
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as TsMessage;

use crate::auth::AuthManager;
use crate::bridge::SharedBridgeState;

struct RelayState {
    bridge: SharedBridgeState,
    auth: Option<AuthManager>,
    tls_insecure: bool,
}

/// A single-port WebSocket bridge that relays connections to Unix sockets
/// (for IPC connectors) or to the Matrix GraphQL socket.
pub struct WsRelay {
    port: u16,
}

impl WsRelay {
    /// Start the WS relay on an ephemeral TCP port.
    ///
    /// Routes:
    /// - `/ws/graphql?token=…&vsn=…` → Matrix upstream
    /// - `/ws/:connector_id` → Unix socket for that connector
    /// - `/ws/:connector_id/*path` → Unix socket with sub-path
    pub async fn start(
        bridge: SharedBridgeState,
        auth: Option<AuthManager>,
    ) -> anyhow::Result<Self> {
        let tls_insecure = std::env::var("MATRIX_TLS_INSECURE")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();

        let state = Arc::new(RelayState {
            bridge,
            auth,
            tls_insecure,
        });

        let app = Router::new()
            .route("/ws/graphql", get(handle_graphql_ws))
            .route("/ws/:connector_id", get(handle_connector_ws))
            .route("/ws/:connector_id/*path", get(handle_connector_ws_path))
            .with_state(state);

        tracing::info!("WsRelay listening on ws://127.0.0.1:{}", port);

        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("WsRelay server error: {}", e);
            }
        });

        Ok(Self { port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

// ── GraphQL WS relay (same as proxy.rs) ────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct GraphqlWsParams {
    token: Option<String>,
    vsn: Option<String>,
}

async fn handle_graphql_ws(
    upgrade: WebSocketUpgrade,
    Query(params): Query<GraphqlWsParams>,
    State(state): State<Arc<RelayState>>,
) -> Response {
    upgrade.on_upgrade(move |client_ws| async move {
        if let Err(e) = run_graphql_ws_relay(client_ws, params, state).await {
            tracing::error!("WsRelay graphql error: {}", e);
        }
    })
}

async fn run_graphql_ws_relay(
    client_ws: ws::WebSocket,
    params: GraphqlWsParams,
    state: Arc<RelayState>,
) -> anyhow::Result<()> {
    let auth = state
        .auth
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no auth configured"))?;

    let base = auth.matrix_url().trim_end_matches('/').to_string();
    let scheme = if base.starts_with("https") {
        "wss"
    } else {
        "ws"
    };
    let host = base
        .trim_start_matches("https://")
        .trim_start_matches("http://");

    let token = params.token.unwrap_or_default();
    let vsn = params.vsn.unwrap_or_else(|| "2.0.0".into());
    let upstream_url = format!(
        "{}://{}/v1alpha/graphql_socket/websocket?token={}&vsn={}",
        scheme,
        host,
        urlencoding::encode(&token),
        vsn
    );

    let tls = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(state.tls_insecure)
        .build()?;
    let connector = tokio_tungstenite::Connector::NativeTls(tls);

    let (upstream_ws, _) = tokio_tungstenite::connect_async_tls_with_config(
        &upstream_url,
        None,
        false,
        Some(connector),
    )
    .await?;

    relay_frames(client_ws, upstream_ws).await;
    Ok(())
}

// ── Connector WS relay (Unix socket) ──────────────────────────────────

async fn handle_connector_ws(
    upgrade: WebSocketUpgrade,
    Path(connector_id): Path<String>,
    State(state): State<Arc<RelayState>>,
) -> Response {
    upgrade.on_upgrade(move |client_ws| async move {
        if let Err(e) = run_connector_ws_relay(client_ws, &connector_id, "/ws", &state).await {
            tracing::error!("WsRelay connector '{}' error: {}", connector_id, e);
        }
    })
}

async fn handle_connector_ws_path(
    upgrade: WebSocketUpgrade,
    Path((connector_id, path)): Path<(String, String)>,
    State(state): State<Arc<RelayState>>,
) -> Response {
    let ws_path = format!("/{}", path);
    upgrade.on_upgrade(move |client_ws| async move {
        if let Err(e) = run_connector_ws_relay(client_ws, &connector_id, &ws_path, &state).await {
            tracing::error!("WsRelay connector '{}' path error: {}", connector_id, e);
        }
    })
}

async fn run_connector_ws_relay(
    client_ws: ws::WebSocket,
    connector_id: &str,
    upstream_path: &str,
    state: &RelayState,
) -> anyhow::Result<()> {
    let guard = state.bridge.read().await;
    let ipc_addr = guard
        .sockets
        .get(connector_id)
        .ok_or_else(|| anyhow::anyhow!("unknown connector '{}'", connector_id))?
        .clone();
    drop(guard);

    // Connect WebSocket over IPC using tokio-tungstenite
    let ipc_stream = crate::ipc::IpcStream::connect(&ipc_addr).await?;
    let ws_url = format!("ws://localhost{}", upstream_path);

    let (upstream_ws, _) = tokio_tungstenite::client_async(ws_url, ipc_stream).await?;

    tracing::debug!(
        "WsRelay: connected to connector '{}' via {}",
        connector_id,
        ipc_addr
    );

    relay_frames(client_ws, upstream_ws).await;
    Ok(())
}

// ── Shared bidirectional frame relay ──────────────────────────────────

async fn relay_frames<S>(
    client_ws: ws::WebSocket,
    upstream_ws: tokio_tungstenite::WebSocketStream<S>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut client_sink, mut client_stream) = client_ws.split();
    let (mut upstream_sink, mut upstream_stream) = upstream_ws.split();

    let c2u = async {
        while let Some(msg) = client_stream.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(_) => break,
            };
            let ts_msg = match msg {
                ws::Message::Text(t) => TsMessage::Text(t.to_string()),
                ws::Message::Binary(b) => TsMessage::Binary(b.to_vec()),
                ws::Message::Ping(p) => TsMessage::Ping(p.to_vec()),
                ws::Message::Pong(p) => TsMessage::Pong(p.to_vec()),
                ws::Message::Close(_) => break,
            };
            if upstream_sink.send(ts_msg).await.is_err() {
                break;
            }
        }
    };

    let u2c = async {
        while let Some(msg) = upstream_stream.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(_) => break,
            };
            let ax_msg = match msg {
                TsMessage::Text(t) => ws::Message::Text(t),
                TsMessage::Binary(b) => ws::Message::Binary(b),
                TsMessage::Ping(p) => ws::Message::Ping(p),
                TsMessage::Pong(p) => ws::Message::Pong(p),
                TsMessage::Close(_) => break,
                _ => continue,
            };
            if client_sink.send(ax_msg).await.is_err() {
                break;
            }
        }
    };

    tokio::select! {
        _ = c2u => {},
        _ = u2c => {},
    }
}
