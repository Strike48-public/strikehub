use base64::Engine;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Acquire a read lock, recovering from poisoned state.
fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|e| e.into_inner())
}

/// Acquire a write lock, recovering from poisoned state.
fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|e| e.into_inner())
}

/// Manages OIDC authentication with Keycloak (discovered via Matrix publicConfig).
///
/// Token is set externally after the system-browser OAuth flow completes
/// (see `oauth::start_oauth_flow`). The refresh loop uses the Keycloak
/// token endpoint directly with the refresh_token grant.
///
/// After OAuth, call [`bootstrap_sandbox_token`] with the Matrix app address
/// to obtain a short-lived sandbox token that Matrix's `/api/v1alpha`
/// endpoint accepts. The sandbox token is refreshed automatically.
#[derive(Clone)]
pub struct AuthManager {
    matrix_url: String,
    tls_insecure: bool,
    token: Arc<RwLock<String>>,
    refresh_token: Arc<RwLock<Option<String>>>,
    token_endpoint: Arc<RwLock<Option<String>>>,
    keycloak_client_id: Arc<RwLock<Option<String>>>,
    /// Sandbox token issued by Matrix for the connector app.
    /// This is the token that `/api/v1alpha` actually accepts.
    sandbox_token: Arc<RwLock<String>>,
    client: reqwest::Client,
}

impl AuthManager {
    pub const DEFAULT_API_URL: &str = "https://studio.strike48.com";

    /// Create from env vars, falling back to the default Strike48 API URL.
    ///
    /// | Var | Purpose | Default |
    /// |-----|---------|---------|
    /// | `STRIKE48_API_URL` | Strike48 API server URL | `wss://studio.strike48.com` |
    /// | `MATRIX_TLS_INSECURE` | Skip TLS verify | `false` |
    pub fn from_env() -> Option<Self> {
        let matrix_url = std::env::var("STRIKE48_API_URL")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| Self::DEFAULT_API_URL.to_string());
        let tls_insecure = std::env::var("MATRIX_TLS_INSECURE")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(tls_insecure)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .ok()?;

