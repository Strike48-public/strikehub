//! Create pre-approved OTT (One-Time Tokens) via the Matrix REST API.
//!
//! StrikeHub uses the existing Keycloak JWT to call `POST /api/connectors/pre-approve`.
//! The returned token is passed to connectors via the `STRIKE48_REGISTRATION_TOKEN`
//! env var so they can self-register without manual admin approval.

/// Create a pre-approved OTT by calling the Matrix pre-approve REST endpoint.
///
/// # Arguments
/// * `matrix_url` — Base Matrix Studio URL (e.g. `https://studio.strike48.test`)
/// * `jwt` — Keycloak JWT from the OIDC login flow
/// * `tls_insecure` — Whether to skip TLS certificate verification
/// * `connector_type` — SDK connector type (e.g. `app-kube-studio`, `pentest-connector`)
///
/// # Returns
/// JSON string `{"token":"ott_...","matrix_url":"https://..."}` for the SDK's
/// `OttProvider.parse_ott()`.  The `matrix_url` is needed because
/// `STRIKE48_API_URL` points to the local proxy which doesn't handle
/// `/api/connectors/register-with-ott`.
pub async fn create_pre_approved_token(
    matrix_url: &str,
    jwt: &str,
    tls_insecure: bool,
    connector_type: &str,
) -> anyhow::Result<String> {
    let base = matrix_url.trim_end_matches('/');
    let url = format!("{}/api/connectors/pre-approve", base);

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(tls_insecure)
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let payload = serde_json::json!({
        "connector_type": connector_type,
        "ttl_minutes": 5
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", jwt))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "pre-approve request failed: {} — {}",
            status,
            &body[..body.len().min(300)]
        );
    }

    let body: serde_json::Value = resp.json().await?;

    let token = body
        .get("token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("No token in pre-approve response: {}", body))?;

    tracing::info!(
        "Pre-approved OTT created: {}...",
        &token[..token.len().min(12)]
    );

    // Return JSON so the SDK's OttProvider.parse_ott() gets the matrix_url.
    // STRIKE48_API_URL points to the local proxy which doesn't handle
    // /api/connectors/register-with-ott, so we must embed the real URL.
    // The SDK's OttData struct expects "matrix_url" (not "api_url").
    let ott_json = serde_json::json!({
        "token": token,
        "matrix_url": base,
    });
    Ok(ott_json.to_string())
}

/// Map a StrikeHub connector ID to the SDK connector_type string used in
/// credential filenames and gateway registration.
pub fn sdk_connector_type(strikehub_id: &str) -> &str {
    match strikehub_id {
        "kubestudio" => "app-kube-studio",
        "pick" => "pentest-connector",
        _ => strikehub_id,
    }
}

/// Check whether saved SDK credentials exist on disk for a given connector.
///
/// The SDK saves credentials to `~/.strike48/credentials/{type}_{instance}.json`
/// after successful OTT registration. If these exist, the connector will use
/// them on startup (priority 3 in the auth chain) and no new OTT is needed.
pub fn has_saved_credentials(strikehub_id: &str, instance_id: &str) -> bool {
    let home = match dirs::home_dir() {
        Some(h) => h.to_string_lossy().to_string(),
        None => return false,
    };

    let sdk_type = sdk_connector_type(strikehub_id);
    let filename = format!("{}_{}.json", sdk_type, instance_id);
    let path = std::path::Path::new(&home)
        .join(".strike48")
        .join("credentials")
        .join(&filename);

    if path.exists() {
        tracing::debug!("Found saved credentials at {}", path.display());
        return true;
    }

    // Also check ~/.matrix/credentials (kubestudio uses this path).
    let alt_path = std::path::Path::new(&home)
        .join(".matrix")
        .join("credentials")
        .join(&filename);

    if alt_path.exists() {
        tracing::debug!("Found saved credentials at {}", alt_path.display());
        return true;
    }

    false
}
