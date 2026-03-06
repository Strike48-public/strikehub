use dioxus::prelude::*;
use sh_core::ConnectorStatus;

#[derive(Debug, Clone, PartialEq, Props)]
pub struct ConnectorItem {
    pub id: String,
    pub display_name: String,
    pub icon: String,
    pub status: ConnectorStatus,
}

fn status_css_class(status: ConnectorStatus) -> &'static str {
    match status {
        ConnectorStatus::Online => "online",
        ConnectorStatus::Offline => "offline",
        ConnectorStatus::Checking => "checking",
    }
}

/// Map an icon name to an SVG path (24x24 viewBox).
/// Supports both hero-* (Heroicons) and Lucide icon names.
pub fn hero_icon_path(icon: &str) -> &'static str {
    match icon {
        // Lucide "server" — single rack-mount server unit
        "hero-server-stack" | "server" => {
            "M2 9a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v2a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V9Zm0 7a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v2a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-2Zm6-6h.01M8 18h.01"
        }
        "hero-ticket" => {
            "M16.5 6v.75m0 3v.75m0 3v.75m0 3V18m-9-5.25h5.25M7.5 15h3M3.375 5.25c-.621 0-1.125.504-1.125 1.125v3.026a2.999 2.999 0 0 1 0 5.198v3.026c0 .621.504 1.125 1.125 1.125h17.25c.621 0 1.125-.504 1.125-1.125v-3.026a2.999 2.999 0 0 1 0-5.198V6.375c0-.621-.504-1.125-1.125-1.125H3.375Z"
        }
        "hero-code-bracket" => {
            "M17.25 6.75 22.5 12l-5.25 5.25m-10.5 0L1.5 12l5.25-5.25m7.5-3-4.5 16.5"
        }
        "hero-user-group" => {
            "M18 18.72a9.094 9.094 0 0 0 3.741-.479 3 3 0 0 0-4.682-2.72m.94 3.198.001.031c0 .225-.012.447-.037.666A11.944 11.944 0 0 1 12 21c-2.17 0-4.207-.576-5.963-1.584A6.062 6.062 0 0 1 6 18.719m12 0a5.971 5.971 0 0 0-.941-3.197m0 0A5.995 5.995 0 0 0 12 12.75a5.995 5.995 0 0 0-5.058 2.772m0 0a3 3 0 0 0-4.681 2.72 8.986 8.986 0 0 0 3.74.477m.94-3.197a5.971 5.971 0 0 0-.94 3.197M15 6.75a3 3 0 1 1-6 0 3 3 0 0 1 6 0Zm6 3a2.25 2.25 0 1 1-4.5 0 2.25 2.25 0 0 1 4.5 0Zm-13.5 0a2.25 2.25 0 1 1-4.5 0 2.25 2.25 0 0 1 4.5 0Z"
        }
        "hero-document-text" => {
            "M19.5 14.25v-2.625a3.375 3.375 0 0 0-3.375-3.375h-1.5A1.125 1.125 0 0 1 13.5 7.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H8.25m0 12.75h7.5m-7.5 3H12M10.5 2.25H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 0 0-9-9Z"
        }
        "hero-shield-exclamation" => {
            "M12 9v3.75m0-10.036A11.959 11.959 0 0 1 3.598 6 11.99 11.99 0 0 0 3 9.75c0 5.592 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.31-.21-2.57-.598-3.75h-.152c-3.196 0-6.1-1.25-8.25-3.286Zm0 13.036h.008v.008H12v-.008Z"
        }
        // plug icon for custom connectors
        "hero-puzzle-piece" => {
            "M14.25 6.087c0-.355.186-.676.401-.959.221-.29.349-.634.349-1.003 0-1.036-1.007-1.875-2.25-1.875s-2.25.84-2.25 1.875c0 .369.128.713.349 1.003.215.283.401.604.401.959v0a.64.64 0 0 1-.657.643 48.39 48.39 0 0 1-4.163-.3c.186 1.613.293 3.25.315 4.907a.656.656 0 0 1-.658.663v0c-.355 0-.676-.186-.959-.401a1.647 1.647 0 0 0-1.003-.349c-1.036 0-1.875 1.007-1.875 2.25s.84 2.25 1.875 2.25c.369 0 .713-.128 1.003-.349.283-.215.604-.401.959-.401v0c.31 0 .555.26.532.57a48.039 48.039 0 0 1-.642 5.056c1.518.19 3.058.309 4.616.354a.64.64 0 0 0 .657-.643v0c0-.355-.186-.676-.401-.959a1.647 1.647 0 0 1-.349-1.003c0-1.035 1.008-1.875 2.25-1.875 1.243 0 2.25.84 2.25 1.875 0 .369-.128.713-.349 1.003-.215.283-.4.604-.4.959v0c0 .333.277.599.61.58a48.1 48.1 0 0 0 5.427-.63 48.05 48.05 0 0 0 .582-4.717.532.532 0 0 0-.533-.57v0c-.355 0-.676.186-.959.401-.29.221-.634.349-1.003.349-1.035 0-1.875-1.007-1.875-2.25s.84-2.25 1.875-2.25c.37 0 .713.128 1.003.349.283.215.604.401.96.401v0a.656.656 0 0 0 .657-.663 48.422 48.422 0 0 0-.37-5.36c-1.886.342-3.81.574-5.766.689a.578.578 0 0 1-.61-.58v0Z"
        }
        // fallback: generic app window
        _ => {
            "M2.25 7.125C2.25 6.504 2.754 6 3.375 6h6c.621 0 1.125.504 1.125 1.125v3.75c0 .621-.504 1.125-1.125 1.125h-6a1.125 1.125 0 0 1-1.125-1.125v-3.75ZM14.25 8.625c0-.621.504-1.125 1.125-1.125h2.25c.621 0 1.125.504 1.125 1.125v.75c0 .621-.504 1.125-1.125 1.125h-2.25a1.125 1.125 0 0 1-1.125-1.125v-.75Zm0 5.25c0-.621.504-1.125 1.125-1.125h2.25c.621 0 1.125.504 1.125 1.125v.75c0 .621-.504 1.125-1.125 1.125h-2.25a1.125 1.125 0 0 1-1.125-1.125v-.75Zm-12 2.25c0-.621.504-1.125 1.125-1.125h6c.621 0 1.125.504 1.125 1.125v2.25c0 .621-.504 1.125-1.125 1.125h-6a1.125 1.125 0 0 1-1.125-1.125v-2.25Z"
        }
    }
}