        Some(Self {
            matrix_url,
            tls_insecure,
            token: Arc::new(RwLock::new(String::new())),
            refresh_token: Arc::new(RwLock::new(None)),
            token_endpoint: Arc::new(RwLock::new(None)),
            keycloak_client_id: Arc::new(RwLock::new(None)),
            sandbox_token: Arc::new(RwLock::new(String::new())),
            client,
        })
    }

    /// Set the token and Keycloak refresh parameters after OAuth completes.
    pub fn set_token(
        &self,
        token: String,
        refresh_token: Option<String>,
        token_endpoint: String,
        client_id: String,
    ) {
        *write_lock(&self.token) = token;
        *write_lock(&self.refresh_token) = refresh_token;
        *write_lock(&self.token_endpoint) = Some(token_endpoint);
        *write_lock(&self.keycloak_client_id) = Some(client_id);
    }

    /// Whether a token is currently available.
    pub fn is_authenticated(&self) -> bool {
        !read_lock(&self.token).is_empty()
    }

    /// Clear all auth state (sign out).
    pub fn clear_auth(&self) {
        *write_lock(&self.token) = String::new();
        *write_lock(&self.refresh_token) = None;
        *write_lock(&self.token_endpoint) = None;
        *write_lock(&self.keycloak_client_id) = None;
        *write_lock(&self.sandbox_token) = String::new();
    }

    /// Get the current Keycloak JWT (empty string if not yet authenticated).
    pub fn token(&self) -> String {
        read_lock(&self.token).clone()
    }

    /// Extract the user's display name from the current JWT, decoded transiently.
    /// Returns `None` if not authenticated or if the claim is missing.
    /// Tries `name` first, then falls back to `preferred_username`.
    pub fn user_display_name(&self) -> Option<String> {
        let token = self.token();
        if token.is_empty() {
            return None;
        }
        let claims = parse_jwt_claims(&token)?;
        claims
            .get("name")
            .and_then(|v| v.as_str())
            .or_else(|| claims.get("preferred_username").and_then(|v| v.as_str()))
            .map(String::from)
    }

    /// Extract the user's email from the current JWT, decoded transiently.
    /// Returns `None` if not authenticated or if the claim is missing.
    pub fn user_email(&self) -> Option<String> {
        let token = self.token();
        if token.is_empty() {
            return None;
        }
        let claims = parse_jwt_claims(&token)?;
        claims
            .get("email")
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    /// Get the sandbox token for Matrix API calls.
    /// Returns empty string if not yet bootstrapped.
    pub fn sandbox_token(&self) -> String {
        read_lock(&self.sandbox_token).clone()
    }

    /// The best token available for Matrix API calls: sandbox token if
    /// available, otherwise the Keycloak JWT.
    pub fn api_token(&self) -> String {
        let st = read_lock(&self.sandbox_token).clone();
        if !st.is_empty() {
            return st;
        }
        read_lock(&self.token).clone()
    }

    /// Bootstrap a sandbox token by fetching Matrix's app-content page.
    ///
    /// Matrix's `/app-content/{address}/` endpoint accepts a Keycloak JWT via
    /// Bearer auth and returns HTML with an injected `__MATRIX_SESSION_TOKEN__`
    /// (a short-lived HMAC sandbox token). We extract that token for API use.
    pub async fn bootstrap_sandbox_token(&self, app_address: &str) -> anyhow::Result<()> {
        let jwt = self.token();
        if jwt.is_empty() {
            anyhow::bail!("No Keycloak JWT available for sandbox token bootstrap");
        }

        let url = format!(
            "{}/app-content/{}/",
            self.matrix_url.trim_end_matches('/'),
            app_address
        );

        tracing::info!("Bootstrapping sandbox token from {}", url);

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", jwt))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "app-content request failed: {} — {}",
                status,
                &body[..body.len().min(200)]
            );
        }

        let html = resp.text().await?;

        // Extract window.__MATRIX_SESSION_TOKEN__ = '...'
        let token = extract_injected_token(&html).ok_or_else(|| {
            anyhow::anyhow!("No __MATRIX_SESSION_TOKEN__ found in app-content HTML")
        })?;

        tracing::info!("Bootstrapped sandbox token (length: {})", token.len());

        *write_lock(&self.sandbox_token) = token;
        Ok(())
    }

    /// Spawn a background loop that refreshes the sandbox token at ~70% of its TTL.
    pub fn spawn_sandbox_refresh_loop(&self) {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                let token = this.sandbox_token();
                if token.is_empty() {
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    continue;
                }

                // Parse TTL from the sandbox token. Sandbox tokens are
                // base64url(JSON).signature — the JSON payload has an "exp" field.
                let sleep_secs = sandbox_ttl_sleep_secs(&token);
                tracing::info!("Sandbox token refresh: sleeping {}s", sleep_secs);
                tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)).await;

                let current = this.sandbox_token();
                if current.is_empty() {
                    continue;
                }

                // Refresh via /api/app/token/refresh
                let mut success = false;
                for attempt in 1..=3 {
                    let refresh_url = format!(
                        "{}/api/app/token/refresh",
                        this.matrix_url.trim_end_matches('/')
                    );
                    match this
                        .client
                        .post(&refresh_url)
                        .header("Authorization", format!("Bearer {}", this.sandbox_token()))
                        .header("Content-Type", "application/json")
                        .send()
                        .await
                    {
                        Ok(resp) if resp.status().is_success() => {
                            if let Ok(body) = resp.json::<serde_json::Value>().await
                                && let Some(new_token) = body.get("token").and_then(|t| t.as_str())
                            {
                                *write_lock(&this.sandbox_token) = new_token.to_string();
                                tracing::info!("Sandbox token refreshed successfully");
                                success = true;
                                break;
                            }
                        }
                        Ok(resp) => {
                            tracing::warn!(
                                "Sandbox refresh attempt {}/3 returned {}",
                                attempt,
                                resp.status()
                            );
                        }
                        Err(e) => {
                            tracing::warn!("Sandbox refresh attempt {}/3 failed: {}", attempt, e);
                        }
                    }
                    if attempt < 3 {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }

                if !success {
                    tracing::error!("Sandbox token refresh failed after 3 attempts");
                    // Don't clear — the token might still work briefly
                }
            }
        });
    }

    /// The Matrix API base URL.
    pub fn matrix_url(&self) -> &str {
        &self.matrix_url
    }

    /// Whether TLS verification is disabled.
    pub fn tls_insecure(&self) -> bool {
        self.tls_insecure
    }

    /// Spawn a background task that refreshes the token based on JWT TTL.
    ///
    /// Parses the `exp` claim from the current access token and sleeps for
    /// 70% of the remaining TTL (minimum 30s). Retries up to 3 times on failure.
    pub fn spawn_refresh_loop(&self) {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                if !this.is_authenticated() {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    continue;
                }

                let sleep_secs = {
                    let token = this.token();
                    ttl_sleep_secs(&token)
                };

                tracing::info!("Auth refresh: sleeping {}s (70% of token TTL)", sleep_secs);
                tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)).await;

                if !this.is_authenticated() {
                    continue;
                }

                // Retry up to 3 times with 5s backoff
                let mut success = false;
                for attempt in 1..=3 {
                    match this.do_refresh().await {
                        Ok(()) => {
                            tracing::info!("Auth refresh: token refreshed successfully");
                            success = true;
                            break;
                        }
                        Err(e) => {
                            tracing::error!("Auth refresh: attempt {}/3 failed: {}", attempt, e);
                            if attempt < 3 {
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            }
                        }
                    }
                }

                if !success {
                    tracing::error!("Auth refresh: all 3 attempts failed, clearing auth state");
                    this.clear_auth();
                }
            }
        });
    }

    /// Refresh the token using Keycloak's token endpoint with refresh_token grant.
    async fn do_refresh(&self) -> anyhow::Result<()> {
        let rt = read_lock(&self.refresh_token)
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No refresh_token available"))?;

        let endpoint = read_lock(&self.token_endpoint)
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No token_endpoint configured"))?;

        let client_id = read_lock(&self.keycloak_client_id)
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No keycloak_client_id configured"))?;

        let resp = self
            .client
            .post(&endpoint)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &rt),
                ("client_id", &client_id),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Token refresh failed: {} — {}", status, body);
        }

        let body: serde_json::Value = resp.json().await?;

        let new_token = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("No access_token in refresh response"))?
            .to_string();

        let new_refresh = body
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(String::from);

        *write_lock(&self.token) = new_token;

        // Update refresh_token if Keycloak rotated it
        if let Some(new_rt) = new_refresh {
            *write_lock(&self.refresh_token) = Some(new_rt);
        }

        Ok(())
    }
}

