use axum::http::header;
use axum::{Router, extract::Query, response::Html, routing::get};
use serde::Deserialize;
use sha2::Digest;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Result of a successful OAuth flow.
#[derive(Debug)]
pub struct OAuthResult {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Keycloak token endpoint (for token refresh).
    pub token_endpoint: String,
    /// Keycloak client_id (for token refresh).
    pub client_id: String,
    /// Handle for the OAuth callback HTTP server.  In server mode the server
    /// is kept alive so the port stays bound (kubectl port-forward drops all
    /// ports when any forwarded port closes).  The caller should abort this
    /// handle before starting a new sign-in flow so the port can be reused.
    pub server_handle: Option<tokio::task::JoinHandle<()>>,
}

/// Returned by [`start_oauth_flow_with`] when the user cancels the in-flight
/// sign-in flow from the waiting state (Strike48/project-management#379).
///
/// Distinct from a timeout or other failure so the UI can reset quietly
/// instead of surfacing an error banner.
#[derive(Debug, thiserror::Error)]
#[error("sign-in cancelled")]
pub struct SignInCancelled;

/// How long the flow waits for the browser callback before giving up.
///
/// 15 minutes: first-run users can spend several minutes in browser
/// first-run wizards or account-creation flows before reaching the Keycloak
/// screen (parent finding #375-5).
pub const OAUTH_TIMEOUT_SECS: u64 = 900;

/// Start the system-browser OAuth flow via Matrix + Keycloak PKCE.
///
/// Two-hop flow that ensures both a Matrix session AND usable tokens:
///
/// 1. Discover Keycloak OIDC config via Matrix publicConfig
/// 2. Generate PKCE code_verifier + code_challenge
/// 3. Open browser → `{matrix}/auth/login?redirect=http://127.0.0.1:{port}/session-created`
///    (`{port}` is an OS-assigned ephemeral loopback port in desktop mode)
/// 4. Matrix → Keycloak → user authenticates → Matrix creates session →
///    redirects to `/session-created`
/// 5. `/session-created` immediately redirects browser to Keycloak's auth
///    endpoint with PKCE (Keycloak session already exists → instant redirect)
/// 6. Keycloak → `/cb?code=...`
/// 7. Server-side: exchange code + code_verifier for tokens
///
/// This ensures Matrix has a session (step 4) so that sandbox token
/// bootstrap and GraphQL API calls work, while also giving us the
/// Keycloak JWT and refresh token directly (step 7).
///
/// `open_browser` is called with the login URL. In desktop mode this calls
/// `open::that()`; in server/liveview mode it can use JS eval to open a
/// popup in the user's browser.
///
/// `callback_base_url` is the externally-reachable base URL for the OAuth
/// callback server (e.g. `http://localhost:4000` when port-forwarding to a
/// container). If `None` (desktop mode), defaults to
/// `http://127.0.0.1:{port}` using the bound loopback ephemeral port, and
/// every consumer of the redirect (Keycloak PKCE `redirect_uri`,
/// `/session-created`, the Matrix login URL, and the token-exchange
/// `redirect_uri` form field) is built from that actual bound port.
///
/// While waiting for the browser callback the flow is cancellable (see the
/// `cancel` parameter of [`start_oauth_flow_with`]) and bounded by a
/// 15-minute timeout.
pub async fn start_oauth_flow(matrix_url: &str, tls_insecure: bool) -> anyhow::Result<OAuthResult> {
    start_oauth_flow_with(matrix_url, tls_insecure, None, None, None, None, None).await
}

/// Like [`start_oauth_flow`] but with:
/// - `callback_base_url` — externally-reachable base for OAuth callbacks
///   (e.g. `http://localhost:4000` when port-forwarding to a container).
/// - `browser_matrix_url` — browser-reachable Matrix URL for the login page.
///   In desktop mode this is the same as `matrix_url`. In server/container
///   mode, `matrix_url` may be an internal cluster address while this is
///   the external URL the user's browser can reach.
/// - `login_url_tx` — when provided, the login URL is sent over this channel
///   instead of calling `open::that()`. Allows the caller to open the URL
///   client-side (e.g. via Dioxus `eval` / `window.open()`).
/// - `login_url_out` — when provided, the final login URL (which embeds the
///   bound callback port) is *also* sent over this channel so the caller
///   can offer "Open sign-in page again" / "Copy sign-in link" actions in
///   the waiting state (Strike48/project-management#379).
/// - `cancel` — when provided, firing this channel aborts the callback
///   server and returns [`SignInCancelled`] without surfacing an error.
#[tracing::instrument(
    name = "oauth.flow",
    skip_all,
    fields(
        matrix_url = %matrix_url,
        outcome = tracing::field::Empty,
        mode = tracing::field::Empty,
        duration_ms = tracing::field::Empty,
    )
)]
pub async fn start_oauth_flow_with(
    matrix_url: &str,
    tls_insecure: bool,
    callback_base_url: Option<String>,
    browser_matrix_url: Option<String>,
    login_url_tx: Option<tokio::sync::oneshot::Sender<String>>,
    login_url_out: Option<tokio::sync::oneshot::Sender<String>>,
    cancel: Option<tokio::sync::oneshot::Receiver<()>>,
) -> anyhow::Result<OAuthResult> {
    run_oauth_flow(
        matrix_url,
        tls_insecure,
        callback_base_url,
        browser_matrix_url,
        login_url_tx,
        login_url_out,
        cancel,
        OAUTH_TIMEOUT_SECS,
        |url| open::that(url),
    )
    .await
}

