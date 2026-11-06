//! PLG download attribution (project-management#315, item #4114).
//!
//! On first launch after sign-in, StrikeHub reports to Matrix which product
//! this installation delivers, plus the app version and host platform —
//! exactly once per install + Studio URL (and again when product or version
//! changes, e.g. an app upgrade). Matrix stores the record on the caller's
//! own tenant and surfaces it in the PLG Activity Report's Product column.
//!
//! Privacy: the payload is exactly `{product, version, platform}`. No email,
//! name, target, or scan data is sent or even representable — the user's
//! identity is resolved server-side from the Bearer token, so the PLG
//! privacy model is preserved by construction.
//!
//! Failure handling: the report is best-effort. A failed attempt is logged
//! and the record is NOT persisted, so the next launch retries (guarded
//! retry). It never blocks sign-in or connector startup.

use crate::auth::AuthManager;
use crate::config::AttributionRecord;
use crate::registry::DEFAULT_CONNECTOR_ID;

/// The product identity this build delivers.
///
/// StrikeHub is the launcher; the product its download delivers is its
/// default/primary connector (Pick today). A future build with a different
/// primary product carries its own id — recorded, never guessed.
pub fn product() -> &'static str {
    DEFAULT_CONNECTOR_ID
}

/// App version of the downloaded artifact.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Host platform tag, e.g. `"macos-aarch64"`.
pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The full current attribution record for this installation.
pub fn current_record() -> AttributionRecord {
    AttributionRecord {
        product: product().to_string(),
        version: version().to_string(),
        platform: platform(),
    }
}

/// Pure staleness predicate.
///
/// Report when nothing has been recorded for this install/URL yet, or when
/// product/version/platform changed since the last report (app upgrade or
/// different product build). Identical records are NOT re-sent.
pub fn needs_report(stored: Option<&AttributionRecord>, current: &AttributionRecord) -> bool {
    match stored {
        None => true,
        Some(stored) => {
            stored.product != current.product
                || stored.version != current.version
                || stored.platform != current.platform
        }
    }
}

/// The `reportClientAttribution` GraphQL mutation body.
pub fn build_mutation(record: &AttributionRecord) -> serde_json::Value {
    serde_json::json!({
        "query": "mutation ReportClientAttribution($input: ReportClientAttributionInput!) { reportClientAttribution(input: $input) { product version platform } }",
        "variables": {
            "input": {
                "product": record.product,
                "version": record.version,
                "platform": record.platform
            }
        }
    })
}

/// POST the mutation to Matrix.
///
/// Tries the WebSocket path first (it accepts PKCE JWTs), then falls back to
/// HTTP POST with the Bearer token — the same dual path as
/// [`crate::auth::fetch_tenant_id`]. Returns `Ok(())` only when Matrix
/// accepted the record.
pub async fn report(
    auth: &AuthManager,
    matrix_ws: Option<&crate::matrix_ws::MatrixWsClient>,
    record: &AttributionRecord,
) -> Result<(), String> {
    let token = auth.token();
    if token.is_empty() {
        return Err("no auth token available".into());
    }

    let body = build_mutation(record);

    if let Some(ws) = matrix_ws {
        let ws_body = serde_json::to_vec(&body).unwrap_or_default();
        match ws.query(&ws_body).await {
            Ok(resp_str) => match parse_response(&resp_str, record) {
                Ok(()) => {
                    tracing::info!("download attribution reported via WS");
                    return Ok(());
                }
                Err(e) => {
                    tracing::debug!("attribution WS report failed ({e}), trying HTTP");
                }
            },
            Err(e) => {
                tracing::debug!("attribution WS query failed ({e}), trying HTTP");
            }
        }
    }

    let base = auth.matrix_url().trim_end_matches('/');
    let url = format!("{base}/api/v1alpha/graphql");

    // Reuse the app's shared HTTP client — its TLS policy (set once from the
    // user's own insecure-TLS setting at AuthManager construction) already
    // covers this target, so no new TLS decision is made here.
    let resp = auth
        .client()
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("attribution request failed: {e}"))?;

    let body_text = resp
        .text()
        .await
        .map_err(|e| format!("failed to read attribution response: {e}"))?;

    parse_response(&body_text, record)
}

