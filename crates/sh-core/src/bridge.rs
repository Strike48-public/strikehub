use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::auth::AuthManager;
use crate::ipc::IpcAddr;
use crate::ipc_runner::ipc_http_get_full;

/// Shared state for the connector bridge (custom protocol handler).
pub struct BridgeState {
    /// Map of connector_id → IPC address for IPC connectors.
    pub sockets: HashMap<String, IpcAddr>,
    /// Auth manager for token injection.
    pub auth: Option<AuthManager>,
    /// WsRelay port (the single TCP port for WebSocket bridging).
    pub ws_bridge_port: Option<u16>,
    /// Auth proxy port — used to route API calls through the proxy so it can
    /// proxy `/api/v1alpha` → `/v1alpha/graphql` with Keycloak JWT.
    pub proxy_port: Option<u16>,
}

/// Thread-safe shared bridge state.
pub type SharedBridgeState = Arc<RwLock<BridgeState>>;

/// Create a new empty bridge state.
pub fn new_bridge_state() -> SharedBridgeState {
    Arc::new(RwLock::new(BridgeState {
        sockets: HashMap::new(),
        auth: None,
        ws_bridge_port: None,
        proxy_port: None,
    }))
}

/// Handle a request from the `connector://` custom protocol.
///
/// Parses the URI, looks up the Unix socket for the connector, forwards the
/// HTTP request, optionally rewrites HTML, and returns `(status, headers, body)`
/// as plain types so that `sh-core` does not depend on wry.
///
/// The `uri` is the full string from the custom protocol request, e.g.
/// `connector://kubestudio/liveview` or on macOS/WebKit
/// `connector://kubestudio/liveview` (host = connector_id).
pub async fn handle_bridge_request(
    state: &SharedBridgeState,
    uri: &str,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    // Parse: connector://{connector_id}/{path}
    let stripped = uri.strip_prefix("connector://").unwrap_or(uri);
    let (connector_id, path) = match stripped.find('/') {
        Some(idx) => (&stripped[..idx], &stripped[idx..]),
        None => (stripped, "/"),
    };

    let guard = state.read().await;
    let socket_path = match guard.sockets.get(connector_id) {
        Some(p) => p.clone(),
        None => {
            tracing::warn!("bridge: unknown connector '{}'", connector_id);
            let body = format!("Unknown connector: {}", connector_id).into_bytes();
            return (
                404,
                vec![("content-type".into(), "text/plain".into())],
                body,
            );
        }
    };

    let auth = guard.auth.clone();
    let ws_bridge_port = guard.ws_bridge_port;
    let proxy_port = guard.proxy_port;
    drop(guard);

    // Forward request to the connector over IPC
    match ipc_http_get_full(&socket_path, path).await {
        Ok((status, mut headers, body)) => {
            let is_html = headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.contains("text/html"));

            if is_html {
                let html = String::from_utf8_lossy(&body);
                let rewritten = rewrite_html_for_ipc(
                    &html,
                    auth.as_ref(),
                    connector_id,
                    ws_bridge_port,
                    proxy_port,
                );
                // Update content-length to match rewritten body
                headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-length"));
                let rewritten_bytes = rewritten.into_bytes();
                headers.push((
                    "content-length".to_string(),
                    rewritten_bytes.len().to_string(),
                ));
                (status, headers, rewritten_bytes)
            } else {
                (status, headers, body)
            }
        }
        Err(e) => {
            tracing::error!(
                "bridge: failed to reach connector '{}': {}",
                connector_id,
                e
            );
            let body = format!("Bridge error: {}", e).into_bytes();
            (
                502,
                vec![("content-type".into(), "text/plain".into())],
                body,
            )
        }
    }
}

/// Rewrite liveview HTML for IPC mode.
///
/// Same purpose as `proxy.rs::rewrite_html()` but the WebSocket URL points to
/// the WS bridge (`ws://127.0.0.1:{bridge_port}/ws/{connector_id}`).
fn rewrite_html_for_ipc(
    html: &str,
    auth: Option<&AuthManager>,
    connector_id: &str,
    ws_bridge_port: Option<u16>,
    proxy_port: Option<u16>,
) -> String {
    let mut result = html.to_string();

    // Rewrite __dioxusGetWsUrl to point to the WS bridge
    if let Some(port) = ws_bridge_port {
        let replacement_fn = format!(
            r#"function __dioxusGetWsUrl(path) {{
      return "ws://127.0.0.1:{}/ws/{}" + path;
    }}"#,
            port, connector_id
        );

        let re = regex::Regex::new(
            r#"function __dioxusGetWsUrl\(path\) \{[\s\S]*?return new_url;[\s\S]*?\}"#,
        );

        if let Ok(re) = re
            && re.is_match(&result)
        {
            tracing::debug!(
                "bridge: rewriting __dioxusGetWsUrl for connector '{}'",
                connector_id
            );
            result = re.replace(&result, replacement_fn.as_str()).to_string();
        }
    }

    // Build injection block
    let mut injected = String::new();

    // Height-filling CSS
    injected.push_str(
        "<style>html, body { height: 100%; margin: 0; overflow: hidden; } #main { height: 100%; }</style>",
    );

    if let Some(auth) = auth {
        // Auth token — prefer sandbox token over raw Keycloak JWT
        let token = auth.api_token();
        if !token.is_empty() {
            let escaped = token.replace('\\', "\\\\").replace('\'', "\\'");
            injected.push_str(&format!(
                "<script>window.__MATRIX_SESSION_TOKEN__ = '{}';</script>",
                escaped
            ));
        }

        // API URL — point at the local proxy so it can rewrite
        // /api/v1alpha → /v1alpha/graphql with Keycloak JWT.
        // Falls back to the direct Matrix URL if the proxy isn't running.
        let api_url = if let Some(pp) = proxy_port {
            format!("http://127.0.0.1:{}", pp)
        } else {
            auth.matrix_url().to_string()
        };
        if !api_url.is_empty() {
            let escaped_url = api_url.replace('\\', "\\\\").replace('\'', "\\'");
            injected.push_str(&format!(
                "<script>window.__MATRIX_API_URL__ = '{}';</script>",
                escaped_url
            ));
        }
    }

    // WS bridge URL for GraphQL subscriptions
    if let Some(port) = ws_bridge_port {
        injected.push_str(&format!(
            "<script>window.__MATRIX_WS_URL__ = 'ws://127.0.0.1:{}/ws/graphql';</script>",
            port
        ));
    }

    // Inject before </head>
    if !injected.is_empty() {
        if let Some(head_end) = result.find("</head>") {
            result.insert_str(head_end, &injected);
        } else if let Some(body_start) = result.find("<body") {
            result.insert_str(body_start, &injected);
        }
    }

    result
}
