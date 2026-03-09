use dioxus::prelude::*;

#[component]
pub fn AccountView(
    server_url: String,
    tenant_id: String,
    instance_id: String,
    on_sign_out: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "account-view",
            div { class: "account-card",
                // User avatar circle
                div { class: "account-avatar",
                    svg {
                        width: "32",
                        height: "32",
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

                h2 { class: "account-heading", "Account" }
                p { class: "account-status", "Signed In" }

                div { class: "account-details",
                    div { class: "account-detail-row",
                        span { class: "account-detail-label", "Server" }
                        span { class: "account-detail-value", "{server_url}" }
                    }
                    if !tenant_id.is_empty() {
                        div { class: "account-detail-row",
                            span { class: "account-detail-label", "Tenant" }
                            span { class: "account-detail-value", "{tenant_id}" }
                        }
                    }
                    if !instance_id.is_empty() {
                        div { class: "account-detail-row",
                            span { class: "account-detail-label", "Instance" }
                            span { class: "account-detail-value", "{instance_id}" }
                        }
                    }
                }

                button {
                    class: "account-sign-out-btn",
                    onclick: move |_| on_sign_out.call(()),
                    "Sign Out"
                }
            }
        }
    }
}