/// Accept only when Matrix echoes back the same product — a GraphQL error
/// (`errors` non-empty) or a missing/changed echo is a failure, so the
/// record is not persisted and the next launch retries.
fn parse_response(raw: &str, record: &AttributionRecord) -> Result<(), String> {
    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid JSON response: {e}"))?;

    if let Some(errors) = v.pointer("/errors").and_then(|e| e.as_array())
        && !errors.is_empty()
    {
        let msg = errors
            .first()
            .and_then(|e| e.pointer("/message").and_then(|m| m.as_str()))
            .unwrap_or("unknown GraphQL error");
        return Err(format!("GraphQL error: {msg}"));
    }

    let echoed = v
        .pointer("/data/reportClientAttribution/product")
        .and_then(|p| p.as_str())
        .map(String::from);

    match echoed.as_deref() {
        Some(p) if p == record.product => Ok(()),
        Some(p) => Err(format!(
            "Matrix recorded unexpected product {p:?} (sent {:?})",
            record.product
        )),
        None => Err("response missing reportClientAttribution.product".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(product: &str, version: &str, platform: &str) -> AttributionRecord {
        AttributionRecord {
            product: product.into(),
            version: version.into(),
            platform: platform.into(),
        }
    }

    #[test]
    fn needs_report_when_never_reported() {
        let current = rec("pick", "0.1.15", "macos-aarch64");
        assert!(needs_report(None, &current));
    }

    #[test]
    fn needs_report_when_version_changed() {
        let current = rec("pick", "0.1.16", "macos-aarch64");
        let stored = rec("pick", "0.1.15", "macos-aarch64");
        assert!(needs_report(Some(&stored), &current));
    }

    #[test]
    fn needs_report_when_product_changed() {
        let current = rec("kubestudio", "0.1.15", "macos-aarch64");
        let stored = rec("pick", "0.1.15", "macos-aarch64");
        assert!(needs_report(Some(&stored), &current));
    }

    #[test]
    fn no_report_for_identical_record() {
        let current = rec("pick", "0.1.15", "macos-aarch64");
        assert!(!needs_report(Some(&current), &current));
    }

    #[test]
    fn current_record_shape() {
        let r = current_record();
        assert_eq!(r.product, DEFAULT_CONNECTOR_ID);
        assert!(!r.version.is_empty());
        assert!(
            r.platform.starts_with(std::env::consts::OS),
            "platform {r:?} should start with the host OS"
        );
        assert!(r.platform.contains('-'));
    }

    #[test]
    fn build_mutation_carrys_only_identity_version_platform() {
        let r = rec("pick", "0.1.15", "macos-aarch64");
        let body = build_mutation(&r);

        let input = body.pointer("/variables/input").cloned().unwrap();
        // Exactly three keys — no user data can ride along.
        assert_eq!(input.as_object().map(|o| o.len()), Some(3));
        assert_eq!(input["product"], "pick");
        assert_eq!(input["version"], "0.1.15");
        assert_eq!(input["platform"], "macos-aarch64");
        assert!(
            body["query"]
                .as_str()
                .unwrap()
                .contains("reportClientAttribution")
        );
    }

    #[test]
    fn parse_response_accepts_matching_echo() {
        let r = rec("pick", "0.1.15", "macos-aarch64");
        let raw = r#"{"data":{"reportClientAttribution":{"product":"pick","version":"0.1.15","platform":"macos-aarch64"}}}"#;
        assert!(parse_response(raw, &r).is_ok());
    }

    #[test]
    fn parse_response_rejects_graphql_errors() {
        let r = rec("pick", "0.1.15", "macos-aarch64");
        let raw = r#"{"errors":[{"message":"Not authenticated"}],"data":null}"#;
        assert!(parse_response(raw, &r).is_err());
    }

    #[test]
    fn parse_response_rejects_missing_product() {
        let r = rec("pick", "0.1.15", "macos-aarch64");
        let raw = r#"{"data":{"reportClientAttribution":null}}"#;
        assert!(parse_response(raw, &r).is_err());
    }

    #[test]
    fn parse_response_rejects_mismatched_product() {
        let r = rec("pick", "0.1.15", "macos-aarch64");
        let raw = r#"{"data":{"reportClientAttribution":{"product":"other"}}}"#;
        assert!(parse_response(raw, &r).is_err());
    }
}