/// Core of the OAuth flow with the two production knobs pinned by
/// [`start_oauth_flow_with`] made injectable for tests: `timeout_secs`
/// (the waiting-state ceiling — [`OAUTH_TIMEOUT_SECS`] in production) and
/// `open_browser` (the system-browser opener — `open::that` on desktop).
/// Tests inject a short timeout and a no-op opener so the cancel/timeout
/// and ephemeral-port behaviour is covered deterministically, without
/// 15-minute sleeps or a real browser (Strike48/project-management#379).
// 9 args = the public 7-arg surface plus the two test-only knobs above.
#[allow(clippy::too_many_arguments)]
async fn run_oauth_flow<O>(
    matrix_url: &str,
    tls_insecure: bool,
    callback_base_url: Option<String>,
    browser_matrix_url: Option<String>,
    login_url_tx: Option<tokio::sync::oneshot::Sender<String>>,
    login_url_out: Option<tokio::sync::oneshot::Sender<String>>,
    cancel: Option<tokio::sync::oneshot::Receiver<()>>,
    timeout_secs: u64,
    open_browser: O,
) -> anyhow::Result<OAuthResult>
where
    O: Fn(&str) -> std::io::Result<()>,
{
    let span = tracing::Span::current();
    let started = std::time::Instant::now();

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(tls_insecure)
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let matrix_base = matrix_url.trim_end_matches('/').to_string();
    // Browser-facing Matrix URL (defaults to same as internal matrix_url)
    let browser_base = browser_matrix_url
        .map(|u| u.trim_end_matches('/').to_string())
        .unwrap_or_else(|| matrix_base.clone());

    // Discover Keycloak from Matrix publicConfig (needed for token_endpoint + client_id)
    let kc = discover_keycloak(&client, &matrix_base).await?;
    tracing::info!(
        "Discovered Keycloak: url={}, realm={}, client_id={}",
        kc.url,
        kc.realm,
        kc.client_id
    );

    let oidc = fetch_oidc_config(&client, &kc.url, &kc.realm).await?;
    tracing::info!(
        "OIDC endpoints: auth={}, token={}",
        oidc.authorization_endpoint,
        oidc.token_endpoint
    );

    // Generate PKCE code_verifier and code_challenge
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);

    let (tx, mut rx) = mpsc::channel::<OAuthResult>(1);

    // Bind the callback server.
    //
    // Desktop (no `callback_base_url`): loopback-only (`127.0.0.1`) on an
    // OS-assigned ephemeral port. The old `0.0.0.0:4000` bind made the
    // sign-in listener reachable from the LAN, triggered OS firewall
    // "publisher unknown" prompts, and could collide with other software
    // on the fixed port (Strike48/project-management#379; parent findings
    // #375-5/#375-7, #376-4, #377-5). Every consumer of the redirect is
    // built below from the *actual* bound port, so an ephemeral port is
    // safe — the Keycloak client's allowed redirect URIs must cover it
    // with a port wildcard (`http://127.0.0.1:*/cb`).
    //
    // Server/port-forward (`callback_base_url` set): kubectl port-forward
    // connects to the pod IP on the fixed port the external base URL
    // advertises, so bind all interfaces on exactly that port (a
    // loopback-only bind would not receive those connections).
    let listener = if let Some(ref base) = callback_base_url {
        let bind_addr = format!("0.0.0.0:{}", callback_port(base));
        tokio::net::TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to bind OAuth callback server to {} (port-forward mode): {}",
                    bind_addr,
                    e
                )
            })?
    } else {
        tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| {
                anyhow::anyhow!("Failed to bind OAuth callback server to 127.0.0.1:0: {}", e)
            })?
    };
    let addr: SocketAddr = listener.local_addr()?;
    tracing::info!("OAuth callback server listening on {}", addr);

    // External base URL for the callback — either provided (server mode)
    // or default to loopback with the bound port (desktop mode).
    let external_base =
        callback_base_url.unwrap_or_else(|| format!("http://127.0.0.1:{}", addr.port()));
    let pkce_redirect_uri = format!("{}/cb", external_base);

    // Build the Keycloak PKCE authorization URL (used by /session-created redirect)
    let keycloak_auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope=openid&code_challenge={}&code_challenge_method=S256",
        oidc.authorization_endpoint,
        urlencoding::encode(&kc.client_id),
        urlencoding::encode(&pkce_redirect_uri),
        urlencoding::encode(&code_challenge),
    );

    // Shared state for the callback handler
    let cb_state = Arc::new(CallbackState {
        client: client.clone(),
        token_endpoint: oidc.token_endpoint.clone(),
        client_id: kc.client_id.clone(),
        redirect_uri: pkce_redirect_uri.clone(),
        code_verifier,
        tx,
        matrix_base_url: browser_base.clone(),
    });

    let keycloak_auth_url_clone = keycloak_auth_url.clone();
    let app = Router::new()
        .route(
            "/session-created",
            get(move || {
                let url = keycloak_auth_url_clone.clone();
                async move {
                    // Matrix session is now created. Redirect to Keycloak's
                    // auth endpoint for PKCE. Since the user just authenticated
                    // via Matrix, Keycloak already has a session — this redirect
                    // is instant (no login prompt).
                    tracing::info!("Matrix session created, redirecting to Keycloak PKCE flow");
                    (
                        [(header::CONNECTION, "close")],
                        axum::response::Redirect::temporary(&url),
                    )
                }
            }),
        )
        .route(
            "/cb",
            get(move |query: Query<CbQuery>| {
                let state = cb_state.clone();
                async move {
                    let resp = handle_oauth_callback(query, state).await;
                    ([(header::CONNECTION, "close")], resp)
                }
            }),
        );

    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    // Build login URL pointing to Matrix's /auth/login endpoint.
    // This goes through Matrix → Keycloak → auth → Matrix creates session →
    // redirects to /session-created → redirects to Keycloak PKCE → /cb?code=...
    let session_created_uri = format!("{}/session-created", external_base);
    let login_url = format!(
        "{}/auth/login?redirect={}",
        browser_base,
        urlencoding::encode(&session_created_uri),
    );

    // Hand the login URL (it embeds the bound callback port) to the caller
    // for "Open sign-in page again" / "Copy sign-in link" actions in the
    // waiting state — before opening the browser, so it is still available
    // if opening the browser fails.
    if let Some(out) = login_url_out {
        let _ = out.send(login_url.clone());
    }

    let is_server_mode = login_url_tx.is_some();
    span.record("mode", if is_server_mode { "server" } else { "desktop" });
    tracing::info!("Opening browser for Matrix login (two-hop: Matrix session + PKCE)");
    if let Some(tx) = login_url_tx {
        // Server mode: send URL back to the caller for client-side opening
        let _ = tx.send(login_url.clone());
        tracing::info!("Login URL sent to caller: {}", login_url);
    } else {
        // Desktop mode: open system browser directly
        if let Err(e) = open_browser(&login_url) {
            tracing::error!("Failed to open system browser: {}", e);
            server_handle.abort();
            anyhow::bail!("Failed to open system browser: {}", e);
        }
    }

    // Wait for the token: 15-minute ceiling, or until the user cancels from
    // the waiting state (Strike48/project-management#379).
    tracing::info!("Waiting for OAuth callback (rx)...");
    let callback_wait =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), rx.recv());
    let result = match cancel {
        Some(cancel) => tokio::select! {
            r = callback_wait => r,
            _ = cancel => {
                span.record("outcome", "cancelled");
                server_handle.abort();
                tracing::info!("OAuth flow cancelled by user");
                return Err(anyhow::anyhow!(SignInCancelled));
            }
        },
        None => callback_wait.await,
    };

    // In server mode, keep the callback server alive so the port stays open
    // (kubectl port-forward drops all ports if any forwarded port closes).
    // The handle is returned to the caller so it can abort the server before
    // starting a new sign-in flow (e.g. on page refresh).
    // In desktop mode, shut it down immediately — it's not needed.
    if !is_server_mode {
        server_handle.abort();
    }

    span.record("duration_ms", started.elapsed().as_millis() as u64);

    match result {
        Ok(Some(mut oauth_result)) => {
            span.record("outcome", "success");
            tracing::info!("OAuth flow completed successfully");
            if is_server_mode {
                oauth_result.server_handle = Some(server_handle);
            }
            Ok(oauth_result)
        }
        Ok(None) => {
            span.record("outcome", "channel_closed");
            server_handle.abort();
            tracing::error!("OAuth callback channel closed unexpectedly");
            anyhow::bail!("OAuth callback channel closed unexpectedly")
        }
        Err(_) => {
            span.record("outcome", "timeout");
            server_handle.abort();
            anyhow::bail!(
                "OAuth flow timed out after 15 minutes. If you finished sign-in in the browser, click Sign In again to start a fresh flow."
            )
        }
    }
}

