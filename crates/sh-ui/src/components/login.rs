use dioxus::prelude::*;

use super::logo::Strike48Logo;

#[component]
pub fn LoginOverlay(
    on_sign_in: EventHandler<()>,
    #[props(default = false)] signing_in: bool,
) -> Element {
    let btn_class = if signing_in {
        "login-btn disabled"
    } else {
        "login-btn"
    };
    let btn_label = if signing_in {
        "Signing in\u{2026}"
    } else {
        "Sign In"
    };

    rsx! {
        div { class: "login-overlay",
            Strike48Logo { width: "180px" }

            h1 { class: "login-title", "StrikeHub" }

            button {
                class: "{btn_class}",
                disabled: signing_in,
                onclick: move |_| {
                    if !signing_in {
                        on_sign_in.call(());
                    }
                },
                "{btn_label}"
            }
        }
    }
}
