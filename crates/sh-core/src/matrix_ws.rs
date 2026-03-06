use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message as TsMessage;

use crate::auth::AuthManager;

/// A query request sent from callers to the background WS task.
struct WsCommand {
    /// The raw JSON body (GraphQL query/mutation).
    body: Vec<u8>,
    /// Oneshot to return the response JSON.
    reply: oneshot::Sender<Result<String, anyhow::Error>>,
}

/// Persistent Absinthe/Phoenix WebSocket client for forwarding GraphQL
/// queries and mutations through Matrix's WebSocket endpoint (which uses
/// `verify_token_with_recovery` and accepts PKCE JWTs).
pub struct MatrixWsClient {
    tx: mpsc::Sender<WsCommand>,
    /// Held to keep the background task alive.
    _handle: tokio::task::JoinHandle<()>,
}

impl MatrixWsClient {
    /// Connect to Matrix's Absinthe GraphQL socket.
    ///
    /// Joins the `__absinthe__:control` channel, starts a heartbeat loop,
    /// and returns a client that can be shared across threads.
    pub async fn connect(auth: AuthManager) -> anyhow::Result<Self> {
        let ws = Self::connect_ws(&auth).await?;
        let (tx, rx) = mpsc::channel::<WsCommand>(64);

        let handle = tokio::spawn(Self::run_loop(ws, rx, auth));

        Ok(Self {
            tx,
            _handle: handle,
        })
    }

    /// Send a GraphQL query/mutation through the WebSocket and wait for the reply.
    ///
    /// `body` is the raw JSON bytes from the HTTP request body, e.g.
    /// `{"query": "...", "variables": {...}}`.
    pub async fn query(&self, body: &[u8]) -> anyhow::Result<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = WsCommand {
            body: body.to_vec(),
            reply: reply_tx,
        };
        self.tx
            .send(cmd)
            .await
            .map_err(|_| anyhow::anyhow!("WS background task gone"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("WS reply channel dropped"))?
    }

    /// Establish the raw WebSocket connection to Matrix.
    async fn connect_ws(
        auth: &AuthManager,
    ) -> anyhow::Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    > {
        let base = auth.matrix_url().trim_end_matches('/').to_string();
        let scheme = if base.starts_with("https") {
            "wss"
        } else {
            "ws"
        };
        let host = base
            .trim_start_matches("https://")
            .trim_start_matches("http://");

        let token = auth.token();
        let url = format!(
            "{}://{}/v1alpha/graphql_socket/websocket?token={}&vsn=2.0.0",
            scheme,
            host,
            urlencoding::encode(&token)
        );

        tracing::info!(
            "MatrixWsClient: connecting to {}",
            url.split('?').next().unwrap_or(&url)
        );

        let tls = native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(auth.tls_insecure())
            .build()?;
        let connector = tokio_tungstenite::Connector::NativeTls(tls);

        let (ws, _) =
            tokio_tungstenite::connect_async_tls_with_config(&url, None, false, Some(connector))
                .await?;

        tracing::info!("MatrixWsClient: WebSocket connected");
        Ok(ws)
    }