/// Port advertised by a callback base URL (`http://localhost:4000` → 4000),
/// defaulting to 4000 (the Helm chart's `oauth-cb` container port) when the
/// URL carries no explicit port.
fn callback_port(base: &str) -> u16 {
    base.rsplit(':')
        .next()
        .and_then(|p| p.split('/').next())
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(4000)
}

// ---------------------------------------------------------------------------
// PKCE helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::extract::{Form, State};
    use axum::routing::post;
    use std::time::{Duration, Instant};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;

    #[test]
    fn parses_explicit_port() {
        assert_eq!(callback_port("http://localhost:4000"), 4000);
        assert_eq!(callback_port("http://localhost:9443/"), 9443);
        assert_eq!(callback_port("http://10.0.0.5:8443/x"), 8443);
    }

    #[test]
    fn defaults_to_4000_without_port() {
        assert_eq!(callback_port("http://localhost"), 4000);
        assert_eq!(callback_port("http://localhost/"), 4000);
        assert_eq!(callback_port(""), 4000);
    }

    /// The public ceiling is the 15-minute bound the waiting state promises
    /// (parent finding #375-5: first-run browser wizards take minutes).
    #[test]
    fn oauth_timeout_is_15_minutes() {
        assert_eq!(OAUTH_TIMEOUT_SECS, 900);
    }

    /// `SignInCancelled` is the quiet-reset signal the UI downcasts on
    /// (sh-ui/src/app.rs) — it must downcast through `anyhow` and carry a
    /// stable display message.
    #[test]
    fn sign_in_cancelled_downcasts_and_displays() {
        let err: anyhow::Error = anyhow::anyhow!(SignInCancelled);
        assert!(err.downcast_ref::<SignInCancelled>().is_some());
        assert_eq!(err.to_string(), "sign-in cancelled");
    }

    // ------------------------------------------------------------------
    // Mock Matrix + Keycloak (in-process) for full-flow tests
    // ------------------------------------------------------------------

    /// One observed token-exchange POST (form fields), for end-to-end
    /// assertions on the redirect_uri the flow built.
    #[derive(Clone, Default)]
    struct TokenCall {
        grant_type: Option<String>,
        code: Option<String>,
        client_id: Option<String>,
        redirect_uri: Option<String>,
        code_verifier_present: bool,
    }

    /// Shared state for the mock identity stack.
    #[derive(Clone)]
    struct MockIdState {
        base: String,
        token_calls: Arc<std::sync::Mutex<Vec<TokenCall>>>,
    }

    #[derive(Deserialize)]
    struct TokenForm {
        grant_type: String,
        code: String,
        client_id: String,
        redirect_uri: String,
        code_verifier: String,
    }

    async fn graphql_handler(State(s): State<MockIdState>) -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "data": {
                "publicConfig": {
                    "keycloak": {
                        "url": s.base,
                        "realm": "test",
                        "clientId": "strikehub-client"
                    }
                }
            }
        }))
    }

    async fn oidc_handler(State(s): State<MockIdState>) -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "authorization_endpoint": format!(
                "{}/realms/test/protocol/openid-connect/auth",
                s.base
            ),
            "token_endpoint": format!(
                "{}/realms/test/protocol/openid-connect/token",
                s.base
            ),
        }))
    }

    async fn token_handler(
        State(s): State<MockIdState>,
        Form(form): Form<TokenForm>,
    ) -> Json<serde_json::Value> {
        s.token_calls.lock().unwrap().push(TokenCall {
            grant_type: Some(form.grant_type),
            code: Some(form.code),
            client_id: Some(form.client_id),
            redirect_uri: Some(form.redirect_uri),
            code_verifier_present: !form.code_verifier.is_empty(),
        });
        Json(serde_json::json!({
            "access_token": "mock-access-token",
            "refresh_token": "mock-refresh-token",
        }))
    }

    /// Serves `publicConfig` (GraphQL), the OIDC well-known, and the token
    /// endpoint on a random loopback port; returns the shared state so tests
    /// can inspect the recorded token-exchange calls.
    async fn spawn_mock_id_stack() -> MockIdState {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = MockIdState {
            base: base.clone(),
            token_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
        };
        let app = Router::new()
            .route("/api/v1alpha/graphql", post(graphql_handler))
            .route(
                "/realms/test/.well-known/openid-configuration",
                get(oidc_handler),
            )
            .route(
                "/realms/test/protocol/openid-connect/token",
                post(token_handler),
            )
            .with_state(state.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        wait_for_accept(&base).await;
        state
    }

    /// Wait until the mock is actually accepting connections (closes the
    /// spawn→accept race so the flow's discovery never sees ECONNREFUSED).
    async fn wait_for_accept(base: &str) {
        let port = port_of(base);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match TcpStream::connect(("127.0.0.1", port)).await {
                Ok(_) => return,
                Err(_) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(e) => panic!("mock id stack never started accepting: {e}"),
            }
        }
    }

    /// The callback base URL the flow embedded in the login URL's
    /// `redirect=` query param (where the browser lands after Matrix
    /// session creation).
    fn external_base_from_login_url(login_url: &str) -> String {
        let encoded = login_url
            .split("redirect=")
            .nth(1)
            .expect("login URL carries a redirect param");
        let decoded = urlencoding::decode(encoded).expect("redirect param decodes");
        decoded
            .strip_suffix("/session-created")
            .expect("redirect param targets /session-created")
            .to_string()
    }

    fn port_of(base: &str) -> u16 {
        base.rsplit_once(':')
            .expect("base has a port")
            .1
            .parse()
            .expect("port parses")
    }

    /// Wait until `port` on loopback is re-bindable — proves the flow's
    /// callback listener was torn down rather than left as a zombie.
    async fn wait_for_port_free(port: u16) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match TcpListener::bind(("127.0.0.1", port)).await {
                Ok(_) => return,
                Err(_) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(e) => panic!("callback port {port} still bound: {e}"),
            }
        }
    }

    /// No system browser in tests — the production wrapper uses
    /// `open::that`.
    fn no_browser(_url: &str) -> std::io::Result<()> {
        Ok(())
    }

    /// Browser stand-in that never follows the 307 (a real browser would,
    /// but we assert on the redirect itself).
    fn no_follow_client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    // ------------------------------------------------------------------
    // Flow tests (cancel / timeout / ephemeral port / happy path)
    // ------------------------------------------------------------------

    /// Full desktop-mode happy path against the mock Matrix/Keycloak: the
    /// callback port is OS-assigned, the Keycloak PKCE `redirect_uri` and
    /// the token-exchange `redirect_uri` are built from the *actually
    /// bound* port, the callback completes, and the listener is torn down.
    #[tokio::test]
    async fn desktop_happy_path_uses_bound_port_and_completes() {
        let mock = spawn_mock_id_stack().await;
        let (out_tx, out_rx) = oneshot::channel::<String>();

        let flow = run_oauth_flow(
            &mock.base,
            false,
            None, // desktop: no callback base URL -> ephemeral loopback bind
            None,
            None, // desktop: no login_url_tx
            Some(out_tx),
            None, // no cancel: run to the happy path
            30,
            no_browser,
        );
        tokio::pin!(flow);

        let login_url = tokio::select! {
            r = &mut flow => panic!("flow ended before reporting the login URL: {r:?}"),
            url = out_rx => url.expect("flow reports the login URL"),
        };
        let external_base = external_base_from_login_url(&login_url);
        let port = port_of(&external_base);
        assert_eq!(
            external_base,
            format!("http://127.0.0.1:{port}"),
            "desktop mode must expose loopback-only callbacks"
        );
        assert_ne!(port, 0, "desktop mode must use an OS-assigned port");
        assert_ne!(
            port, 4000,
            "desktop mode must not fall back to the fixed 4000"
        );

        // Drive the "browser": Matrix session-created -> Keycloak auth redirect.
        let client = no_follow_client();
        let resp = client
            .get(format!("{external_base}/session-created"))
            .send()
            .await
            .expect("/session-created reachable on the bound port");
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::TEMPORARY_REDIRECT,
            "/session-created redirects to the Keycloak auth endpoint"
        );
        let location = resp
            .headers()
            .get(axum::http::header::LOCATION)
            .expect("Location header")
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            location.contains(&format!(
                "redirect_uri={}",
                urlencoding::encode(&format!("{external_base}/cb"))
            )),
            "Keycloak PKCE redirect_uri must use the bound port: {location}"
        );

        // Keycloak sends the browser back to /cb with the auth code.
        let resp = client
            .get(format!("{external_base}/cb?code=mock-auth-code"))
            .send()
            .await
            .expect("/cb reachable on the bound port");
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.text().await.unwrap();
        assert!(body.contains("Signed in"), "success page expected: {body}");

        let result = flow.await.expect("flow completes");
        assert_eq!(result.access_token, "mock-access-token");
        assert_eq!(result.refresh_token.as_deref(), Some("mock-refresh-token"));
        assert_eq!(
            result.token_endpoint,
            format!("{}/realms/test/protocol/openid-connect/token", mock.base)
        );
        assert_eq!(result.client_id, "strikehub-client");
        assert!(
            result.server_handle.is_none(),
            "desktop mode does not keep the listener alive"
        );

        // The token exchange saw the bound-port redirect_uri (end-to-end).
        {
            let calls = mock.token_calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].grant_type.as_deref(), Some("authorization_code"));
            assert_eq!(calls[0].code.as_deref(), Some("mock-auth-code"));
            assert_eq!(calls[0].client_id.as_deref(), Some("strikehub-client"));
            assert_eq!(
                calls[0].redirect_uri.as_deref(),
                Some(format!("{external_base}/cb").as_ref())
            );
            assert!(calls[0].code_verifier_present);
        }

        // No zombie listener: the ephemeral port is free again.
        wait_for_port_free(port).await;
    }

    /// Cancellation from the waiting state: the flow returns
    /// `SignInCancelled` (the UI's quiet-reset signal) and tears the
    /// callback listener down — the port is re-bindable, no zombie.
    #[tokio::test]
    async fn cancel_returns_sign_in_cancelled_and_tears_down_listener() {
        let mock = spawn_mock_id_stack().await;
        let (out_tx, out_rx) = oneshot::channel::<String>();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

        let flow = run_oauth_flow(
            &mock.base,
            false,
            None,
            None,
            None,
            Some(out_tx),
            Some(cancel_rx),
            30,
            no_browser,
        );
        tokio::pin!(flow);

        let login_url = tokio::select! {
            r = &mut flow => panic!("flow ended before reporting the login URL: {r:?}"),
            url = out_rx => url.expect("flow reports the login URL before waiting"),
        };
        let port = port_of(&external_base_from_login_url(&login_url));
        // Sanity: the listener is actually live while the flow waits.
        TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("callback listener is live while waiting");

        cancel_tx.send(()).expect("cancel channel open");
        let err = flow.await.expect_err("cancel must not complete the flow");
        assert!(
            err.downcast_ref::<SignInCancelled>().is_some(),
            "cancel must return SignInCancelled, got: {err}"
        );

        // The select! cancel branch aborts the callback server before
        // returning — the port must be free again (no zombie listener).
        wait_for_port_free(port).await;
    }

    /// The flow gives up at the (injected) timeout, aborts its callback
    /// listener, and surfaces an actionable error that is NOT
    /// `SignInCancelled`.
    #[tokio::test]
    async fn timeout_fires_aborts_listener_and_returns_actionable_error() {
        let mock = spawn_mock_id_stack().await;
        let (out_tx, out_rx) = oneshot::channel::<String>();

        let flow = run_oauth_flow(
            &mock.base,
            false,
            None,
            None,
            None,
            Some(out_tx),
            None,
            1, // injected short timeout — no 900 s sleeps in tests
            no_browser,
        );
        tokio::pin!(flow);

        let login_url = tokio::select! {
            r = &mut flow => panic!("flow ended before reporting the login URL: {r:?}"),
            url = out_rx => url.expect("flow reports the login URL before waiting"),
        };
        let port = port_of(&external_base_from_login_url(&login_url));

        let started = Instant::now();
        let err = flow.await.expect_err("timeout must not complete the flow");
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(900),
            "gave up before the injected 1 s timeout: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "timeout did not fire promptly: {elapsed:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("timed out"),
            "actionable timeout message expected, got: {msg}"
        );
        assert!(
            err.downcast_ref::<SignInCancelled>().is_none(),
            "a timeout is not a user cancel"
        );

        wait_for_port_free(port).await;
    }

    /// Server/port-forward mode keeps its observable contract: it binds
    /// exactly the port the external base URL advertises, the login URL
    /// embeds that port, the callback completes, and the server handle is
    /// handed back to the caller (kept alive so port-forward stays up).
    #[tokio::test]
    async fn server_mode_binds_advertised_port_completes_and_keeps_handle() {
        let mock = spawn_mock_id_stack().await;
        // Find a free port for the advertised callback base.
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let base = format!("http://127.0.0.1:{port}");

        let (url_tx, url_rx) = oneshot::channel::<String>();
        let flow = run_oauth_flow(
            &mock.base,
            false,
            Some(base.clone()),
            None,
            Some(url_tx), // server mode
            None,
            None,
            10,
            no_browser,
        );
        tokio::pin!(flow);

        let login_url = tokio::select! {
            r = &mut flow => panic!("flow ended before reporting the login URL: {r:?}"),
            url = url_rx => url.expect("server mode reports the login URL"),
        };
        let expected_redirect =
            urlencoding::encode(&format!("{base}/session-created")).into_owned();
        assert!(
            login_url.contains(&expected_redirect),
            "login URL must embed the advertised callback port: {login_url}"
        );

        // Drive the "browser" straight to /cb on the advertised port.
        let client = no_follow_client();
        let resp = client
            .get(format!("{base}/cb?code=mock-auth-code"))
            .send()
            .await
            .expect("/cb reachable on the advertised port");
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let mut result = flow.await.expect("flow completes");
        assert_eq!(result.access_token, "mock-access-token");
        assert!(
            result.server_handle.is_some(),
            "server mode keeps the listener alive for port-forward"
        );

        // The handle is abortable and frees the port (sh-ui does this
        // before the next sign-in and on sign-out).
        let handle = result.server_handle.take().expect("server handle");
        handle.abort();
        wait_for_port_free(port).await;
    }
}

