use dioxus::prelude::*;
use sh_core::{ConnectorStatus, ConnectorTransport};

use super::logo::Strike48Logo;
use super::setup::{ConnectorCards, CustomConnector, SetupConnector, SetupView};

#[component]
pub fn ContentArea(
    active_name: Option<String>,
    active_url: Option<String>,
    active_status: Option<ConnectorStatus>,
    proxy_port: Option<u16>,
    active_port: Option<u16>,
    active_transport: Option<ConnectorTransport>,
    active_id: Option<String>,
    /// Matrix app address for routing through `/app-content/{address}/`.
    #[props(default)]
    matrix_app_address: Option<String>,
    show_setup: bool,
    setup_connectors: Vec<SetupConnector>,
    custom_connectors: Vec<CustomConnector>,
    on_add_custom: EventHandler<(String, String)>,
    on_remove_custom: EventHandler<String>,
    on_select: EventHandler<String>,
    #[props(default)] on_hover: Option<EventHandler<Option<String>>>,
    #[props(default)] hovered_id: Option<String>,
    #[props(default = 0)] auth_version: u32,
    #[props(default = false)] dev_mode: bool,
) -> Element {
    // Choose the content URL based on transport mode:
    //   IPC  → connector://{id}/liveview  (custom protocol handler via bridge)
    //          Bridge injects __MATRIX_API_URL__ pointing at the auth proxy,
    //          which proxies /api/v1alpha → /v1alpha/graphql with JWT.
    //   TCP  → http://127.0.0.1:{proxy}/c/{port}/liveview  (auth proxy)
    let url = match active_transport {
        Some(ConnectorTransport::Ipc) => active_id
            .as_ref()
            .map(|id| format!("dioxus://index.html/connector/{}/liveview", id)),
        _ => match (proxy_port, active_port) {
            (Some(pp), Some(cp)) => Some(format!("http://127.0.0.1:{}/c/{}/liveview", pp, cp)),
            _ => active_url.clone(),
        },
    };

    // Append auth_version as a cache-buster to force iframe reload when auth state changes
    let url = url.map(|u| {
        if auth_version > 0 {
            let sep = if u.contains('?') { "&" } else { "?" };
            format!("{}{}_av={}", u, sep, auth_version)
        } else {
            u
        }
    });

    let active_connectors: Vec<SetupConnector> = setup_connectors
        .iter()
        .filter(|c| c.enabled)
        .cloned()
        .collect();

    rsx! {
        div { class: "content-area",
            if show_setup {
                SetupView {
                    connectors: setup_connectors,
                    custom_connectors: custom_connectors,
                    on_add_custom: on_add_custom,
                    on_remove_custom: on_remove_custom,
                    on_select: on_select,
                    on_hover: on_hover,
                    hovered_id: hovered_id.clone(),
                    dev_mode: dev_mode,
                }
            } else {
                match (&active_name, &url, &active_status) {
                    (Some(_name), Some(url), Some(ConnectorStatus::Online)) => {
                        rsx! {
                            div { class: "content-frame-wrapper",
                                iframe {
                                    class: "content-webview",
                                    src: "{url}",
                                    allow: "clipboard-read; clipboard-write; autoplay; fullscreen; accelerometer; gyroscope",
                                }
                            }
                        }
                    },
                    (Some(name), _, Some(ConnectorStatus::Checking)) => rsx! {
                        div { class: "content-offline",
                            h3 { "Connecting to {name}..." }
                            p { "Checking if the connector is running." }
                        }
                    },
                    (Some(name), _, Some(ConnectorStatus::Offline)) => rsx! {
                        div { class: "content-offline",
                            h3 { "{name} is offline" }
                            p { "The connector will appear here once it's online." }
                        }
                    },
                    _ => {
                        rsx! {
                            div { class: "content-empty",
                                Strike48Logo { width: "180px" }
                                h2 { "StrikeHub" }
                                if !active_connectors.is_empty() {
                                    ConnectorCards {
                                        connectors: active_connectors.clone(),
                                        on_select: on_select,
                                        on_hover: on_hover,
                                        hovered_id: hovered_id.clone(),
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}