/// Parse the JWT `exp` claim and compute sleep duration as 70% of remaining TTL.
/// Returns at least 30 seconds. Falls back to 4 minutes on parse failure.
fn ttl_sleep_secs(token: &str) -> u64 {
    const FALLBACK_SECS: u64 = 4 * 60;
    const MIN_SECS: u64 = 30;

    let exp = match parse_jwt_exp(token) {
        Some(e) => e,
        None => {
            tracing::warn!("Could not parse JWT exp, using {}s fallback", FALLBACK_SECS);
            return FALLBACK_SECS;
        }
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if exp <= now {
        return MIN_SECS;
    }

    let remaining = exp - now;
    let sleep = (remaining as f64 * 0.7) as u64;
    sleep.max(MIN_SECS)
}

/// Decode the JWT payload into a JSON object (no signature verification).
fn parse_jwt_claims(token: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .ok()?;
    serde_json::from_slice(&payload).ok()
}

/// Extract the `exp` field from a JWT's payload (no signature verification needed —
/// we just need the expiry for scheduling).
fn parse_jwt_exp(token: &str) -> Option<u64> {
    parse_jwt_claims(token)?.get("exp")?.as_u64()
}

/// Extract `window.__MATRIX_SESSION_TOKEN__ = '...'` from HTML.
fn extract_injected_token(html: &str) -> Option<String> {
    // Look for the pattern: window.__MATRIX_SESSION_TOKEN__ = '...'
    let marker = "window.__MATRIX_SESSION_TOKEN__";
    let idx = html.find(marker)?;
    let rest = &html[idx + marker.len()..];
    // Skip whitespace and '='
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?;
    let rest = rest.trim_start();
    // Extract the quoted value (single or double quotes)
    let (quote, rest) = if let Some(rest) = rest.strip_prefix('\'') {
        ('\'', rest)
    } else if let Some(rest) = rest.strip_prefix('"') {
        ('"', rest)
    } else {
        return None;
    };
    let end = rest.find(quote)?;
    let token = &rest[..end];
    if token.is_empty() {
        return None;
    }
    Some(token.to_string())
}

/// Compute sleep duration for sandbox token refresh.
/// Sandbox tokens are `base64url(JSON).signature` with an `exp` field in the JSON.
fn sandbox_ttl_sleep_secs(token: &str) -> u64 {
    const FALLBACK_SECS: u64 = 4 * 60;
    const MIN_SECS: u64 = 30;

    // Sandbox tokens: base64url(payload).signature
    let payload_b64 = token.split('.').next().unwrap_or("");
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(payload_b64))
        .ok();

    let exp = payload
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|json| json.get("exp")?.as_u64());

    let Some(exp) = exp else {
        // Try JWT format as fallback (3-part)
        return parse_jwt_exp(token)
            .map(|e| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if e <= now {
                    MIN_SECS
                } else {
                    ((e - now) as f64 * 0.7) as u64
                }
                .max(MIN_SECS)
            })
            .unwrap_or(FALLBACK_SECS);
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if exp <= now {
        return MIN_SECS;
    }

    let remaining = exp - now;
    ((remaining as f64 * 0.7) as u64).max(MIN_SECS)
}