/// Generate a random 32-byte code verifier, base64url-encoded (no padding).
fn generate_code_verifier() -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// Compute the S256 code challenge: base64url(sha256(code_verifier)).
fn generate_code_challenge(verifier: &str) -> String {
    use base64::Engine;
    let digest = sha2::Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Shared state passed to the OAuth callback handler.
struct CallbackState {
    client: reqwest::Client,
    token_endpoint: String,
    client_id: String,
    redirect_uri: String,
    code_verifier: String,
    tx: mpsc::Sender<OAuthResult>,
    matrix_base_url: String,
}

/// Handle the OAuth callback: exchange the authorization code for tokens server-side.
async fn handle_oauth_callback(
    Query(query): Query<CbQuery>,
    state: Arc<CallbackState>,
) -> Html<String> {
    // Check for errors from Keycloak
    if let Some(ref err) = query.error {
        let desc = query
            .error_description
            .as_deref()
            .unwrap_or("Unknown error");
        tracing::error!("OAuth error: {} — {}", err, desc);
        return Html(error_page(&format!(
            "Authentication error: {} — {}",
            err, desc
        )));
    }

    let code = match query.code {
        Some(ref c) => c.clone(),
        None => {
            tracing::error!("OAuth callback: no authorization code received");
            return Html(error_page(
                "No authorization code received. Please try again.",
            ));
        }
    };

    tracing::info!("OAuth callback received authorization code, exchanging for tokens...");

    // Exchange authorization code + PKCE verifier for tokens server-side
    let resp = state
        .client
        .post(&state.token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &state.client_id),
            ("redirect_uri", &state.redirect_uri),
            ("code_verifier", &state.code_verifier),
        ])
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Token exchange request failed: {}", e);
            return Html(error_page(&format!("Token exchange failed: {}", e)));
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::error!("Token exchange failed: {} — {}", status, body);
        return Html(error_page(&format!(
            "Token exchange failed: {} — {}",
            status, body
        )));
    }

    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Failed to parse token response: {}", e);
            return Html(error_page(&format!(
                "Failed to parse token response: {}",
                e
            )));
        }
    };

    let access_token = match body.get("access_token").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            tracing::error!("No access_token in token response");
            return Html(error_page("No access_token in token response"));
        }
    };

    let refresh_token = body
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(String::from);

    tracing::info!("Token exchange successful");

    // Send the tokens to the waiting OAuth flow (mpsc(1) — first send wins,
    // duplicates from browser redirect replays are silently ignored).
    match state.tx.try_send(OAuthResult {
        access_token,
        refresh_token,
        token_endpoint: state.token_endpoint.clone(),
        client_id: state.client_id.clone(),
        server_handle: None, // set by the caller after recv
    }) {
        Ok(()) => tracing::info!("OAuth result sent to receiver"),
        Err(mpsc::error::TrySendError::Full(_)) => {
            tracing::info!("OAuth result already buffered (duplicate callback, ignoring)");
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            tracing::error!("OAuth receiver dropped — sign-in task was cancelled");
        }
    }

    Html(success_page(&state.matrix_base_url))
}

