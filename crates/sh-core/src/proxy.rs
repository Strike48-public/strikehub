use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::{FromRequestParts, Path, Query, Request, State, WebSocketUpgrade, ws},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{any, get},
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as TsMessage;

use crate::auth::AuthManager;
use crate::matrix_ws::MatrixWsClient;

struct ProxyState {
    auth: AuthManager,
    http: reqwest::Client,
    /// The port this proxy is listening on, so we can inject ws:// URLs.
    proxy_port: u16,
    tls_insecure: bool,
    /// Persistent Absinthe WS client for forwarding GraphQL through the
    /// WebSocket endpoint (which accepts PKCE JWTs).
    matrix_ws: tokio::sync::RwLock<Option<Arc<MatrixWsClient>>>,
}

/// HTTP proxy that fetches connector liveview HTML, injects the Matrix auth
/// token and rewrites the Dioxus WebSocket URL so it connects directly to the
/// connector (no WS proxying needed).
pub struct ConnectorProxy {
    port: u16,
    state: Arc<ProxyState>,
}

impl ConnectorProxy {
    /// Start the proxy server on an ephemeral port. Returns immediately after
    /// binding; the server runs in a background tokio task.
    pub async fn start(auth: AuthManager) -> anyhow::Result<Self> {
        let tls_insecure = std::env::var("MATRIX_TLS_INSECURE")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(tls_insecure)
            .build()?;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();

        let state = Arc::new(ProxyState {
            auth,
            http,
            proxy_port: port,
            tls_insecure,
            matrix_ws: tokio::sync::RwLock::new(None),
        });

        let app = Router::new()
            .route("/c/:port/liveview", get(handle_liveview))
            .route("/c/:port/*path", get(handle_passthrough))
            .route("/ws/graphql", get(handle_graphql_ws))
            // Proxy /api/v1alpha → /v1alpha/graphql with the Keycloak JWT
            // as a ?token= query parameter (matching the WebSocket relay pattern).
            .route("/api/v1alpha", any(handle_app_graphql_rewrite))
            .route("/api/v1alpha/*path", any(handle_app_graphql_rewrite))
            // Matrix reverse-proxy (kept for future /app-content/ support).
            .route("/matrix/*path", any(handle_matrix_proxy))
            .with_state(state.clone());

        tracing::info!("Auth proxy listening on http://127.0.0.1:{}", port);

        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("Auth proxy server error: {}", e);
            }
        });

        Ok(Self { port, state })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Attach a persistent Absinthe WS client so that GraphQL requests
    /// are forwarded through the WebSocket instead of HTTP POST.
    pub async fn set_matrix_ws(&self, ws: Arc<MatrixWsClient>) {
        *self.state.matrix_ws.write().await = Some(ws);
    }

    /// Detach the WS client (e.g. on sign-out) so stale connections
    /// don't interfere with the next sign-in.
    pub async fn clear_matrix_ws(&self) {
        *self.state.matrix_ws.write().await = None;
    }
}

/// Fetch connector liveview HTML, inject auth token + rewrite WS URL.
async fn handle_liveview(
    Path(connector_port): Path<u16>,
    State(state): State<Arc<ProxyState>>,
) -> Response {
    let url = format!("http://127.0.0.1:{}/liveview", connector_port);

    let resp = match state.http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(
                "Failed to fetch liveview from port {}: {}",
                connector_port,
                e
            );
            return (StatusCode::BAD_GATEWAY, "Failed to reach connector").into_response();
        }
    };

    let status = resp.status();
    if !status.is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            format!("Connector returned {}", status),
        )
            .into_response();
    }

    let html = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to read liveview body: {}", e);
            return (StatusCode::BAD_GATEWAY, "Failed to read connector response").into_response();
        }
    };

    let rewritten = rewrite_html(&html, &state.auth, connector_port, state.proxy_port);
    Html(rewritten).into_response()
}

/// Pass-through proxy for static assets (CSS, JS, WASM, etc.).
async fn handle_passthrough(
    Path((connector_port, path)): Path<(u16, String)>,
    State(state): State<Arc<ProxyState>>,
) -> Response {
    let url = format!("http://127.0.0.1:{}/{}", connector_port, path);

    let resp = match state.http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Proxy pass-through failed for {}: {}", url, e);
            return (StatusCode::BAD_GATEWAY, "Failed to reach connector").into_response();
        }
    };

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Failed to read pass-through body: {}", e);
            return (StatusCode::BAD_GATEWAY, "Failed to read response").into_response();
        }
    };

    (status, [(header::CONTENT_TYPE, content_type)], bytes).into_response()
}

