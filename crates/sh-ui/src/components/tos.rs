use dioxus::prelude::*;

#[component]
pub fn PickTosOverlay(on_accept: EventHandler<()>, on_decline: EventHandler<()>) -> Element {
    rsx! {
        div { class: "tos-overlay",
            // Shield / warning icon
            svg {
                class: "tos-icon",
                width: "48",
                height: "48",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "1.5",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                // Shield path
                path { d: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" }
                // Exclamation mark
                line { x1: "12", y1: "8", x2: "12", y2: "12" }
                line { x1: "12", y1: "16", x2: "12.01", y2: "16" }
            }

            h1 { class: "tos-heading", "Pick \u{2014} Terms of Use" }

            div { class: "tos-body",
                p { class: "tos-text",
                    strong { "PICK IS FOR EDUCATIONAL USE ONLY." }
                    " You must have informed consent from the network and device owner before \
                     using Pick on a network or device. You may not attempt to bypass the \
                     Strike48 Services\u{2019} safety filters, training guardrails. Engaging in, \
                     promoting, or facilitating any activity that violates applicable local, \
                     state, national, or international laws or regulations is prohibited. \
                     Misuse of Pick may violate applicable criminal law, including wiretapping \
                     and computer abuse laws. Violations of these restrictions shall terminate \
                     your rights to use the Service automatically and immediately. You agree to \
                     indemnify Strike48 for any claim arising from your violation of these \
                     restrictions. Strike48 reserves the right to report suspected or actual \
                     illegal activities to law enforcement authorities. Your use of these \
                     Services are also subject to "
                    a {
                        class: "tos-link",
                        href: "https://strike48.com/terms",
                        target: "_blank",
                        "Strike48\u{2019}s Terms of Use"
                    }
                    "."
                }
            }

            div { class: "tos-buttons",
                button {
                    class: "tos-btn-decline",
                    onclick: move |_| on_decline.call(()),
                    "Decline"
                }
                button {
                    class: "tos-btn-accept",
                    onclick: move |_| on_accept.call(()),
                    "Accept"
                }
            }
        }
    }
}
