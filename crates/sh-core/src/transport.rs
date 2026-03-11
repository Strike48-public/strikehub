//! Auto-detect the best transport scheme (gRPC vs WebSocket) for a Matrix
//! gateway by probing candidate endpoints **in parallel**.
//!
//! Given a base Studio URL like `https://studio.strike48.com`, the candidates
//! are probed concurrently:
//!
//! | Preference | Scheme  | Host derivation           |
//! |------------|---------|---------------------------|
//! | 1          | grpcs   | `connectors-{host}`       |
//! | 2          | wss     | `{host}` (unchanged)      |
//!
//! For `http://` base URLs, the plaintext equivalents (`grpc`, `ws`) are used.
//!
//! All candidates are probed at the same time. The highest-preference
//! candidate that responds wins. Total probe time is bounded by a single
//! timeout (~5 s) rather than N × timeout.
//!
//! gRPC candidates are verified with an HTTP/2 request (gRPC requires h2).
//! WebSocket candidates use a plain TCP connect.

use std::time::Duration;

use tokio::net::TcpStream;

/// Timeout for individual probe attempts (TCP connect / HTTP request).
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Hard ceiling for the entire parallel detection. Guarantees `detect_transport`
/// never blocks longer than this, even if DNS resolution or TLS hangs.
const DETECT_DEADLINE: Duration = Duration::from_secs(5);

/// Transport scheme for a Matrix gateway connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportScheme {
    Grpcs,
    Grpc,
    Wss,
    Ws,
}

impl TransportScheme {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Grpcs => "grpcs",
            Self::Grpc => "grpc",
            Self::Wss => "wss",
            Self::Ws => "ws",
        }
    }

    fn default_port(&self) -> u16 {
        match self {
            Self::Grpcs | Self::Wss => 443,
            Self::Grpc | Self::Ws => 80,
        }
    }

    fn is_grpc(&self) -> bool {
        matches!(self, Self::Grpcs | Self::Grpc)
    }
}

impl std::fmt::Display for TransportScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A candidate endpoint to probe for reachability.
#[derive(Debug, Clone)]
pub struct TransportCandidate {
    pub scheme: TransportScheme,
    pub host: String,
    pub port: u16,
    url: String,
}

