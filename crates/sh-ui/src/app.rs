use crate::components::sidebar::ConnectorItem;
use crate::components::{
    AccountView, ContentArea, CustomConnector, LoginOverlay, PickTosOverlay, PreflightOverlay,
    SetupConnector, Sidebar,
};
use crate::theme;
use dioxus::prelude::*;
use sh_core::{
    AggregatePreflightResult, AuthManager, ConnectorConfig, ConnectorProxy, ConnectorRuntime,
    ConnectorStatus, ConnectorTransport, HubConfig, IpcConnectorRunner, MatrixWsClient, WsRelay,
    builtin_manifests, fetch_connector_apps, fetch_tenant_id, run_preflight_all,
    run_preflight_full, start_oauth_flow_with,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::oneshot;

/// Forward environment variables to the connector child process.
///
/// These are read directly from the StrikeHub process env so they're available
/// immediately at connector startup (before AuthManager is initialised).
///
/// Forwarded:
///   TENANT_ID           — Matrix tenant
///   STRIKE48_TENANT     — alias for TENANT_ID
///   STRIKE48_API_URL    — Strike48 HTTP API
///   STRIKE48_URL        — Matrix gateway WSS URL (for registration)
///   INSTANCE_ID         — stable connector instance name
///   MATRIX_TLS_INSECURE — accept self-signed certs
///   KUBESTUDIO_AI       — enable AI features in the connector
///   KUBESTUDIO_MODE     — permission mode (read/write)
fn matrix_env_vars() -> Vec<(String, String)> {
    const FORWARD_KEYS: &[&str] = &[
        "TENANT_ID",
        "STRIKE48_TENANT",
        "STRIKE48_API_URL",
        "STRIKE48_URL",
        "INSTANCE_ID",
        "MATRIX_TLS_INSECURE",
        "KUBESTUDIO_AI",
        "KUBESTUDIO_MODE",
    ];

    // Defaults for keys that should always be present.
    const DEFAULTS: &[(&str, &str)] = &[
        ("STRIKE48_API_URL", "https://studio.strike48.com"),
        ("STRIKE48_URL", "wss://studio.strike48.com"),
    ];

    let mut vars: Vec<(String, String)> = FORWARD_KEYS
        .iter()
        .filter_map(|&key| std::env::var(key).ok().map(|val| (key.to_string(), val)))
        .collect();

    // Fill in defaults for any keys not already present.
    for &(key, default) in DEFAULTS {
        if !vars.iter().any(|(k, _)| k == key) {
            vars.push((key.to_string(), default.to_string()));
        }
    }

    vars
}

/// Helper: rebuild the connectors list from setup + custom state and persist.
fn sync_config(
    setup_connectors: &[SetupConnector],
    custom_connectors: &[CustomConnector],
    hub_config: &mut Signal<HubConfig>,
    connectors: &mut Signal<Vec<ConnectorConfig>>,
) {
    // Snapshot runtime state (fetched name, icon, online status) before rebuilding.
    // sync_config recreates ConnectorConfigs from HubConfig which doesn't carry
    // the display_name/icon fetched at runtime from /connector/info.
    let existing: HashMap<String, ConnectorConfig> = connectors
        .read()
        .iter()
        .map(|c| (c.id.clone(), c.clone()))
        .collect();

    let mut cfg = hub_config.read().clone();
    cfg.connectors.clear();

    for c in setup_connectors {
        if c.enabled {
            cfg.connectors.insert(
                c.manifest.id.to_string(),
                sh_core::ConnectorEntry {
                    display_name: Some(c.manifest.name.to_string()),
                    binary: c.manifest.binary_hint.map(|s| s.to_string()),
                    port: c.manifest.default_port,
                    icon: c.manifest.icon.to_string(),
                    auto_start: true,
                    enabled: true,
                    transport: c.manifest.default_transport,
                    socket_path: None,
                    instance_id: Some(sh_core::generate_instance_id(c.manifest.id)),
                },
            );
        }
    }

    for c in custom_connectors {
        let id = format!("ipc-{}", sh_core::slug_from_path(&c.socket_path));
        let inst_id = sh_core::generate_instance_id(&id);
        cfg.connectors.insert(
            id,
            sh_core::ConnectorEntry {
                display_name: Some(c.name.clone()),
                binary: None,
                port: 0,
                icon: "app".to_string(),
                auto_start: false,
                enabled: true,
                transport: ConnectorTransport::Ipc,
                socket_path: Some(c.socket_path.clone()),
                instance_id: Some(inst_id),
            },
        );
    }

    cfg.setup_complete = true;
    let _ = cfg.save();
    let mut new_connectors = cfg.to_connectors();

    // Restore runtime state for connectors that were already online with
    // fetched info — avoids clobbering names/icons from /connector/info.
    for conn in new_connectors.iter_mut() {
        if let Some(prev) = existing.get(&conn.id) {
            conn.status = prev.status;
            conn.display_name = prev.display_name.clone();
            conn.icon = prev.icon.clone();
        }
    }

    hub_config.set(cfg);
    connectors.set(new_connectors);
}

#[component]
pub fn App() -> Element {
    let mut hub_config = use_signal(|| {
        let mut cfg = HubConfig::load().unwrap_or_else(|_| HubConfig {
            setup_complete: false,
            pick_tos_accepted: false,
            connectors: Default::default(),
            instance_id: None,
        });
        // Merge manifest defaults so that saved configs pick up transport
        // and binary changes when the code is upgraded.
        cfg.apply_manifest_defaults(&builtin_manifests());
        cfg.ensure_instance_ids();
        cfg
    });
    let mut connectors = use_signal(move || hub_config.read().to_connectors());
    let mut active_id = use_signal(|| None::<String>);
    let mut proxy_port = use_signal(|| None::<u16>);
    let mut proxy_handle = use_signal(|| None::<ConnectorProxy>);
    let mut ws_bridge_port = use_signal(|| None::<u16>);
    let mut auth_manager = use_signal(|| None::<AuthManager>);
    let mut is_signed_in = use_signal(|| false);
    let mut signing_in = use_signal(|| false);
    let mut auth_version = use_signal(|| 0u32);
    // Matrix app address discovered via connectorApps GraphQL after sign-in.
    // Used to route content through /app-content/{address}/ for sandbox tokens.
    let mut matrix_app_address = use_signal(|| None::<String>);

    // Show setup on first launch only; returning users go straight to connectors
    let mut show_setup = use_signal(move || !hub_config.read().setup_complete);

    // Show account details view when the profile icon is clicked.
    let mut show_account = use_signal(|| false);

    // Dev mode: show the "Add Connector" form in Settings only when STRIKEHUB_DEV is set.
    let dev_mode = use_signal(|| std::env::var("STRIKEHUB_DEV").is_ok());

    // Persistent WS client for GraphQL queries (shared with preflight).
    let mut ws_client_signal: Signal<Option<Arc<MatrixWsClient>>> = use_signal(|| None);

    // Preflight check state: aggregate results for all enabled connectors.
    // Runs automatically after sign-in; dismissed via Continue/Skip.
    let mut preflight_result: Signal<Option<AggregatePreflightResult>> = use_signal(|| None);
    let mut preflight_checking = use_signal(|| false);
    let mut preflight_dismissed = use_signal(|| false);

    // Register a "connector" asset handler so that connector content can be served
    // through the dioxus:// scheme (which passes the hardcoded navigation handler).
    // URLs like dioxus://index.html/connector/{id}/liveview are routed here.
    #[cfg(feature = "desktop")]
    dioxus::desktop::use_asset_handler("connector", move |request, responder| {
        // The request URI path is e.g. /connector/kubestudio/liveview
        let uri = request.uri().clone();
        let path = uri.path();
        let stripped = path.strip_prefix("/connector/").unwrap_or(path);
        // Preserve query string for the bridge
        let connector_uri = match uri.query() {
            Some(q) => format!("connector://{}?{}", stripped, q),
            None => format!("connector://{}", stripped),
        };

        tokio::spawn(async move {
            let Some(state) = crate::get_bridge_state() else {
                let resp = http::Response::builder()
                    .status(500)
                    .body(Vec::from("bridge state not initialised"))
                    .unwrap();
                responder.respond(resp);
                return;
            };

            let (status, headers, body) =
                sh_core::bridge::handle_bridge_request(state, &connector_uri).await;

            let mut builder = http::Response::builder().status(status);
            for (k, v) in &headers {
                builder = builder.header(k.as_str(), v.as_str());
            }
            let resp = builder.body(body).unwrap();
            responder.respond(resp);
        });
    });

    // Shared hover state between sidebar and content tiles
    let mut hovered_id = use_signal(|| None::<String>);

    // Runners keyed by connector id
    let mut runners: Signal<HashMap<String, IpcConnectorRunner>> = use_signal(HashMap::new);

    // Mutex to prevent race conditions when starting connectors
    let starting_lock: Signal<Arc<tokio::sync::Mutex<HashSet<String>>>> =
        use_signal(|| Arc::new(tokio::sync::Mutex::new(HashSet::new())));

    // Setup state: builtin connectors with enable toggles
    let setup_connectors: Signal<Vec<SetupConnector>> = use_signal(move || {
        let cfg = hub_config.read();
        builtin_manifests()
            .into_iter()
            .map(|m| {
                let enabled = cfg.connectors.get(m.id).map(|e| e.enabled).unwrap_or(true);
                SetupConnector {
                    manifest: m.clone(),
                    enabled,
                }
            })
            .collect()
    });

    // Custom connectors (user-specified IPC sockets for externally-managed services)
    let mut custom_connectors: Signal<Vec<CustomConnector>> = use_signal(move || {
        let cfg = hub_config.read();
        cfg.connectors
            .iter()
            .filter(|(id, _)| id.starts_with("ipc-"))
            .filter_map(|(id, e)| {
                let socket_path = e.socket_path.clone()?;
                Some(CustomConnector {
                    name: e.display_name.clone().unwrap_or_else(|| id.clone()),
                    socket_path,
                })
            })
            .collect()
    });

    // Start proxy + WsRelay on mount (if STRIKE48_API_URL is set).
    // Auth is deferred — user clicks "Sign In" to complete the OAuth flow.
    use_effect(move || {
        spawn(async move {
            let Some(auth) = AuthManager::from_env() else {
                tracing::info!("Auth proxy disabled (no API URL configured)");
                return;
            };

            // Start ConnectorProxy immediately (works with empty token;
            // token is read dynamically on each request).
            match ConnectorProxy::start(auth.clone()).await {
                Ok(p) => {
                    proxy_port.set(Some(p.port()));
                    proxy_handle.set(Some(p));
                }
                Err(e) => {
                    tracing::error!("Failed to start auth proxy: {}", e);
                }
            }

            // Update bridge state with auth info + proxy port
            if let Some(bridge) = crate::get_bridge_state() {
                let mut guard = bridge.write().await;
                guard.auth = Some(auth.clone());
                guard.proxy_port = *proxy_port.peek();
            }

            // Start WsRelay (single WebSocket bridge for IPC connectors)
            let bridge = crate::get_bridge_state().cloned();
            if let Some(bridge) = bridge {
                match WsRelay::start(bridge, Some(auth.clone())).await {
                    Ok(relay) => {
                        let port = relay.port();
                        ws_bridge_port.set(Some(port));
                        // Store in bridge state so the HTML rewriter knows the port
                        if let Some(bs) = crate::get_bridge_state() {
                            bs.write().await.ws_bridge_port = Some(port);
                        }
                        tracing::info!("WsRelay started on port {}", port);
                    }
                    Err(e) => {
                        tracing::error!("Failed to start WsRelay: {}", e);
                    }
                }
            }

            auth_manager.set(Some(auth));
        });
    });

    // Auto-start builtin connectors that were enabled from a previous session.
    // Register custom IPC connectors in bridge state (they're externally managed).
    // Uses read() so the effect re-runs when connectors are populated (e.g. after
    // first-launch setup completes via sync_config). The contains_key guard on
    // runners prevents double-starting connectors that are already running.
    //
    // Auth is always configured (default API URL is baked in), so always
    // delay connector startup until sign-in completes. This ensures
    // TENANT_ID / STRIKE48_TENANT are available in the environment.
    use_effect(move || {
        let signed_in = *is_signed_in.read();
        if !signed_in {
            return;
        }
        let current = connectors.read().clone();
        spawn(async move {
            let mut env_vars = matrix_env_vars();
            // Override STRIKE48_API_URL so the connector's chat panel routes
            // API calls through our proxy (which proxies /api/v1alpha
            // → /v1alpha/graphql with Keycloak JWT as ?token= param).
            if let Some(pp) = *proxy_port.peek() {
                env_vars.push((
                    "STRIKE48_API_URL".into(),
                    format!("http://127.0.0.1:{}", pp),
                ));
            }

            for conn in &current {
                // Custom IPC connectors are externally managed — just register
                // their socket in bridge state so the protocol handler can reach them.
                if conn.id.starts_with("ipc-") {
                    if let Some(bridge) = crate::get_bridge_state() {
                        bridge
                            .write()
                            .await
                            .sockets
                            .insert(conn.id.clone(), conn.ipc_addr());
                    }
                    continue;
                }

                // Check if already running
                if runners.read().contains_key(&conn.id) {
                    continue;
                }

                // Acquire lock and check if another task is already starting this connector
                let lock = starting_lock.read().clone();
                let mut starting = lock.lock().await;

                // Double-check after acquiring lock (another task may have started it)
                if starting.contains(&conn.id) || runners.read().contains_key(&conn.id) {
                    continue;
                }

                // Mark as starting to prevent race condition
                starting.insert(conn.id.clone());
                drop(starting); // Release lock early

                let Some(ref binary) = conn.binary else {
                    tracing::warn!(
                        "IPC connector '{}' has no binary configured, skipping",
                        conn.id
                    );
                    // Remove from starting set
                    let mut starting = lock.lock().await;
                    starting.remove(&conn.id);
                    continue;
                };
                let binary_path = std::path::PathBuf::from(binary);
                // Per-connector instance ID (persisted in config).
                let mut conn_env = env_vars.clone();
                conn_env.push(("INSTANCE_ID".into(), conn.instance_id.clone()));
                tracing::info!(
                    "Starting connector '{}' with INSTANCE_ID={}",
                    conn.id,
                    conn.instance_id
                );
                match IpcConnectorRunner::start(&conn.id, &binary_path, &conn_env).await {
                    Ok(runner) => {
                        tracing::info!(
                            "started IPC connector '{}' → {}",
                            conn.id,
                            runner.ipc_addr()
                        );
                        // Register IPC address in bridge state
                        if let Some(bridge) = crate::get_bridge_state() {
                            bridge
                                .write()
                                .await
                                .sockets
                                .insert(conn.id.clone(), runner.ipc_addr().clone());
                        }
                        // Fetch info
                        let mut updated = connectors.read().clone();
                        if let Some(c) = updated.iter_mut().find(|c| c.id == conn.id) {
                            if runner.health_check().await {
                                c.status = ConnectorStatus::Online;
                            }
                            if let Some((name, icon)) = runner.fetch_info().await {
                                c.display_name = name;
                                if let Some(icon) = icon {
                                    c.icon = icon;
                                }
                            }
                        }
                        // Insert into runners BEFORE updating connectors —
                        // connectors.set() re-triggers this effect (read subscription),
                        // and the runners guard must already have the entry to prevent
                        // double-spawning.
                        runners.write().insert(conn.id.clone(), runner);

                        // Remove from starting set now that it's running
                        let mut starting = lock.lock().await;
                        starting.remove(&conn.id);
                        drop(starting);

                        connectors.set(updated);
                    }
                    Err(e) => {
                        tracing::error!("failed to start IPC connector '{}': {}", conn.id, e);

                        // Remove from starting set on failure
                        let mut starting = lock.lock().await;
                        starting.remove(&conn.id);
                    }
                }
            }
        });
    });

    // Periodic health checks every 3 seconds
    use_effect(move || {
        spawn(async move {
            let mut info_fetched = std::collections::HashSet::<String>::new();
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                let current = connectors.read().clone();
                if current.is_empty() {
                    continue;
                }
                let mut updated = current.clone();
                for conn in updated.iter_mut() {
                    // Health check over Unix socket — try managed runner first,
                    // fall back to direct socket check for custom connectors.
                    let healthy = {
                        let runners_guard = runners.read();
                        if let Some(r) = runners_guard.get(&conn.id) {
                            r.health_check().await
                        } else {
                            check_health_ipc(&conn.ipc_addr()).await
                        }
                    };

                    conn.status = if healthy {
                        ConnectorStatus::Online
                    } else {
                        ConnectorStatus::Offline
                    };

                    // Only fetch connector info once per connector
                    if conn.status == ConnectorStatus::Online && !info_fetched.contains(&conn.id) {
                        let runners_guard = runners.read();
                        if let Some(r) = runners_guard.get(&conn.id) {
                            if let Some((name, icon)) = r.fetch_info().await {
                                conn.display_name = name;
                                if let Some(icon) = icon {
                                    conn.icon = icon;
                                }
                                info_fetched.insert(conn.id.clone());
                            }
                        } else if let Some(info) = fetch_connector_info_ipc(&conn.ipc_addr()).await
                        {
                            conn.display_name = info.name;
                            if let Some(icon) = info.icon {
                                conn.icon = icon;
                            }
                            info_fetched.insert(conn.id.clone());
                        }
                    }
                    // If connector goes offline, allow re-fetch when it comes back
                    if conn.status == ConnectorStatus::Offline {
                        info_fetched.remove(&conn.id);
                    }
                }
                connectors.set(updated);
            }
        });
    });

    // Auth is always configured (default API URL baked in), so this is always true.
    // We still check auth_manager for the edge case where from_env() somehow fails.
    let has_matrix_url = sh_core::AuthManager::from_env().is_some();

    // Sign-in coroutine: lives on App's scope (never unmounted) so the
    // async work survives LoginOverlay being unmounted during the flow.
    // The OAuth browser callback runs in tokio::spawn (Send-safe), while
    // signal updates happen here in the Dioxus coroutine context.
    let sign_in_coro = use_coroutine(move |mut rx: dioxus::prelude::UnboundedReceiver<()>| {
        async move {
            use futures_util::StreamExt;
            while rx.next().await.is_some() {
                let auth = auth_manager.read().clone();
                let Some(auth) = auth else { continue };
                signing_in.set(true);

                let auth_clone = auth.clone();
                let callback_base = std::env::var("STRIKEHUB_CALLBACK_URL").ok();
                let browser_api_url = std::env::var("STRIKE48_EXTERNAL_URL").ok();

                // In server/liveview mode, we can't call open::that() on the
                // server. Instead, the OAuth flow sends the login URL back via
                // a oneshot channel, and we open it client-side via JS eval.
                #[cfg(not(feature = "desktop"))]
                let (url_tx, url_rx) = oneshot::channel::<String>();
                #[cfg(feature = "desktop")]
                let url_tx_opt: Option<oneshot::Sender<String>> = None;
                #[cfg(not(feature = "desktop"))]
                let url_tx_opt = Some(url_tx);

                let oauth_handle = tokio::spawn(async move {
                    let matrix_url = auth_clone.matrix_url().to_string();
                    let tls_insecure = auth_clone.tls_insecure();
                    start_oauth_flow_with(
                        &matrix_url,
                        tls_insecure,
                        callback_base,
                        browser_api_url,
                        url_tx_opt,
                    )
                    .await
                });

                // In server mode, wait for the login URL and open it in the
                // user's browser via JavaScript.
                #[cfg(not(feature = "desktop"))]
                {
                    if let Ok(login_url) = url_rx.await {
                        let js = format!(
                            "window.open('{}', '_blank')",
                            login_url.replace('\'', "\\'")
                        );
                        let _ = document::eval(&js);
                    }
                }

                let oauth_result = match oauth_handle.await {
                    Ok(Ok(result)) => result,
                    Ok(Err(e)) => {
                        tracing::error!("Sign-in failed: {:#}", e);
                        signing_in.set(false);
                        continue;
                    }
                    Err(e) => {
                        tracing::error!("Sign-in task panicked: {:#}", e);
                        signing_in.set(false);
                        continue;
                    }
                };

                auth.set_token(
                    oauth_result.access_token,
                    oauth_result.refresh_token,
                    oauth_result.token_endpoint,
                    oauth_result.client_id,
                );
                auth.spawn_refresh_loop();
                // Bring StrikeHub to the foreground so the user doesn't have to
                // manually switch back from the browser after OAuth.
                #[cfg(feature = "desktop")]
                dioxus::desktop::window().set_focus();
                let next = *auth_version.peek() + 1;
                auth_version.set(next);
                tracing::info!("OAuth tokens acquired, resolving tenant...");

                // Post-sign-in setup (WS client, sandbox token, etc.)
                let ws_client = match MatrixWsClient::connect(auth.clone()).await {
                    Ok(ws) => {
                        let ws = Arc::new(ws);
                        if let Some(proxy) = proxy_handle.peek().as_ref() {
                            proxy.set_matrix_ws(ws.clone()).await;
                        }
                        tracing::info!("MatrixWsClient created and attached to proxy");
                        ws_client_signal.set(Some(ws.clone()));
                        Some(ws)
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to create MatrixWsClient: {} — \
                             falling back to HTTP-only GraphQL",
                            e
                        );
                        None
                    }
                };

                let tenant = fetch_tenant_id(&auth, ws_client.as_deref())
                    .await
                    .or_else(|| {
                        let v = std::env::var("TENANT_ID").ok()?;
                        (!v.is_empty()).then_some(v)
                    })
                    .unwrap_or_default();
                tracing::info!("Resolved tenant_id={:?}", tenant);
                let instance = hub_config.read().instance_id.clone().unwrap_or_default();
                tracing::info!("INSTANCE_ID={:?}", instance);

                // Propagate tenant to child connectors via process env so
                // restarts and late-spawned connectors pick it up.
                if !tenant.is_empty() {
                    // SAFETY: we are single-threaded in the Dioxus runtime at
                    // this point; no other thread reads these env vars yet.
                    unsafe {
                        std::env::set_var("TENANT_ID", &tenant);
                        std::env::set_var("STRIKE48_TENANT", &tenant);
                    }
                    tracing::info!("Set TENANT_ID and STRIKE48_TENANT in process env");
                }

                // Signal sign-in AFTER tenant is in env so the connector
                // startup effect sees the tenant when it fires.
                is_signed_in.set(true);
                tracing::info!("Sign-in completed, connectors may now start");

                if !tenant.is_empty() && !instance.is_empty() {
                    let addr = format!("matrix:{}:app-kube-studio:{}", tenant, instance);
                    tracing::info!("Matrix app address: {}", addr);
                    matrix_app_address.set(Some(addr.clone()));

                    match auth.bootstrap_sandbox_token(&addr).await {
                        Ok(()) => {
                            tracing::info!("Sandbox token bootstrapped successfully");
                            auth.spawn_sandbox_refresh_loop();
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to bootstrap sandbox token: {} — \
                                 GraphQL will route through WS client",
                                e
                            );
                        }
                    }

                    let apps = fetch_connector_apps(&auth, ws_client.as_deref()).await;
                    if !apps.is_empty() {
                        tracing::info!("Discovered {} connector app(s) from Matrix", apps.len());
                    }

                    let mut updated = connectors.read().clone();
                    for c in updated.iter_mut() {
                        if c.transport == ConnectorTransport::Ipc && !c.id.starts_with("ipc-") {
                            c.matrix_app_address = Some(addr.clone());
                        }
                    }
                    connectors.set(updated);
                    let next = *auth_version.peek() + 1;
                    auth_version.set(next);
                }

                signing_in.set(false);
                tracing::info!("signing_in reset to false");
            }
        }
    });

    // Run preflight checks for all enabled connectors after sign-in.
    // Waits a few seconds for connectors to start and register before
    // checking their Matrix registration status.
    use_effect(move || {
        let signed_in = *is_signed_in.read();
        if signed_in && !*preflight_dismissed.peek() && preflight_result.peek().is_none() {
            let connector_ids: Vec<String> = {
                let ids: Vec<String> = connectors
                    .read()
                    .iter()
                    .filter(|c| !c.id.starts_with("ipc-"))
                    .map(|c| c.id.clone())
                    .collect();
                if ids.is_empty() {
                    builtin_manifests()
                        .iter()
                        .map(|m| m.id.to_string())
                        .collect()
                } else {
                    ids
                }
            };
            let auth = auth_manager.read().clone();
            preflight_checking.set(true);
            spawn(async move {
                // Give connectors time to start and register with Matrix.
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;

                // Build runtime info from current connector state.
                let runtimes: Vec<ConnectorRuntime> = connectors
                    .read()
                    .iter()
                    .filter(|c| !c.id.starts_with("ipc-"))
                    .map(|c| ConnectorRuntime {
                        id: c.id.clone(),
                        name: c.display_name.clone(),
                        status: c.status,
                    })
                    .collect();

                let ws = ws_client_signal.read().clone();
                let result = if let Some(ref auth) = auth {
                    run_preflight_full(&connector_ids, auth, ws.as_deref(), &runtimes).await
                } else {
                    run_preflight_all(&connector_ids).await
                };
                preflight_result.set(Some(result));
                preflight_checking.set(false);
            });
        }
    });

    let on_sign_in = move |_: ()| {
        sign_in_coro.send(());
    };

    let on_sign_out = move |_: ()| {
        if let Some(auth) = auth_manager.read().as_ref() {
            auth.clear_auth();
        }
        is_signed_in.set(false);
        signing_in.set(false);
        matrix_app_address.set(None);
        ws_client_signal.set(None);
        preflight_result.set(None);
        preflight_dismissed.set(false);
        show_account.set(false);
        // Detach stale WS client from the proxy so the next sign-in starts fresh
        spawn(async move {
            if let Some(proxy) = proxy_handle.peek().as_ref() {
                proxy.clear_matrix_ws().await;
            }
        });
        let next = *auth_version.peek() + 1;
        auth_version.set(next);
        tracing::info!("Signed out");
    };

    let on_select = move |id: String| {
        // If setup hasn't been completed yet, sync config first to populate
        // the connectors signal from the manifest defaults.
        if !hub_config.read().setup_complete {
            let sc = setup_connectors.read().clone();
            let cc = custom_connectors.read().clone();
            sync_config(&sc, &cc, &mut hub_config, &mut connectors);
        }
        active_id.set(Some(id));
        hovered_id.set(None);
        show_setup.set(false);
        show_account.set(false);
    };

    // Add custom IPC connector: appears in sidebar immediately
    let on_add_custom = move |(name, socket_path): (String, String)| {
        {
            let mut cc = custom_connectors.write();
            if cc.iter().any(|c| c.socket_path == socket_path) {
                return;
            }
            cc.push(CustomConnector {
                name: name.clone(),
                socket_path: socket_path.clone(),
            });
        }
        // Sync sidebar + persist
        let sc = setup_connectors.read().clone();
        let cc = custom_connectors.read().clone();
        sync_config(&sc, &cc, &mut hub_config, &mut connectors);

        // Register the socket in bridge state so the custom protocol handler can find it
        let id = format!("ipc-{}", sh_core::slug_from_path(&socket_path));
        let ipc_addr = sh_core::IpcAddr::from_string(&socket_path);
        let id_for_health = id.clone();
        spawn(async move {
            if let Some(bridge) = crate::get_bridge_state() {
                bridge
                    .write()
                    .await
                    .sockets
                    .insert(id.clone(), ipc_addr.clone());
            }
            // Health check over IPC
            if check_health_ipc(&ipc_addr).await {
                let mut updated = connectors.read().clone();
                if let Some(c) = updated.iter_mut().find(|c| c.id == id_for_health) {
                    c.status = ConnectorStatus::Online;
                }
                connectors.set(updated);
            }
        });
    };

    // Remove custom connector: disappears from sidebar immediately
    let on_remove_custom = move |socket_path: String| {
        let id = format!("ipc-{}", sh_core::slug_from_path(&socket_path));
        {
            let mut cc = custom_connectors.write();
            cc.retain(|c| c.socket_path != socket_path);
        }
        // Sync sidebar + persist
        let sc = setup_connectors.read().clone();
        let cc = custom_connectors.read().clone();
        sync_config(&sc, &cc, &mut hub_config, &mut connectors);
        // Remove from bridge state
        if let Some(bridge) = crate::get_bridge_state() {
            let bridge = bridge.clone();
            let id_for_cleanup = id.clone();
            spawn(async move {
                bridge.write().await.sockets.remove(&id_for_cleanup);
            });
        }
        // Clear active selection if it was this connector
        if active_id.read().as_ref() == Some(&id) {
            active_id.set(None);
        }
    };

    let on_hover = move |id: Option<String>| {
        hovered_id.set(id);
    };

    let on_settings = move |_: ()| {
        active_id.set(None);
        hovered_id.set(None);
        show_setup.set(true);
        show_account.set(false);
    };

    let on_account = move |_: ()| {
        active_id.set(None);
        hovered_id.set(None);
        show_setup.set(false);
        show_account.set(true);
    };

    let current_connectors = connectors.read();

    let sidebar_items: Vec<ConnectorItem> = current_connectors
        .iter()
        .map(|c| ConnectorItem {
            id: c.id.clone(),
            display_name: c.display_name.clone(),
            icon: c.icon.clone(),
            status: c.status,
        })
        .collect();

    let active = active_id
        .read()
        .as_ref()
        .and_then(|id| current_connectors.iter().find(|c| &c.id == id))
        .cloned();

    let active_name = active.as_ref().map(|c| c.display_name.clone());
    let active_url = active.as_ref().map(|c| c.url());
    let active_status = active.as_ref().map(|c| c.status);
    let active_port = active.as_ref().map(|c| c.port);
    let active_transport = active.as_ref().map(|c| c.transport);
    let active_conn_id = active.as_ref().map(|c| c.id.clone());
    let active_matrix_addr = active.as_ref().and_then(|c| c.matrix_app_address.clone());
    let current_proxy_port = *proxy_port.read();

    let setup_list = setup_connectors.read().clone();
    let custom_list = custom_connectors.read().clone();
    let is_setup = *show_setup.read();
    let is_account = *show_account.read();

    // Account details for account view
    let account_server_url = auth_manager
        .read()
        .as_ref()
        .map(|a| a.matrix_url().to_string())
        .unwrap_or_default();
    let account_tenant_id = std::env::var("TENANT_ID").unwrap_or_default();
    let account_instance_id = hub_config.read().instance_id.clone().unwrap_or_default();

    // Show aggregate preflight overlay after sign-in until dismissed.
    // While checks are still running (preflight_result is None), keep showing
    // the overlay with a "checking" state to avoid flashing the home screen.
    let show_preflight = if *is_signed_in.read() && !*preflight_dismissed.read() {
        Some(
            preflight_result
                .read()
                .clone()
                .unwrap_or(AggregatePreflightResult { results: vec![] }),
        )
    } else {
        None
    };
    let is_preflight_checking = *preflight_checking.read();

    rsx! {
        style { "{theme::theme_css()}" }
        style { "{theme::app_css()}" }

        div { class: "app-shell",
            div {
                class: "app-container",
                if show_preflight.is_none() {
                    Sidebar {
                        connectors: sidebar_items,
                        active_id: active_id.read().clone(),
                        show_settings: is_setup,
                        on_select: on_select,
                        on_hover: on_hover,
                        hovered_id: hovered_id.read().clone(),
                        on_settings: on_settings,
                        is_signed_in: *is_signed_in.read(),
                        signing_in: *signing_in.read(),
                        has_matrix_url: has_matrix_url,
                        show_account: is_account,
                        on_sign_in: on_sign_in,
                        on_sign_out: on_sign_out,
                        on_account: on_account,
                    }
                }
                if !*is_signed_in.read() && has_matrix_url {
                    LoginOverlay {
                        on_sign_in: on_sign_in,
                        signing_in: *signing_in.read(),
                    }
                } else if active_id.read().as_deref() == Some("pick") && !hub_config.read().pick_tos_accepted {
                    PickTosOverlay {
                        on_accept: move |_: ()| {
                            let mut cfg = hub_config.read().clone();
                            cfg.pick_tos_accepted = true;
                            let _ = cfg.save();
                            hub_config.set(cfg);
                        },
                        on_decline: move |_: ()| {
                            active_id.set(None);
                        },
                    }
                } else if let Some(aggregate_result) = show_preflight {
                    PreflightOverlay {
                        result: aggregate_result,
                        checking: is_preflight_checking,
                        on_recheck: move |_: ()| {
                            preflight_checking.set(true);
                            let auth = auth_manager.read().clone();
                            let ids: Vec<String> = builtin_manifests()
                                .iter()
                                .map(|m| m.id.to_string())
                                .collect();
                            spawn(async move {
                                let runtimes: Vec<ConnectorRuntime> = connectors
                                    .read()
                                    .iter()
                                    .filter(|c| !c.id.starts_with("ipc-"))
                                    .map(|c| ConnectorRuntime {
                                        id: c.id.clone(),
                                        name: c.display_name.clone(),
                                        status: c.status,
                                    })
                                    .collect();
                                let ws = ws_client_signal.read().clone();
                                let result = if let Some(ref auth) = auth {
                                    run_preflight_full(&ids, auth, ws.as_deref(), &runtimes).await
                                } else {
                                    run_preflight_all(&ids).await
                                };
                                preflight_result.set(Some(result));
                                preflight_checking.set(false);
                            });
                        },
                        on_continue: move |_: ()| {
                            preflight_dismissed.set(true);
                        },
                    }
                } else if is_account {
                    AccountView {
                        server_url: account_server_url,
                        tenant_id: account_tenant_id,
                        instance_id: account_instance_id,
                        on_sign_out: on_sign_out,
                    }
                } else {
                    ContentArea {
                        active_name: active_name,
                        active_url: active_url,
                        active_status: active_status,
                        proxy_port: current_proxy_port,
                        active_port: active_port,
                        active_transport: active_transport,
                        active_id: active_conn_id,
                        matrix_app_address: active_matrix_addr,
                        show_setup: is_setup,
                        setup_connectors: setup_list,
                        custom_connectors: custom_list,
                        on_add_custom: on_add_custom,
                        on_remove_custom: on_remove_custom,
                        on_select: on_select,
                        on_hover: on_hover,
                        hovered_id: hovered_id.read().clone(),
                        auth_version: *auth_version.read(),
                        dev_mode: *dev_mode.read(),
                    }
                }
            }
        }
    }
}

