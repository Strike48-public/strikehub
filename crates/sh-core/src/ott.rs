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

/// Check whether saved SDK credentials exist on disk for a given connector
/// and are complete enough for `private_key_jwt` authentication.
///
/// The SDK saves credentials to `~/.strike48/credentials/{type}_{instance}.json`
/// after successful OTT registration. A valid file must contain a `kid` field
/// (key-ID) so Keycloak can look up the public key. Files without `kid` were
/// created by an older SDK and will fail with "Unable to load public key".
pub fn has_saved_credentials(strikehub_id: &str, instance_id: &str) -> bool {
    let home = match dirs::home_dir() {
        Some(h) => h.to_string_lossy().to_string(),
        None => return false,
    };

    let sdk_type = sdk_connector_type(strikehub_id);
    let filename = format!("{}_{}.json", sdk_type, instance_id);

    for dir in &[".strike48", ".matrix"] {
        let path = std::path::Path::new(&home)
            .join(dir)
            .join("credentials")
            .join(&filename);

        if path.exists() {
            if credentials_have_kid(&path) {
                tracing::debug!("Found valid credentials at {}", path.display());
                return true;
            }
            tracing::warn!(
                "Credentials at {} missing 'kid' (old SDK), will create fresh OTT",
                path.display()
            );
        }
    }

    false
}

/// Delete all saved credentials and keys whose filename contains the given
/// URL slug.  This covers both `.strike48` and `.matrix` directories and
/// removes credentials for every connector scoped to that Studio URL.
///
/// Returns a list of deleted file paths.
pub fn clear_credentials_for_url(studio_url: &str) -> Vec<String> {
    let slug = crate::url_slug(studio_url);
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return vec![],
    };

    let mut deleted = Vec::new();
    for base in &[".strike48", ".matrix"] {
        for subdir in &["credentials", "keys"] {
            let dir = home.join(base).join(subdir);
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name.to_string_lossy().contains(&slug) {
                    let path = entry.path();
                    if std::fs::remove_file(&path).is_ok() {
                        tracing::info!("Deleted credential: {}", path.display());
                        deleted.push(path.display().to_string());
                    }
                }
            }
        }
    }
    deleted
}

/// Check whether a credential JSON file contains a `kid` (key ID) field.
///
/// The `kid` is required for Keycloak to locate the public key during
/// `private_key_jwt` token exchange. Files without it are from an older SDK.
fn credentials_have_kid(path: &std::path::Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return false;
    };
    json.get("kid")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
}