impl TransportCandidate {
    fn new(scheme: TransportScheme, host: String, port: u16) -> Self {
        let url = if port == scheme.default_port() {
            format!("{}://{}", scheme.as_str(), host)
        } else {
            format!("{}://{}:{}", scheme.as_str(), host, port)
        };
        Self {
            scheme,
            host,
            port,
            url,
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// The HTTP(S) URL used for probing (gRPC runs over HTTP/2).
    fn probe_url(&self) -> String {
        let http_scheme = if matches!(self.scheme, TransportScheme::Grpcs | TransportScheme::Wss) {
            "https"
        } else {
            "http"
        };
        if self.port == self.scheme.default_port() {
            format!("{}://{}", http_scheme, self.host)
        } else {
            format!("{}://{}:{}", http_scheme, self.host, self.port)
        }
    }
}

/// Build the ordered list of transport candidates from a base Studio URL.
///
/// For `https://` URLs:
///   1. `grpcs://connectors-{host}:{port}`
///   2. `wss://{host}:{port}`
///
/// For `http://` URLs:
///   1. `grpc://connectors-{host}:{port}`
///   2. `ws://{host}:{port}`
pub fn build_candidates(base_url: &str) -> Vec<TransportCandidate> {
    let trimmed = base_url.trim().trim_end_matches('/');

    let (tls, host_and_port) = if let Some(rest) = trimmed.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        (false, rest)
    } else {
        let rest = trimmed
            .find("://")
            .map(|i| &trimmed[i + 3..])
            .unwrap_or(trimmed);
        (true, rest)
    };

    let (host, port) = parse_host_port(host_and_port, if tls { 443 } else { 80 });

    let grpc_host = format!("connectors-{}", host);

    let (grpc_scheme, ws_scheme) = if tls {
        (TransportScheme::Grpcs, TransportScheme::Wss)
    } else {
        (TransportScheme::Grpc, TransportScheme::Ws)
    };

    vec![
        TransportCandidate::new(grpc_scheme, grpc_host, port),
        TransportCandidate::new(ws_scheme, host.to_string(), port),
    ]
}

/// Probe all candidates **in parallel** and return the highest-preference
/// one that is reachable. Falls back to the WebSocket candidate if nothing
/// responds within the timeout.
///
/// gRPC candidates are probed with an HTTP/2 request (verifies h2 support).
/// WebSocket candidates are probed with a TCP connect.
pub async fn detect_transport(base_url: &str, tls_insecure: bool) -> TransportCandidate {
    let candidates = build_candidates(base_url);
    if candidates.is_empty() {
        tracing::warn!("No transport candidates for {}", base_url);
        return fallback_ws_candidate(base_url);
    }

    tracing::info!(
        "Auto-detecting transport for {} ({} candidates, deadline {:?})",
        base_url,
        candidates.len(),
        DETECT_DEADLINE,
    );
    for c in &candidates {
        tracing::info!(
            "  candidate: {} (probe via {})",
            c.url(),
            if c.scheme.is_grpc() { "HTTP/2" } else { "TCP" }
        );
    }

    let probe_futures: Vec<_> = candidates
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            let url = c.probe_url();
            let is_grpc = c.scheme.is_grpc();
            let host = c.host.clone();
            let port = c.port;
            let display_url = c.url.clone();
            let scheme = c.scheme;
            async move {
                tracing::info!("Probing {} ...", display_url);
                let reachable = if is_grpc {
                    probe_h2(&url, tls_insecure).await
                } else {
                    probe_tcp(&host, port).await
                };
                if reachable {
                    tracing::info!("  {} => reachable ({})", display_url, scheme);
                } else {
                    tracing::info!("  {} => not reachable", display_url);
                }
                (idx, reachable)
            }
        })
        .collect();

    // Hard deadline: even if individual probes hang, we return within DETECT_DEADLINE.
    let results =
        match tokio::time::timeout(DETECT_DEADLINE, futures::future::join_all(probe_futures)).await
        {
            Ok(results) => results,
            Err(_) => {
                let fallback = candidates.into_iter().last().unwrap();
                tracing::warn!(
                    "Transport detection timed out after {:?}, falling back to {}",
                    DETECT_DEADLINE,
                    fallback.url()
                );
                return fallback;
            }
        };

    // Pick the highest-preference (lowest index) candidate that succeeded.
    let winner = results
        .iter()
        .filter(|(_, reachable)| *reachable)
        .min_by_key(|(idx, _)| *idx)
        .map(|(idx, _)| *idx);

    // Log the full result set for diagnostics.
    for (idx, reachable) in &results {
        let c = &candidates[*idx];
        tracing::info!(
            "  result: {} = {}",
            c.url(),
            if *reachable {
                "reachable"
            } else {
                "unreachable"
            }
        );
    }

    if let Some(idx) = winner {
        let chosen = &candidates[idx];
        tracing::info!(
            "Transport selected: {} (scheme: {})",
            chosen.url(),
            chosen.scheme
        );
        chosen.clone()
    } else {
        let fallback = candidates.into_iter().last().unwrap();
        tracing::warn!(
            "No candidates reachable for {}, falling back to {} (scheme: {})",
            base_url,
            fallback.url(),
            fallback.scheme,
        );
        fallback
    }
}

/// Probe with an HTTP/2 request. gRPC requires HTTP/2, so any h2 response
/// (even an error status) confirms the endpoint speaks the right protocol.
async fn probe_h2(url: &str, tls_insecure: bool) -> bool {
    let client = match reqwest::Client::builder()
        .danger_accept_invalid_certs(tls_insecure)
        .connect_timeout(PROBE_TIMEOUT)
        .timeout(PROBE_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Failed to build HTTP client for probe: {}", e);
            return false;
        }
    };

    match client.get(url).send().await {
        Ok(resp) => {
            let is_h2 = resp.version() == reqwest::Version::HTTP_2;
            tracing::info!(
                "HTTP/2 probe {}: status={}, version={:?}, h2={}",
                url,
                resp.status(),
                resp.version(),
                is_h2,
            );
            is_h2
        }
        Err(e) => {
            tracing::info!("HTTP/2 probe {} failed: {}", url, e);
            false
        }
    }
}

