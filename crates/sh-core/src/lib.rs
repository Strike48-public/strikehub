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
#[cfg(windows)]
pub mod job;
pub mod matrix_ws;
pub mod oauth;
pub mod ott;
pub mod preflight;
// NOT cfg-gated on purpose: `pub use process::{...}` below and the
// cross-platform call sites (`ipc_runner` spawn, `sh-ui` main teardown +
// ProcessTreeGuard) exist on every platform. The unix-only pieces inside
// are individually cfg-gated. (Review of #116: the old `#[cfg(unix)]` here
// made `pub use process::...` an unresolved import — and the whole crate —
// fail to compile for x86_64-pc-windows-msvc, which no PR CI job ever
// checked because the MSVC MSI build is main-gated.)
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
pub use process::{
    ProcessTreeGuard, TEARDOWN_GRACE_SECS, TrackedChild, collect_descendants, spawn_tracked,
    teardown_process_tree, tracked_children_snapshot,
};
#[cfg(unix)]
pub use process::{
    add_managed_root, atexit_teardown, detach_process_group, install_exit_handler, is_managed_exe,
    is_process_group_leader, managed_roots, sweep_managed_roots,
};
pub use proxy::ConnectorProxy;
pub use registry::{
    ConnectorManifest, DEFAULT_CONNECTOR_ID, all_manifests, builtin_manifests, merge_manifests,
};
pub use transport::detect_transport;
pub use ws_relay::WsRelay;
