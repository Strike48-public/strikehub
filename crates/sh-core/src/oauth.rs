use axum::http::header;
use axum::{Router, extract::Query, response::Html, routing::get};
use serde::Deserialize;
use sha2::Digest;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Result of a successful OAuth flow.
pub struct OAuthResult {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Keycloak token endpoint (for token refresh).
    pub token_endpoint: String,
    /// Keycloak client_id (for token refresh).
    pub client_id: String,
}

/// Start the system-browser OAuth flow via Matrix + Keycloak PKCE.
///
/// Two-hop flow that ensures both a Matrix session AND usable tokens:
///
/// 1. Discover Keycloak OIDC config via Matrix publicConfig
/// 2. Generate PKCE code_verifier + code_challenge
/// 3. Open browser → `{matrix}/auth/login?redirect=http://127.0.0.1:4000/session-created`
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
/// callback server (e.g. `http://localhost:8080` when port-forwarding).
/// If `None`, defaults to `http://127.0.0.1:{port}` using the bound port.
pub async fn start_oauth_flow(matrix_url: &str, tls_insecure: bool) -> anyhow::Result<OAuthResult> {
    start_oauth_flow_with(matrix_url, tls_insecure, None, None, None).await
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
pub async fn start_oauth_flow_with(
    matrix_url: &str,
    tls_insecure: bool,
    callback_base_url: Option<String>,
    browser_matrix_url: Option<String>,
    login_url_tx: Option<tokio::sync::oneshot::Sender<String>>,
) -> anyhow::Result<OAuthResult> {
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

    // Bind callback server — use 0.0.0.0 so it's reachable from outside
    // containers when port-forwarded. Try port 4000 first (whitelisted in
    // Keycloak), fall back to any available port.
    let bind_addr = "0.0.0.0:4000";
    let listener = match tokio::net::TcpListener::bind(bind_addr).await {
        Ok(l) => l,
        Err(_) => tokio::net::TcpListener::bind("0.0.0.0:0")
            .await
            .map_err(|e| anyhow::anyhow!("Failed to bind OAuth callback server: {}", e))?,
    };
    let addr: SocketAddr = listener.local_addr()?;
    tracing::info!("OAuth callback server listening on {}", addr);

    // External base URL for the callback — either provided (server mode)
    // or default to localhost with the bound port (desktop mode).
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

    let is_server_mode = login_url_tx.is_some();
    tracing::info!("Opening browser for Matrix login (two-hop: Matrix session + PKCE)");
    if let Some(tx) = login_url_tx {
        // Server mode: send URL back to the caller for client-side opening
        let _ = tx.send(login_url.clone());
        tracing::info!("Login URL sent to caller: {}", login_url);
    } else {
        // Desktop mode: open system browser directly
        if let Err(e) = open::that(&login_url) {
            tracing::error!("Failed to open system browser: {}", e);
            server_handle.abort();
            anyhow::bail!("Failed to open system browser: {}", e);
        }
    }

    // Wait for token with 5-minute timeout
    tracing::info!("Waiting for OAuth callback (rx)...");
    let result = tokio::time::timeout(std::time::Duration::from_secs(300), rx.recv()).await;

    // In server mode, keep the callback server alive so the port stays open
    // (kubectl port-forward drops all ports if any forwarded port closes).
    // In desktop mode, shut it down immediately — it's not needed.
    if !is_server_mode {
        server_handle.abort();
    }

    match result {
        Ok(Some(oauth_result)) => {
            tracing::info!("OAuth flow completed successfully");
            Ok(oauth_result)
        }
        Ok(None) => {
            tracing::error!("OAuth callback channel closed unexpectedly");
            anyhow::bail!("OAuth callback channel closed unexpectedly")
        }
        Err(_) => anyhow::bail!("OAuth flow timed out after 5 minutes"),
    }
}

// ---------------------------------------------------------------------------
// PKCE helpers
// ---------------------------------------------------------------------------

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
    }) {
        Ok(()) => tracing::info!("OAuth result sent to receiver"),
        Err(mpsc::error::TrySendError::Full(_)) => {
            tracing::info!("OAuth result already buffered (duplicate callback, ignoring)");
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            tracing::error!("OAuth receiver dropped — sign-in task was cancelled");
        }
    }

    Html(success_page())
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

async fn discover_keycloak(
    client: &reqwest::Client,
    matrix_base: &str,
) -> anyhow::Result<KeycloakConfig> {
    let url = format!("{}/api/v1alpha/graphql", matrix_base);
    let query = serde_json::json!({
        "query": "query { publicConfig { keycloak { url realm clientId } } }"
    });

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&query)
        .send()
        .await?;

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

fn success_page() -> String {
    r#"<!DOCTYPE html><html><head><meta charset="utf-8">
<title>StrikeHub - Signed In</title>
<style>
  body { font-family: system-ui, sans-serif; display: flex; align-items: center;
         justify-content: center; min-height: 100vh; margin: 0;
         background: #1a1a1a; color: #e0e0e0; }
  .container { text-align: center; max-width: 400px; padding: 2rem; }
  h2 { margin-bottom: 1rem; font-weight: 600; color: #4ade80; }
  .status { color: #888; font-size: 14px; }
</style></head><body>
<div class="container"><h2>Signed in!</h2><p class="status">You can close this tab.</p></div>
<script>setTimeout(function(){window.close()},1000)</script>
</body></html>"#
        .to_string()
}

fn error_page(message: &str) -> String {
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
        message
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