// ---------------------------------------------------------------------------
// Keycloak discovery
// ---------------------------------------------------------------------------

/// Keycloak discovery info from Matrix publicConfig.
struct KeycloakConfig {
    url: String,
    realm: String,
    client_id: String,
}

struct OidcEndpoints {
    authorization_endpoint: String,
    token_endpoint: String,
}

/// POST a GraphQL query with retry + linear backoff.
///
/// Transient connect/DNS/TLS failures ("error sending request") previously
/// aborted the whole sign-in flow after a single attempt, dumping the user
/// back on the sign-in screen. Retry a few times before giving up.
async fn gql_post_with_retry(
    client: &reqwest::Client,
    url: &str,
    query: &serde_json::Value,
    attempts: u32,
) -> anyhow::Result<reqwest::Response> {
    let mut last: Option<reqwest::Error> = None;
    for attempt in 1..=attempts {
        match client
            .post(url)
            .header("Content-Type", "application/json")
            .json(query)
            .send()
            .await
        {
            Ok(r) => return Ok(r),
            Err(e) => {
                tracing::warn!(
                    "GraphQL POST to {} failed (attempt {}/{}): {}",
                    url,
                    attempt,
                    attempts,
                    e
                );
                last = Some(e);
                if attempt < attempts {
                    tokio::time::sleep(std::time::Duration::from_millis(700 * attempt as u64))
                        .await;
                }
            }
        }
    }
    Err(anyhow::anyhow!(
        "request failed after {} attempts: {}",
        attempts,
        last.map(|e| e.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    ))
}

async fn discover_keycloak(
    client: &reqwest::Client,
    matrix_base: &str,
) -> anyhow::Result<KeycloakConfig> {
    let url = format!("{}/api/v1alpha/graphql", matrix_base);
    let query = serde_json::json!({
        "query": "query { publicConfig { keycloak { url realm clientId } } }"
    });

    let resp = gql_post_with_retry(client, &url, &query, 4).await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("publicConfig query failed: {} — {}", status, body);
    }

    let body: serde_json::Value = resp.json().await?;
    let kc = body
        .pointer("/data/publicConfig/keycloak")
        .ok_or_else(|| anyhow::anyhow!("publicConfig response missing keycloak field"))?;

    Ok(KeycloakConfig {
        url: kc
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("keycloak.url missing"))?
            .trim_end_matches('/')
            .to_string(),
        realm: kc
            .get("realm")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("keycloak.realm missing"))?
            .to_string(),
        client_id: kc
            .get("clientId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("keycloak.clientId missing"))?
            .to_string(),
    })
}