/// Rewrite HTML to inject auth token, API URL, WS proxy URL, and fix the Dioxus WebSocket URL.
fn rewrite_html(html: &str, auth: &AuthManager, connector_port: u16, proxy_port: u16) -> String {
    let mut result = html.to_string();

    // Rewrite __dioxusGetWsUrl to point directly at the connector
    let replacement_fn = format!(
        r#"function __dioxusGetWsUrl(path) {{
      return "ws://127.0.0.1:{}" + path;
    }}"#,
        connector_port
    );

    let re = regex::Regex::new(
        r#"function __dioxusGetWsUrl\(path\) \{[\s\S]*?return new_url;[\s\S]*?\}"#,
    );

    if let Ok(re) = re
        && re.is_match(&result)
    {
        tracing::debug!(
            "Rewriting __dioxusGetWsUrl for connector port {}",
            connector_port
        );
        result = re.replace(&result, replacement_fn.as_str()).to_string();
    }

    // Inject styles and scripts before </head>
    let mut injected = String::new();

    // Ensure the iframe body fills the viewport so flex layouts (e.g. Three.js canvas) work
    injected.push_str(
        "<style>html, body { height: 100%; margin: 0; overflow: hidden; } #main { height: 100%; }</style>",
    );

    // Auth token + API URL injection so connectors can reach the Matrix API.
    // Prefer sandbox token (accepted by /api/v1alpha) over raw Keycloak JWT.
    let token = auth.api_token();
    if !token.is_empty() {
        let escaped = token.replace('\\', "\\\\").replace('\'', "\\'");
        injected.push_str(&format!(
            "<script>window.__MATRIX_SESSION_TOKEN__ = '{}';</script>",
            escaped
        ));
    }

    // Point __MATRIX_API_URL__ at the local proxy so TCP connectors route
    // their GraphQL calls through us (proxy → WS → Matrix), which accepts
    // PKCE JWTs via verify_token_with_recovery.
    let api_url = format!("http://127.0.0.1:{}", proxy_port);
    let escaped_url = api_url.replace('\\', "\\\\").replace('\'', "\\'");
    injected.push_str(&format!(
        "<script>window.__MATRIX_API_URL__ = '{}';</script>",
        escaped_url
    ));

    // WebSocket proxy URL so connectors with client-side subscriptions can
    // reach the Matrix GraphQL socket through us (avoiding TLS cert issues).
    injected.push_str(&format!(
        "<script>window.__MATRIX_WS_URL__ = 'ws://127.0.0.1:{}/ws/graphql';</script>",
        proxy_port
    ));

    if !injected.is_empty() {
        if let Some(head_end) = result.find("</head>") {
            result.insert_str(head_end, &injected);
        } else if let Some(body_start) = result.find("<body") {
            result.insert_str(body_start, &injected);
        }
    }

    result
}

/// Query parameters forwarded from the client WebSocket URL.
#[derive(Debug, serde::Deserialize)]
struct GraphqlWsParams {
    token: Option<String>,
    vsn: Option<String>,
}

/// WebSocket proxy for the Matrix GraphQL subscription socket.
///
/// The client (e.g. Three.js subscription manager) connects to
///   `ws://127.0.0.1:PROXY/ws/graphql?token=...&vsn=2.0.0`
/// and we relay to
///   `wss://MATRIX_HOST/v1alpha/graphql_socket/websocket?token=...&vsn=2.0.0`
/// using a TLS connector that honours `MATRIX_TLS_INSECURE`.
async fn handle_graphql_ws(
    upgrade: WebSocketUpgrade,
    Query(params): Query<GraphqlWsParams>,
    State(state): State<Arc<ProxyState>>,
) -> Response {
    upgrade.on_upgrade(move |client_ws| async move {
        if let Err(e) = run_graphql_ws_proxy(client_ws, params, state).await {
            tracing::error!("GraphQL WS proxy error: {}", e);
        }
    })
}

async fn run_graphql_ws_proxy(
    client_ws: ws::WebSocket,
    params: GraphqlWsParams,
    state: Arc<ProxyState>,
) -> anyhow::Result<()> {
    // Build the upstream URL
    let base = state.auth.matrix_url().trim_end_matches('/').to_string();
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

    tracing::debug!(
        "GraphQL WS proxy connecting to: {}",
        upstream_url.split('?').next().unwrap_or(&upstream_url)
    );

    // Build TLS connector that respects MATRIX_TLS_INSECURE
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

    tracing::debug!("GraphQL WS proxy: upstream connected");

    // Split both sides and relay frames
    let (mut client_sink, mut client_stream) = client_ws.split();
    let (mut upstream_sink, mut upstream_stream) = upstream_ws.split();

    // Client → upstream
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

    // Upstream → client
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

    tracing::debug!("GraphQL WS proxy: connection closed");
    Ok(())
}