    /// Join the Absinthe control channel. Returns `Ok(())` on successful join reply.
    async fn join_channel<S>(
        ws_sink: &mut S,
        ws_stream: &mut (
                 impl StreamExt<Item = Result<TsMessage, tokio_tungstenite::tungstenite::Error>> + Unpin
             ),
    ) -> anyhow::Result<()>
    where
        S: SinkExt<TsMessage> + Unpin,
        S::Error: std::fmt::Display,
    {
        // Phoenix v2 join: [join_ref, ref, topic, event, payload]
        let join_msg = serde_json::json!(["1", "1", "__absinthe__:control", "phx_join", {}]);
        let join_text = serde_json::to_string(&join_msg)?;
        ws_sink
            .send(TsMessage::Text(join_text))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send join: {}", e))?;

        // Wait for join reply (with timeout)
        let deadline = tokio::time::sleep(std::time::Duration::from_secs(10));
        tokio::pin!(deadline);

        loop {
            tokio::select! {
                msg = ws_stream.next() => {
                    match msg {
                        Some(Ok(TsMessage::Text(text))) => {
                            if let Ok(arr) = serde_json::from_str::<serde_json::Value>(&text) {
                                // Reply: [join_ref, ref, topic, "phx_reply", {"status": "ok", ...}]
                                if arr.get(3).and_then(|v| v.as_str()) == Some("phx_reply")
                                    && arr.get(1).and_then(|v| v.as_str()) == Some("1")
                                {
                                    let status = arr
                                        .get(4)
                                        .and_then(|v| v.get("status"))
                                        .and_then(|v| v.as_str());
                                    if status == Some("ok") {
                                        tracing::info!("MatrixWsClient: Absinthe control channel joined");
                                        return Ok(());
                                    } else {
                                        anyhow::bail!(
                                            "Channel join rejected: {}",
                                            arr.get(4).unwrap_or(&serde_json::Value::Null)
                                        );
                                    }
                                }
                            }
                        }
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => anyhow::bail!("WS error during join: {}", e),
                        None => anyhow::bail!("WS closed during join"),
                    }
                }
                _ = &mut deadline => {
                    anyhow::bail!("Timed out waiting for channel join reply");
                }
            }
        }
    }

    /// Background loop: owns the WS connection, handles commands, heartbeats, and reconnection.
    async fn run_loop(
        ws: tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        mut rx: mpsc::Receiver<WsCommand>,
        auth: AuthManager,
    ) {
        let ref_counter = AtomicU64::new(2); // 1 was used for join
        let mut pending: HashMap<String, oneshot::Sender<Result<String, anyhow::Error>>> =
            HashMap::new();

        let (mut ws_sink, mut ws_stream) = ws.split();

        // Join the control channel
        if let Err(e) = Self::join_channel(&mut ws_sink, &mut ws_stream).await {
            tracing::error!("MatrixWsClient: failed to join channel: {}", e);
            // Drain pending commands with error
            while let Ok(cmd) = rx.try_recv() {
                let _ = cmd.reply.send(Err(anyhow::anyhow!("WS join failed")));
            }
            // Attempt reconnection loop
            Self::reconnect_loop(rx, auth).await;
            return;
        }

        let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(30));
        heartbeat_interval.tick().await; // consume immediate first tick