/// TCP connect probe with timeout.
async fn probe_tcp(host: &str, port: u16) -> bool {
    let addr = format!("{}:{}", host, port);
    match tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect(&addr)).await {
        Ok(Ok(_)) => {
            tracing::info!("TCP probe {}: connected", addr);
            true
        }
        Ok(Err(e)) => {
            tracing::info!("TCP probe {}: {}", addr, e);
            false
        }
        Err(_) => {
            tracing::info!("TCP probe {}: timed out after {:?}", addr, PROBE_TIMEOUT);
            false
        }
    }
}

/// Build a fallback WS/WSS candidate from the base URL.
fn fallback_ws_candidate(base_url: &str) -> TransportCandidate {
    let trimmed = base_url.trim().trim_end_matches('/');
    let (scheme, host_and_port, default_port) = if let Some(rest) = trimmed.strip_prefix("https://")
    {
        (TransportScheme::Wss, rest, 443u16)
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        (TransportScheme::Ws, rest, 80u16)
    } else {
        let rest = trimmed
            .find("://")
            .map(|i| &trimmed[i + 3..])
            .unwrap_or(trimmed);
        (TransportScheme::Wss, rest, 443u16)
    };
    let (host, port) = parse_host_port(host_and_port, default_port);
    TransportCandidate::new(scheme, host.to_string(), port)
}