async fn fetch_oidc_config(
    client: &reqwest::Client,
    keycloak_url: &str,
    realm: &str,
) -> anyhow::Result<OidcEndpoints> {
    let url = format!(
        "{}/realms/{}/.well-known/openid-configuration",
        keycloak_url, realm
    );
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("OIDC well-known fetch failed: {} — {}", status, body);
    }
    let body: serde_json::Value = resp.json().await?;
    Ok(OidcEndpoints {
        authorization_endpoint: body
            .get("authorization_endpoint")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing authorization_endpoint"))?
            .to_string(),
        token_endpoint: body
            .get("token_endpoint")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing token_endpoint"))?
            .to_string(),
    })
}

// ---------------------------------------------------------------------------
// HTML helpers
// ---------------------------------------------------------------------------

fn success_page(matrix_base_url: &str) -> String {
    // Redirect to the app root, not /studio/ — StrikeHub is the shell.
    let home_url = format!("{}/", matrix_base_url.trim_end_matches('/'));
    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8">
<title>StrikeHub - Signed In</title>
<meta http-equiv="refresh" content="0;url={home_url}">
<style>
  body {{ font-family: system-ui, sans-serif; display: flex; align-items: center;
         justify-content: center; min-height: 100vh; margin: 0;
         background: #1a1a1a; color: #e0e0e0; }}
  .container {{ text-align: center; max-width: 400px; padding: 2rem; }}
  h2 {{ margin-bottom: 1rem; font-weight: 600; color: #4ade80; }}
  .status {{ color: #888; font-size: 14px; }}
  a {{ color: #4ade80; }}
</style></head><body>
<script>window.location.replace('{home_url}');</script>
<div class="container"><h2>Signed in!</h2><p class="status">Redirecting…</p></div>
</body></html>"#
    )
}

/// Escape a string for safe embedding inside a JavaScript single-quoted
/// string literal.  Handles quote breakout, backslash injection, newlines,
/// and `</script>` tag injection.
pub fn js_string_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\x00"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            '<' => out.push_str("\\x3c"),
            '>' => out.push_str("\\x3e"),
            _ => out.push(c),
        }
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn error_page(message: &str) -> String {
    let safe_message = html_escape(message);
    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8">
<title>StrikeHub - Sign-in Error</title>
<style>
  body {{ font-family: system-ui, sans-serif; display: flex; align-items: center;
         justify-content: center; min-height: 100vh; margin: 0;
         background: #1a1a1a; color: #e0e0e0; }}
  .container {{ text-align: center; max-width: 500px; padding: 2rem; }}
  h2 {{ margin-bottom: 1rem; font-weight: 600; color: #f87171; }}
  .status {{ color: #888; font-size: 14px; }}
</style></head><body>
<div class="container"><h2>Sign-in failed</h2><p class="status">{}</p></div>
</body></html>"#,
        safe_message
    )
}

// ---------------------------------------------------------------------------
// Serde types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CbQuery {
    code: Option<String>,
    #[allow(dead_code)]
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}
