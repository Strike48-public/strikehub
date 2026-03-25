pub mod auth;
pub mod bridge;
pub mod config;
pub mod embedded;
pub mod error;
pub mod ipc;
pub mod ipc_runner;
pub mod matrix_ws;
pub mod oauth;
pub mod ott;
pub mod preflight;
pub mod proxy;
pub mod registry;
pub mod transport;
pub mod ws_relay;

pub use auth::{AuthManager, ConnectorAppInfo, fetch_connector_apps, fetch_tenant_id};
pub use bridge::{BridgeState, SharedBridgeState, new_bridge_state};
pub use config::{
    ConnectorConfig, ConnectorEntry, ConnectorStatus, ConnectorTransport, HubConfig,
    generate_instance_id, slug_from_path, url_slug,
};
pub use error::HubError;
pub use ipc::{IpcAddr, IpcStream};
pub use ipc_runner::IpcConnectorRunner;
pub use matrix_ws::MatrixWsClient;
pub use oauth::{js_string_escape, start_oauth_flow, start_oauth_flow_with};
pub use ott::{create_pre_approved_token, has_saved_credentials, sdk_connector_type};
pub use preflight::{
    AggregatePreflightResult, CheckStatus, ConnectorRuntime, HostOs, PreflightCheck,
    PreflightResult, run_preflight, run_preflight_all, run_preflight_full,
};
pub use proxy::ConnectorProxy;
pub use registry::{ConnectorManifest, builtin_manifests};
pub use transport::detect_transport;
pub use ws_relay::WsRelay;