/// Parse `host:port` or just `host`, stripping any path component.
/// Handles bracketed IPv6 (`[::1]:8443`) and bare IPv6 (`::1`).
fn parse_host_port(input: &str, default_port: u16) -> (&str, u16) {
    let authority = input.split('/').next().unwrap_or(input);

    // Bracketed IPv6: [::1]:port
    if authority.starts_with('[')
        && let Some(bracket_end) = authority.find(']')
    {
        let host = &authority[..bracket_end + 1];
        let rest = &authority[bracket_end + 1..];
        let port = rest
            .strip_prefix(':')
            .and_then(|p| p.parse().ok())
            .unwrap_or(default_port);
        return (host, port);
    }

    // Bare IPv6 (multiple colons, no brackets) — don't try to split port.
    let colon_count = authority.chars().filter(|&c| c == ':').count();
    if colon_count > 1 {
        return (authority, default_port);
    }

    // IPv4 or hostname: host:port
    if let Some(colon) = authority.rfind(':') {
        let maybe_port = &authority[colon + 1..];
        if let Ok(port) = maybe_port.parse::<u16>() {
            return (&authority[..colon], port);
        }
    }

    (authority, default_port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_candidates_https() {
        let candidates = build_candidates("https://studio.strike48.com");
        assert_eq!(candidates.len(), 2);

        assert_eq!(candidates[0].scheme, TransportScheme::Grpcs);
        assert_eq!(candidates[0].host, "connectors-studio.strike48.com");
        assert_eq!(candidates[0].port, 443);
        assert_eq!(
            candidates[0].url(),
            "grpcs://connectors-studio.strike48.com"
        );

        assert_eq!(candidates[1].scheme, TransportScheme::Wss);
        assert_eq!(candidates[1].host, "studio.strike48.com");
        assert_eq!(candidates[1].port, 443);
        assert_eq!(candidates[1].url(), "wss://studio.strike48.com");
    }

    #[test]
    fn test_build_candidates_http() {
        let candidates = build_candidates("http://studio.strike48.test");
        assert_eq!(candidates.len(), 2);

        assert_eq!(candidates[0].scheme, TransportScheme::Grpc);
        assert_eq!(candidates[0].host, "connectors-studio.strike48.test");
        assert_eq!(candidates[0].port, 80);
        assert_eq!(
            candidates[0].url(),
            "grpc://connectors-studio.strike48.test"
        );

        assert_eq!(candidates[1].scheme, TransportScheme::Ws);
        assert_eq!(candidates[1].host, "studio.strike48.test");
        assert_eq!(candidates[1].port, 80);
        assert_eq!(candidates[1].url(), "ws://studio.strike48.test");
    }

    #[test]
    fn test_build_candidates_with_port() {
        let candidates = build_candidates("https://studio.strike48.com:8443");
        assert_eq!(candidates.len(), 2);

        assert_eq!(candidates[0].port, 8443);
        assert_eq!(
            candidates[0].url(),
            "grpcs://connectors-studio.strike48.com:8443"
        );

        assert_eq!(candidates[1].port, 8443);
        assert_eq!(candidates[1].url(), "wss://studio.strike48.com:8443");
    }

    #[test]
    fn test_build_candidates_with_path() {
        let candidates = build_candidates("https://studio.strike48.com/some/path");
        assert_eq!(candidates[0].host, "connectors-studio.strike48.com");
        assert_eq!(candidates[0].port, 443);
        assert_eq!(candidates[1].host, "studio.strike48.com");
    }

    #[test]
    fn test_build_candidates_trailing_slash() {
        let candidates = build_candidates("https://studio.strike48.com/");
        assert_eq!(candidates[0].host, "connectors-studio.strike48.com");
        assert_eq!(candidates[1].host, "studio.strike48.com");
    }

    #[test]
    fn test_candidate_url_omits_default_port() {
        let c = TransportCandidate::new(TransportScheme::Grpcs, "host.com".into(), 443);
        assert_eq!(c.url(), "grpcs://host.com");

        let c = TransportCandidate::new(TransportScheme::Grpc, "host.com".into(), 80);
        assert_eq!(c.url(), "grpc://host.com");
    }

    #[test]
    fn test_candidate_url_includes_non_default_port() {
        let c = TransportCandidate::new(TransportScheme::Grpcs, "host.com".into(), 8443);
        assert_eq!(c.url(), "grpcs://host.com:8443");
    }

    #[test]
    fn test_candidate_probe_url() {
        let c = TransportCandidate::new(
            TransportScheme::Grpcs,
            "connectors-studio.strike48.com".into(),
            443,
        );
        assert_eq!(c.probe_url(), "https://connectors-studio.strike48.com");

        let c = TransportCandidate::new(TransportScheme::Ws, "studio.strike48.test".into(), 80);
        assert_eq!(c.probe_url(), "http://studio.strike48.test");

        let c = TransportCandidate::new(TransportScheme::Grpc, "host.test".into(), 9090);
        assert_eq!(c.probe_url(), "http://host.test:9090");
    }

    #[test]
    fn test_fallback_ws_candidate() {
        let c = fallback_ws_candidate("https://studio.strike48.com");
        assert_eq!(c.scheme, TransportScheme::Wss);
        assert_eq!(c.host, "studio.strike48.com");
        assert_eq!(c.url(), "wss://studio.strike48.com");

        let c = fallback_ws_candidate("http://studio.strike48.test");
        assert_eq!(c.scheme, TransportScheme::Ws);
        assert_eq!(c.host, "studio.strike48.test");
        assert_eq!(c.url(), "ws://studio.strike48.test");
    }

    #[test]
    fn test_parse_host_port() {
        assert_eq!(
            parse_host_port("studio.strike48.com", 443),
            ("studio.strike48.com", 443)
        );
        assert_eq!(
            parse_host_port("studio.strike48.com:8443", 443),
            ("studio.strike48.com", 8443)
        );
        assert_eq!(
            parse_host_port("studio.strike48.com:80", 443),
            ("studio.strike48.com", 80)
        );
        assert_eq!(
            parse_host_port("studio.strike48.com/path", 443),
            ("studio.strike48.com", 443)
        );
        assert_eq!(
            parse_host_port("studio.strike48.com:9090/path", 443),
            ("studio.strike48.com", 9090)
        );

        // Bracketed IPv6
        assert_eq!(parse_host_port("[::1]:8443", 443), ("[::1]", 8443));
        assert_eq!(parse_host_port("[::1]", 443), ("[::1]", 443));
        assert_eq!(
            parse_host_port("[2001:db8::1]:9090", 443),
            ("[2001:db8::1]", 9090)
        );

        // Bare IPv6 (no brackets) — should not misparse last segment as port
        assert_eq!(parse_host_port("::1", 443), ("::1", 443));
        assert_eq!(parse_host_port("2001:db8::1", 443), ("2001:db8::1", 443));
    }
}