/// Render a connector icon as inline SVG from a hero-* icon name.
#[component]
fn ConnectorIcon(icon: String, #[props(default = 20)] size: u32) -> Element {
    let path_d = hero_icon_path(&icon);
    rsx! {
        svg {
            class: "connector-icon",
            width: "{size}",
            height: "{size}",
            view_box: "0 0 24 24",
            fill: "none",
            xmlns: "http://www.w3.org/2000/svg",
            path {
                d: "{path_d}",
                stroke: "currentColor",
                stroke_width: "1.5",
                stroke_linecap: "round",
                stroke_linejoin: "round",
            }
        }
    }
}

#[component]
pub fn Sidebar(
    connectors: Vec<ConnectorItem>,
    active_id: Option<String>,
    #[props(default = false)] show_settings: bool,
    on_select: EventHandler<String>,
    #[props(default)] on_hover: Option<EventHandler<Option<String>>>,
    #[props(default)] hovered_id: Option<String>,
    on_settings: EventHandler<()>,
    #[props(default = false)] is_signed_in: bool,
    #[props(default = false)] signing_in: bool,
    #[props(default = false)] has_matrix_url: bool,
    on_sign_in: EventHandler<()>,
    on_sign_out: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "sidebar-rail",
            // Strike48 "48" mark at top — opens website in system browser
            div {
                class: "rail-logo",
                title: "strike48.com",
                onclick: move |_| { let _ = open::that("https://www.strike48.com/"); },
                svg {
                    class: "rail-logo-svg",
                    width: "28",
                    height: "22",
                    view_box: "406 2 200 158",
                    xmlns: "http://www.w3.org/2000/svg",
                    // "4" numeral with pickaxe detail
                    path {
                        fill: "#efbf04",
                        d: "M492.12,44.69l4.57-6.49c7.69,5.08,14.58,10.38,20.57,15.44l6.96-5.35c-5.56-6.85-12.37-13.88-20.53-20.04l3.42-4.86-1.9-1.02,3.39-4.66-17.27-8.61-3.09,4.15-2.08-1.12-3.06,4.15c-31.41-13.52-63.83-9.27-63.83-9.27,0,0,26.65,4.3,56.53,19.17l-3.58,4.85,1.84,1.27-64.04,86.04v15.53h53.72v20.95h22.51v-20.95h16.24v-18.81h-16.24v-49.18l-21.8,31.2v17.98h-26.79l52.33-71.82,2.12,1.45Z",
                    }
                    // "8" numeral
                    path {
                        fill: "#efbf04",
                        d: "M586.73,102.38c7.41-4.56,11.68-11.54,11.68-20.52,0-17.53-15.67-28.5-38.62-28.5-9.8,0-18.29,2.06-24.77,5.8,3.26,4.77,7.09,10.74,10.94,17.58,2.38-4.1,7.32-6.43,13.83-6.43,9.41,0,15.68,4.7,15.68,12.54s-6.13,12.26-15.68,12.26-15.39-4.7-15.39-12.26c0-1.17.15-2.25.41-3.27-4.41-4.13-10.83-10.1-16.44-15.26-4.42,4.62-6.91,10.56-6.91,17.53,0,8.98,4.13,15.96,11.54,20.52-9.55,4.84-15.11,12.83-15.11,23.51,0,18.81,16.67,30.64,41.9,30.64s42.18-11.83,42.18-30.64c0-10.69-5.56-18.67-15.25-23.51ZM559.8,139.57c-11.4,0-18.53-5.56-18.53-14.39s7.13-14.25,18.53-14.25,18.81,5.42,18.81,14.25-7.27,14.39-18.81,14.39Z",
                    }
                }
            }

            // Separator
            div { class: "rail-separator" }

            // Connector icons
            div { class: "rail-connectors",
                {
                    let locked = has_matrix_url && !is_signed_in;
                    rsx! {
                        for connector in connectors.iter() {
                            {
                                let id = connector.id.clone();
                                let is_active = active_id.as_ref() == Some(&connector.id);
                                let is_hovered = hovered_id.as_ref() == Some(&connector.id);
                                let dot_class = if locked { "locked" } else { status_css_class(connector.status) };
                                let class = if locked {
                                    "rail-item locked"
                                } else if is_active {
                                    "rail-item active"
                                } else if is_hovered {
                                    "rail-item hovered"
                                } else {
                                    "rail-item"
                                };
                                let label = connector.display_name.clone();
                                let hover_id = connector.id.clone();
                                rsx! {
                                    div {
                                        class: "{class}",
                                        onclick: move |_| {
                                            if !locked {
                                                on_select.call(id.clone());
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
                                        title: "{label}",
                                        div { class: "rail-icon-wrapper",
                                            div { class: "rail-icon",
                                                ConnectorIcon { icon: connector.icon.clone(), size: 20 }
                                            }
                                            div { class: "rail-status-dot {dot_class}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Bottom actions
            div { class: "rail-footer",
                if has_matrix_url && is_signed_in {
                    div {
                        class: "rail-action signed-in",
                        onclick: move |_| on_sign_out.call(()),
                        title: "Sign Out",
                        svg {
                            width: "20",
                            height: "20",
                            view_box: "0 0 24 24",
                            fill: "none",
                            xmlns: "http://www.w3.org/2000/svg",
                            path {
                                d: "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2",
                                stroke: "currentColor",
                                stroke_width: "1.5",
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                            }
                            circle {
                                cx: "9",
                                cy: "7",
                                r: "4",
                                stroke: "currentColor",
                                stroke_width: "1.5",
                            }
                            path {
                                d: "M16 11l2 2 4-4",
                                stroke: "currentColor",
                                stroke_width: "1.5",
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                            }
                        }
                    }
                }
                div {
                    class: {
                        let locked = has_matrix_url && !is_signed_in;
                        if locked {
                            "rail-action rail-settings locked"
                        } else if show_settings {
                            "rail-action rail-settings active"
                        } else {
                            "rail-action rail-settings"
                        }
                    },
                    onclick: move |_| {
                        let locked = has_matrix_url && !is_signed_in;
                        if !locked {
                            on_settings.call(());
                        }
                    },
                    title: "Settings",
                    svg {
                        width: "20",
                        height: "20",
                        view_box: "0 0 24 24",
                        fill: "none",
                        xmlns: "http://www.w3.org/2000/svg",
                        path {
                            d: "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z",
                            stroke: "currentColor",
                            stroke_width: "1.5",
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                        }
                        circle {
                            cx: "12",
                            cy: "12",
                            r: "3",
                            stroke: "currentColor",
                            stroke_width: "1.5",
                        }
                    }
                }
            }
        }
    }
}