/// Metadata about a connector app discovered from the Matrix API.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorAppInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub connector_type: Option<String>,
    /// The app address used to construct `/app-content/{address}/` URLs.
    #[serde(default)]
    pub address: Option<String>,
}

/// Query the Matrix GraphQL API for registered connector apps.
///
/// If a [`MatrixWsClient`] is provided, tries the WebSocket path first
/// (which accepts PKCE JWTs). Falls back to direct HTTP POST.
pub async fn fetch_connector_apps(
    auth: &AuthManager,
    matrix_ws: Option<&crate::matrix_ws::MatrixWsClient>,
) -> Vec<ConnectorAppInfo> {
    let token = auth.token();
    if token.is_empty() {
        return Vec::new();
    }

    let query_json = serde_json::json!({
        "query": "query ConnectorApps { connectorApps { name description icon connectorType address } }"
    });

    // Try WebSocket path first
    if let Some(ws) = matrix_ws {
        let body = serde_json::to_vec(&query_json).unwrap_or_default();
        match ws.query(&body).await {
            Ok(resp_str) => {
                if let Ok(body) = serde_json::from_str::<serde_json::Value>(&resp_str) {
                    let apps = body
                        .pointer("/data/connectorApps")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let result: Vec<ConnectorAppInfo> = apps
                        .into_iter()
                        .filter_map(|v| serde_json::from_value(v).ok())
                        .collect();
                    if !result.is_empty() {
                        tracing::info!("fetch_connector_apps: got {} apps via WS", result.len());
                        return result;
                    }
                }
                tracing::debug!("fetch_connector_apps: WS response had no apps, trying HTTP");
            }
            Err(e) => {
                tracing::debug!("fetch_connector_apps: WS query failed: {}, trying HTTP", e);
            }
        }
    }

    // HTTP fallback
    let base = auth.matrix_url().trim_end_matches('/');
    let url = format!("{}/api/v1alpha/graphql", base);

    let tls_insecure = std::env::var("MATRIX_TLS_INSECURE")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let client = match reqwest::Client::builder()
        .danger_accept_invalid_certs(tls_insecure)
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let resp = match client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .json(&query_json)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("Failed to query ConnectorApps: {}", e);
            return Vec::new();
        }
    };

    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!("Failed to parse ConnectorApps response: {}", e);
            return Vec::new();
        }
    };

    let apps = body
        .pointer("/data/connectorApps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    apps.into_iter()
        .filter_map(|v| serde_json::from_value::<ConnectorAppInfo>(v).ok())
        .collect()
}

/// Fetch the tenant ID from Matrix via the `userDetails` GraphQL query.
///
/// Tries WebSocket first, falls back to HTTP POST.
/// Returns `None` if the query fails or the field is missing.
pub async fn fetch_tenant_id(
    auth: &AuthManager,
    matrix_ws: Option<&crate::matrix_ws::MatrixWsClient>,
) -> Option<String> {
    let token = auth.token();
    if token.is_empty() {
        return None;
    }

    let query_json = serde_json::json!({
        "query": "query { userDetails { details } }"
    });

    // Try WebSocket path first
    if let Some(ws) = matrix_ws {
        let body = serde_json::to_vec(&query_json).unwrap_or_default();
        if let Ok(resp_str) = ws.query(&body).await {
            if let Some(id) = parse_tenant_id(&resp_str) {
                tracing::info!("fetch_tenant_id: resolved tenant via WS: {}", id);
                return Some(id);
            }
            tracing::debug!("fetch_tenant_id: WS response missing tenant, trying HTTP");
        }
    }

    // HTTP fallback
    let base = auth.matrix_url().trim_end_matches('/');
    let url = format!("{}/api/v1alpha/graphql", base);

    let tls_insecure = std::env::var("MATRIX_TLS_INSECURE")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(tls_insecure)
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .json(&query_json)
        .send()
        .await
        .ok()?;

    let body = resp.text().await.ok()?;
    let id = parse_tenant_id(&body);
    if let Some(ref id) = id {
        tracing::info!("fetch_tenant_id: resolved tenant via HTTP: {}", id);
    }
    id
}

fn parse_tenant_id(raw: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let details = v.pointer("/data/userDetails/details")?;
    // details may arrive as a stringified JSON blob or as an object
    let details = if let Some(s) = details.as_str() {
        serde_json::from_str::<serde_json::Value>(s).ok()?
    } else {
        details.clone()
    };
    details.pointer("/domain/id")?.as_str().map(String::from)
}
