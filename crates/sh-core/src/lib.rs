pub mod auth;
pub mod bridge;
pub mod config;
pub mod error;
pub mod ipc;
pub mod ipc_runner;
pub mod matrix_ws;
pub mod oauth;
pub mod proxy;
pub mod registry;
pub mod ws_relay;

pub use auth::{AuthManager, ConnectorAppInfo, fetch_connector_apps, fetch_tenant_id};
pub use bridge::{BridgeState, SharedBridgeState, new_bridge_state};
pub use config::{
    ConnectorConfig, ConnectorEntry, ConnectorStatus, ConnectorTransport, HubConfig, slug_from_path,
};
pub use error::HubError;
pub use ipc::{IpcAddr, IpcStream};
pub use ipc_runner::IpcConnectorRunner;
pub use matrix_ws::MatrixWsClient;
pub use oauth::{start_oauth_flow, start_oauth_flow_with};
pub use proxy::ConnectorProxy;
pub use registry::{ConnectorManifest, builtin_manifests};
pub use ws_relay::WsRelay;