        loop {
            tokio::select! {
                // Incoming command from a caller
                cmd = rx.recv() => {
                    let Some(cmd) = cmd else {
                        // All senders dropped, shut down
                        tracing::info!("MatrixWsClient: all senders dropped, shutting down");
                        break;
                    };

                    let ref_id = ref_counter.fetch_add(1, Ordering::Relaxed).to_string();

                    // Parse the body to extract query + variables
                    let payload: serde_json::Value = match serde_json::from_slice(&cmd.body) {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = cmd.reply.send(Err(anyhow::anyhow!("Invalid JSON body: {}", e)));
                            continue;
                        }
                    };

                    // Build Absinthe doc message: [null, ref, topic, "doc", payload]
                    let doc_msg = serde_json::json!([
                        serde_json::Value::Null,
                        ref_id,
                        "__absinthe__:control",
                        "doc",
                        payload
                    ]);

                    match serde_json::to_string(&doc_msg) {
                        Ok(text) => {
                            if let Err(e) = ws_sink.send(TsMessage::Text(text)).await {
                                tracing::error!("MatrixWsClient: failed to send doc: {}", e);
                                let _ = cmd.reply.send(Err(anyhow::anyhow!("WS send failed: {}", e)));
                                // Connection likely dead, break to reconnect
                                break;
                            }
                            pending.insert(ref_id, cmd.reply);
                        }
                        Err(e) => {
                            let _ = cmd.reply.send(Err(anyhow::anyhow!("JSON serialize error: {}", e)));
                        }
                    }
                }

                // Incoming WS message from Matrix
                msg = ws_stream.next() => {
                    match msg {
                        Some(Ok(TsMessage::Text(text))) => {
                            if let Ok(arr) = serde_json::from_str::<serde_json::Value>(&text) {
                                let event = arr.get(3).and_then(|v| v.as_str()).unwrap_or("");
                                let msg_ref = arr.get(1).and_then(|v| v.as_str()).map(String::from);

                                match event {
                                    "phx_reply" => {
                                        if let Some(ref_id) = msg_ref
                                            && let Some(reply_tx) =
                                                pending.remove(&ref_id)
                                        {
                                            // Extract the response data
                                            let response = arr
                                                .get(4)
                                                .and_then(|v| v.get("response"))
                                                .cloned()
                                                .unwrap_or(serde_json::Value::Null);
                                            let _ = reply_tx.send(Ok(
                                                serde_json::to_string(&response)
                                                    .unwrap_or_default(),
                                            ));
                                        }
                                    }
                                    "phx_error" => {
                                        tracing::error!("MatrixWsClient: channel error: {}", text);
                                        // Fail all pending
                                        for (_, tx) in pending.drain() {
                                            let _ = tx.send(Err(anyhow::anyhow!("Channel error")));
                                        }
                                        break;
                                    }
                                    "phx_close" => {
                                        tracing::warn!("MatrixWsClient: channel closed by server");
                                        for (_, tx) in pending.drain() {
                                            let _ = tx.send(Err(anyhow::anyhow!("Channel closed")));
                                        }
                                        break;
                                    }
                                    _ => {
                                        // Subscription events or other messages — ignore for now
                                        tracing::trace!("MatrixWsClient: event={} ref={:?}", event, msg_ref);
                                    }
                                }
                            }
                        }
                        Some(Ok(TsMessage::Ping(data))) => {
                            let _ = ws_sink.send(TsMessage::Pong(data)).await;
                        }
                        Some(Ok(TsMessage::Close(_))) | None => {
                            tracing::warn!("MatrixWsClient: WebSocket closed");
                            for (_, tx) in pending.drain() {
                                let _ = tx.send(Err(anyhow::anyhow!("WS connection closed")));
                            }
                            break;
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            tracing::error!("MatrixWsClient: WS error: {}", e);
                            for (_, tx) in pending.drain() {
                                let _ = tx.send(Err(anyhow::anyhow!("WS error: {}", e)));
                            }
                            break;
                        }
                    }
                }

                // Heartbeat
                _ = heartbeat_interval.tick() => {
                    let ref_id = ref_counter.fetch_add(1, Ordering::Relaxed).to_string();
                    let hb = serde_json::json!([
                        serde_json::Value::Null,
                        ref_id,
                        "phoenix",
                        "heartbeat",
                        {}
                    ]);
                    if let Ok(text) = serde_json::to_string(&hb)
                        && let Err(e) = ws_sink.send(TsMessage::Text(text)).await
                    {
                        tracing::warn!("MatrixWsClient: heartbeat send failed: {}", e);
                        break;
                    }
                }
            }
        }

        // Connection lost — enter reconnection loop
        Self::reconnect_loop(rx, auth).await;
    }

    /// Reconnect loop: keeps trying to re-establish the WS connection with backoff.
    async fn reconnect_loop(rx: mpsc::Receiver<WsCommand>, auth: AuthManager) {
        let mut backoff = 2u64;
        loop {
            tracing::info!("MatrixWsClient: reconnecting in {}s...", backoff);
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;

            if !auth.is_authenticated() {
                tracing::info!("MatrixWsClient: not authenticated, stopping reconnect");
                return;
            }

            match Self::connect_ws(&auth).await {
                Ok(ws) => {
                    tracing::info!("MatrixWsClient: reconnected successfully");
                    // Re-enter the main run_loop (tail call via spawn won't work,
                    // so we just call it directly — this is already in a spawned task).
                    Box::pin(Self::run_loop(ws, rx, auth)).await;
                    return;
                }
                Err(e) => {
                    tracing::warn!("MatrixWsClient: reconnect failed: {}", e);
                    backoff = (backoff * 2).min(60);
                }
            }
        }
    }
}