// ── App GraphQL → v1alpha rewrite ─────────────────────────────────────────

/// Proxy `/api/v1alpha` requests to Matrix's `/api/v1alpha/graphql`.
///
/// If a persistent Absinthe WS client is available, queries are forwarded
/// through the WebSocket (which accepts PKCE JWTs via `verify_token_with_recovery`).
/// Falls back to direct HTTP POST if the WS path fails or is unavailable.
async fn handle_app_graphql_rewrite(
    State(state): State<Arc<ProxyState>>,
    req: Request<Body>,
) -> Response {
    // Read the body first (needed for both WS and HTTP paths)
    let (parts, body) = req.into_parts();
    let body_bytes = match axum::body::to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("GraphQL proxy: failed to read body: {}", e);
            return (StatusCode::BAD_REQUEST, "Bad request").into_response();
        }
    };

    // Try WebSocket path first (accepts PKCE JWTs)
    if let Some(ws) = state.matrix_ws.read().await.as_ref() {
        match ws.query(&body_bytes).await {
            Ok(response) => {
                tracing::info!("GraphQL proxy: WS response (body len: {})", response.len());
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json".to_string())],
                    response,
                )
                    .into_response();
            }
            Err(e) => {
                tracing::warn!(
                    "GraphQL proxy: WS query failed, falling back to HTTP: {}",
                    e
                );
            }
        }
    }

    // HTTP fallback
    let base = state.auth.matrix_url().trim_end_matches('/').to_string();
    let token = state.auth.api_token();
    let upstream_url = format!("{}/api/v1alpha/graphql", base);

    let mut builder = state.http.post(&upstream_url);

    if !token.is_empty() {
        builder = builder.header("Authorization", format!("Bearer {}", token));
    }

    // Forward Content-Type
    if let Some(ct) = parts.headers.get(header::CONTENT_TYPE) {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }

    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes.to_vec());
    }

    tracing::info!(
        "GraphQL proxy: /api/v1alpha → {}/api/v1alpha/graphql (Bearer, token len: {}, body len: {})",
        base,
        token.len(),
        body_bytes.len()
    );

    let resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("GraphQL proxy: upstream failed: {}", e);
            return (StatusCode::BAD_GATEWAY, "Failed to reach Matrix").into_response();
        }
    };

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();

    let resp_bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("GraphQL proxy: failed to read response: {}", e);
            return (StatusCode::BAD_GATEWAY, "Failed to read response").into_response();
        }
    };

    tracing::info!(
        "GraphQL proxy: upstream returned {} (body: {})",
        status,
        String::from_utf8_lossy(&resp_bytes.as_ref()[..resp_bytes.len().min(300)])
    );

    (status, [(header::CONTENT_TYPE, content_type)], resp_bytes).into_response()
}

// ── Matrix reverse-proxy ──────────────────────────────────────────────────

