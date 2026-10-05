pub mod allowlist;
pub mod auth;
pub mod bridge;
pub mod config;
pub mod connector_fetch;
pub mod connector_seed;
pub mod connector_version;
pub mod embedded;
pub mod error;
pub mod ipc;
pub mod ipc_runner;
pub mod matrix_ws;
pub mod oauth;
pub mod ott;
pub mod preflight;
#[cfg(unix)]
pub mod process;
pub mod proxy;
pub mod registry;
#[cfg(feature = "sentry")]
pub mod sentry_init;
pub mod transport;
pub mod webview2;
pub mod ws_relay;

pub use allowlist::{RepoAllowlist, get_allowlist, init_allowlist, load_allowlist};
pub use auth::{AuthManager, ConnectorAppInfo, fetch_connector_apps, fetch_tenant_id};
pub use bridge::{BridgeState, SharedBridgeState, new_bridge_state};
pub use config::{
    AllowlistConfig, ConnectorConfig, ConnectorEntry, ConnectorStatus, ConnectorTransport,
    DynamicConnectorDef, HubConfig, StartFailure, default_easy_mode, generate_instance_id, log_dir,
    resolve_easy_mode, slug_from_path, url_slug,
};
pub use connector_fetch::{EnsureResult, bin_cache_dir, ensure_all_connector_binaries};
pub use connector_seed::seed_bundled_connectors;
pub use error::HubError;
pub use ipc::{IpcAddr, IpcStream};
pub use ipc_runner::IpcConnectorRunner;
pub use matrix_ws::MatrixWsClient;
pub use oauth::{SignInCancelled, js_string_escape, start_oauth_flow, start_oauth_flow_with};
pub use ott::{
    PreApprovedOtt, create_pre_approved_token, has_saved_credentials, sdk_connector_type,
};
pub use preflight::{
    AggregatePreflightResult, CheckStatus, ConnectorRuntime, HostOs, PreflightCheck,
    PreflightResult, run_preflight, run_preflight_all, run_preflight_full,
};
#[cfg(unix)]
pub use process::{detach_process_group, is_process_group_leader};
pub use proxy::ConnectorProxy;
pub use registry::{
    ConnectorManifest, DEFAULT_CONNECTOR_ID, all_manifests, builtin_manifests, merge_manifests,
};
pub use transport::detect_transport;
pub use ws_relay::WsRelay;