struct ConnectorInfo {
    name: String,
    icon: Option<String>,
}

/// Fetch connector info over IPC (for custom IPC connectors without a runner).
async fn fetch_connector_info_ipc(addr: &sh_core::IpcAddr) -> Option<ConnectorInfo> {
    use http_body_util::BodyExt;
    use hyper_util::rt::TokioIo;

    let stream = sh_core::IpcStream::connect(addr).await.ok()?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.ok()?;
    tokio::spawn(conn);
    let req = hyper::Request::builder()
        .uri("/connector/info")
        .body(http_body_util::Empty::<hyper::body::Bytes>::new())
        .ok()?;
    let resp = sender.send_request(req).await.ok()?;
    let body_bytes = resp.into_body().collect().await.ok()?.to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).ok()?;
    let name = body.get("name")?.as_str()?.to_string();
    let icon = body.get("icon").and_then(|v| v.as_str()).map(String::from);
    Some(ConnectorInfo { name, icon })
}

/// Health check for an externally-managed IPC connector.
async fn check_health_ipc(addr: &sh_core::IpcAddr) -> bool {
    use hyper_util::rt::TokioIo;

    let Ok(stream) = sh_core::IpcStream::connect(addr).await else {
        return false;
    };
    let io = TokioIo::new(stream);
    let Ok((mut sender, conn)) = hyper::client::conn::http1::handshake(io).await else {
        return false;
    };
    tokio::spawn(conn);
    let req = hyper::Request::builder()
        .uri("/health")
        .body(http_body_util::Empty::<hyper::body::Bytes>::new());
    let Ok(req) = req else { return false };
    match sender.send_request(req).await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}