/// Reverse-proxy to Matrix.
///
/// - **GET `/matrix/app-content/{appAddress}/`**: adds `Authorization: Bearer`
///   with the Keycloak JWT so Matrix authenticates, serves HTML, and injects a
///   sandbox `__MATRIX_SESSION_TOKEN__`. For HTML responses, a `<base>` tag is
///   injected so relative asset URLs resolve through the proxy.
///
/// - **POST `/matrix/api/v1alpha`** (and other API paths): passes through
///   the request as-is — the JS already carries `Authorization: Bearer {sandbox_token}`.
///
/// - **WebSocket `/matrix/api/app/ws/websocket`**: upgrade is handled by
///   forwarding to the Matrix host with TLS.
async fn handle_matrix_proxy(
    Path(path): Path<String>,
    State(state): State<Arc<ProxyState>>,
    req: Request<Body>,
) -> Response {
    // Check for WebSocket upgrade
    if req.headers().get("upgrade").and_then(|v| v.to_str().ok()) == Some("websocket") {
        return handle_matrix_ws_upgrade(path, state, req).await;
    }

    let method = req.method().clone();
    let base = state.auth.matrix_url().trim_end_matches('/').to_string();
    let upstream_url = format!("{}/{}", base, path);

    tracing::debug!("Matrix proxy: {} {} → {}", method, path, upstream_url);

    // Determine whether we should add the Keycloak Bearer token.
    // For the initial app-content load (GET), we add it so Matrix can authenticate.
    // For subsequent API calls, the request already carries the sandbox token.
    let is_initial_load = method == reqwest::Method::GET && path.starts_with("app-content/");

    let mut builder = state.http.request(method.clone(), &upstream_url);

    if is_initial_load {
        // Inject Keycloak JWT for the initial page load
        let token = state.auth.token();
        if !token.is_empty() {
            builder = builder.header("Authorization", format!("Bearer {}", token));
        }
    } else {
        // Pass through any Authorization header from the client (sandbox token)
        if let Some(auth_header) = req.headers().get("Authorization")
            && let Ok(v) = auth_header.to_str()
        {
            builder = builder.header("Authorization", v);
        }
    }

    // Forward Content-Type for POST requests
    if let Some(ct) = req.headers().get(header::CONTENT_TYPE) {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }

    // Forward Accept header
    if let Some(accept) = req.headers().get(header::ACCEPT) {
        builder = builder.header(header::ACCEPT, accept);
    }

    // Forward request body for POST/PUT
    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Matrix proxy: failed to read request body: {}", e);
            return (StatusCode::BAD_REQUEST, "Failed to read request body").into_response();
        }
    };
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes.to_vec());
    }

    let resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Matrix proxy: upstream request failed: {}", e);
            return (StatusCode::BAD_GATEWAY, "Failed to reach Matrix server").into_response();
        }
    };

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

    // Collect response headers we care about
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let resp_bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Matrix proxy: failed to read upstream response: {}", e);
            return (StatusCode::BAD_GATEWAY, "Failed to read Matrix response").into_response();
        }
    };

    // For HTML responses from app-content, inject a <base> tag so relative
    // asset paths (JS, CSS, images) resolve through our proxy.
    if is_initial_load && content_type.contains("text/html") {
        let html = String::from_utf8_lossy(&resp_bytes);
        let base_href = format!("http://127.0.0.1:{}/matrix/", state.proxy_port);
        let base_tag = format!("<base href=\"{}\" />", base_href);
        let patched = if let Some(pos) = html.find("<head>") {
            format!("{}{}{}", &html[..pos + 6], base_tag, &html[pos + 6..])
        } else if let Some(pos) = html.find("<head ") {
            // <head ...> with attributes
            if let Some(end) = html[pos..].find('>') {
                let insert = pos + end + 1;
                format!("{}{}{}", &html[..insert], base_tag, &html[insert..])
            } else {
                html.to_string()
            }
        } else {
            // Fallback: prepend
            format!("{}{}", base_tag, html)
        };
        return (status, [(header::CONTENT_TYPE, content_type)], patched).into_response();
    }

    (status, [(header::CONTENT_TYPE, content_type)], resp_bytes).into_response()
}

/// Handle WebSocket upgrade for Matrix proxy paths.
async fn handle_matrix_ws_upgrade(
    path: String,
    state: Arc<ProxyState>,
    req: Request<Body>,
) -> Response {
    // Extract query string before consuming the request
    let query = req.uri().query().unwrap_or("").to_string();

    // Extract the WebSocket upgrade from request parts
    let (mut parts, body) = req.into_parts();
    let ws_result = WebSocketUpgrade::from_request_parts(&mut parts, &()).await;
    // Reconstruct unused body (WebSocketUpgrade only needs headers)
    let _ = body;

    let Ok(ws_upgrade) = ws_result else {
        return (StatusCode::BAD_REQUEST, "WebSocket upgrade failed").into_response();
    };

    ws_upgrade.on_upgrade(move |client_ws| async move {
        if let Err(e) = run_matrix_ws_proxy(client_ws, &path, &query, &state).await {
            tracing::error!("Matrix WS proxy error: {}", e);
        }
    })
}

/// Relay WebSocket frames between client and Matrix upstream.
async fn run_matrix_ws_proxy(
    client_ws: ws::WebSocket,
    path: &str,
    query: &str,
    state: &ProxyState,
) -> anyhow::Result<()> {
    let base = state.auth.matrix_url().trim_end_matches('/').to_string();
    let scheme = if base.starts_with("https") {
        "wss"
    } else {
        "ws"
    };
    let host = base
        .trim_start_matches("https://")
        .trim_start_matches("http://");

    let upstream_url = if query.is_empty() {
        format!("{}://{}/{}", scheme, host, path)
    } else {
        format!("{}://{}/{}?{}", scheme, host, path, query)
    };

    tracing::debug!(
        "Matrix WS proxy connecting to: {}",
        upstream_url.split('?').next().unwrap_or(&upstream_url)
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

    tracing::debug!("Matrix WS proxy: upstream connected");

    let (mut client_sink, mut client_stream) = client_ws.split();
    let (mut upstream_sink, mut upstream_stream) = upstream_ws.split();

    let c2u = async {
        while let Some(Ok(msg)) = client_stream.next().await {
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
        while let Some(Ok(msg)) = upstream_stream.next().await {
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

    tracing::debug!("Matrix WS proxy: connection closed");
    Ok(())
}
