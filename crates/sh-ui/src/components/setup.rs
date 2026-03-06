use crate::components::logo::Strike48Logo;
use crate::components::sidebar::hero_icon_path;
use dioxus::prelude::*;
use sh_core::registry::ConnectorManifest;

/// State for a builtin connector in the setup view.
#[derive(Debug, Clone, PartialEq)]
pub struct SetupConnector {
    pub manifest: ConnectorManifest,
    pub enabled: bool,
}

/// A custom (user-added) connector communicating over a Unix socket.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomConnector {
    pub name: String,
    pub socket_path: String,
}

/// Shared card grid used by both the Settings view and the dashboard landing page.
#[component]
pub fn ConnectorCards(
    connectors: Vec<SetupConnector>,
    #[props(default)] custom_connectors: Vec<CustomConnector>,
    #[props(default)] on_remove_custom: Option<EventHandler<String>>,
    #[props(default)] on_add_custom: Option<EventHandler<(String, String)>>,
    #[props(default)] on_select: Option<EventHandler<String>>,
    #[props(default)] on_hover: Option<EventHandler<Option<String>>>,
    #[props(default)] hovered_id: Option<String>,
    #[props(default = false)] dev_mode: bool,
) -> Element {
    let mut custom_name = use_signal(String::new);
    let mut custom_socket = use_signal(String::new);

    rsx! {
        div { class: "connector-cards",
            // Builtin connector cards
            for conn in connectors.iter() {
                {
                    let icon_path = hero_icon_path(conn.manifest.icon);
                    let id = conn.manifest.id.to_string();
                    let select_id = id.clone();
                    let hover_id = id.clone();
                    let is_hovered = hovered_id.as_ref() == Some(&id);
                    let card_class = if is_hovered {
                        "connector-card hovered"
                    } else {
                        "connector-card"
                    };
                    rsx! {
                        div {
                            class: "{card_class}",
                            onclick: move |_| {
                                if let Some(on_select) = on_select {
                                    on_select.call(select_id.clone());
                                }
                            },
                            onmouseenter: move |_| {
                                if let Some(on_hover) = on_hover {
                                    on_hover.call(Some(hover_id.clone()));
                                }
                            },
                            onmouseleave: move |_| {
                                if let Some(on_hover) = on_hover {
                                    on_hover.call(None);
                                }
                            },
                            div { class: "card-icon-wrapper",
                                svg {
                                    class: "card-icon",
                                    width: "32",
                                    height: "32",
                                    view_box: "0 0 24 24",
                                    fill: "none",
                                    xmlns: "http://www.w3.org/2000/svg",
                                    path {
                                        d: "{icon_path}",
                                        stroke: "currentColor",
                                        stroke_width: "1.5",
                                        stroke_linecap: "round",
                                        stroke_linejoin: "round",
                                    }
                                }
                            }
                            h3 { class: "card-name", "{conn.manifest.name}" }
                            p { class: "card-description", "{conn.manifest.description}" }
                        }
                    }
                }
            }

            // Individual custom connector cards
            for cc in custom_connectors.iter() {
                {
                    let icon_path = hero_icon_path("hero-puzzle-piece");
                    let socket_path = cc.socket_path.clone();
                    let id = format!("ipc-{}", sh_core::slug_from_path(&cc.socket_path));
                    let select_id = id.clone();
                    let hover_id = id.clone();
                    let is_hovered = hovered_id.as_ref() == Some(&id);
                    let card_class = if is_hovered {
                        "connector-card hovered"
                    } else {
                        "connector-card"
                    };
                    rsx! {
                        div {
                            class: "{card_class}",
                            onclick: move |_| {
                                if let Some(on_select) = on_select {
                                    on_select.call(select_id.clone());
                                }
                            },
                            onmouseenter: move |_| {
                                if let Some(on_hover) = on_hover {
                                    on_hover.call(Some(hover_id.clone()));
                                }
                            },
                            onmouseleave: move |_| {
                                if let Some(on_hover) = on_hover {
                                    on_hover.call(None);
                                }
                            },
                            div { class: "card-icon-wrapper",
                                svg {
                                    class: "card-icon",
                                    width: "32",
                                    height: "32",
                                    view_box: "0 0 24 24",
                                    fill: "none",
                                    xmlns: "http://www.w3.org/2000/svg",
                                    path {
                                        d: "{icon_path}",
                                        stroke: "currentColor",
                                        stroke_width: "1.5",
                                        stroke_linecap: "round",
                                        stroke_linejoin: "round",
                                    }
                                }
                            }
                            if let Some(on_remove) = on_remove_custom {
                                button {
                                    class: "card-remove-btn",
                                    title: "Remove",
                                    onclick: move |e| {
                                        e.stop_propagation();
                                        on_remove.call(socket_path.clone());
                                    },
                                    "\u{00d7}"
                                }
                            }
                            h3 { class: "card-name", "{cc.name}" }
                            p { class: "card-description card-socket-path", "{cc.socket_path}" }
                        }
                    }
                }
            }

            // Add custom connector card (form) — only visible in dev mode
            if dev_mode {
                if let Some(on_add) = on_add_custom {
                    {
                        let icon_path = hero_icon_path("hero-puzzle-piece");
                        rsx! {
                            div { class: "connector-card add-card",
                                div { class: "card-icon-wrapper",
                                    svg {
                                        class: "card-icon",
                                        width: "32",
                                        height: "32",
                                        view_box: "0 0 24 24",
                                        fill: "none",
                                        xmlns: "http://www.w3.org/2000/svg",
                                        path {
                                            d: "{icon_path}",
                                            stroke: "currentColor",
                                            stroke_width: "1.5",
                                            stroke_linecap: "round",
                                            stroke_linejoin: "round",
                                        }
                                    }
                                }
                                h3 { class: "card-name", "Add Connector" }
                                p { class: "card-description", "Connect to a service on a Unix socket." }
                                form {
                                    class: "custom-card-form",
                                    onsubmit: move |e: Event<FormData>| {
                                        let name_val = e.values().get("cname").map(|v| v.as_value()).unwrap_or_default();
                                        let socket_val = e.values().get("csocket").map(|v| v.as_value()).unwrap_or_default();
                                        let socket_trimmed = socket_val.trim().to_string();
                                        if !socket_trimmed.is_empty() {
                                            let n = if name_val.trim().is_empty() {
                                                // Derive name from the socket filename
                                                std::path::Path::new(&socket_trimmed)
                                                    .file_stem()
                                                    .and_then(|s| s.to_str())
                                                    .unwrap_or("custom")
                                                    .to_string()
                                            } else {
                                                name_val.trim().to_string()
                                            };
                                            on_add.call((n, socket_trimmed));
                                            custom_name.set(String::new());
                                            custom_socket.set(String::new());
                                        }
                                    },
                                    input {
                                        class: "custom-name-input",
                                        r#type: "text",
                                        name: "cname",
                                        placeholder: "Name (optional)",
                                        value: "{custom_name}",
                                        oninput: move |e: Event<FormData>| custom_name.set(e.value()),
                                    }
                                    input {
                                        class: "custom-socket-input",
                                        r#type: "text",
                                        name: "csocket",
                                        placeholder: if cfg!(windows) { r"\\.\pipe\my-connector" } else { "/tmp/my-connector.sock" },
                                        value: "{custom_socket}",
                                        oninput: move |e: Event<FormData>| custom_socket.set(e.value()),
                                    }
                                    button { class: "custom-add-btn", r#type: "submit", "Add" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn SetupView(
    connectors: Vec<SetupConnector>,
    custom_connectors: Vec<CustomConnector>,
    on_add_custom: EventHandler<(String, String)>,
    on_remove_custom: EventHandler<String>,
    #[props(default)] on_select: Option<EventHandler<String>>,
    #[props(default)] on_hover: Option<EventHandler<Option<String>>>,
    #[props(default)] hovered_id: Option<String>,
    #[props(default = false)] dev_mode: bool,
) -> Element {
    rsx! {
        div { class: "setup-view",
            Strike48Logo { width: "180px" }
            h2 { "StrikeHub" }

            ConnectorCards {
                connectors: connectors,
                custom_connectors: custom_connectors,
                on_remove_custom: on_remove_custom,
                on_add_custom: on_add_custom,
                on_select: on_select,
                on_hover: on_hover,
                hovered_id: hovered_id,
                dev_mode: dev_mode,
            }
        }
    }
}
