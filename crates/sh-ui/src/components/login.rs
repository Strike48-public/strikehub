use dioxus::prelude::*;
use sh_core::AuthManager;

use super::logo::Strike48Logo;

/// Sign-in overlay with an optional custom Studio URL input.
///
/// The main "Sign In" button uses the env/packaged URL (empty string signals
/// "use default"). Clicking "Custom URL sign in..." reveals an input that
/// defaults to the compiled-in constant [`AuthManager::DEFAULT_API_URL`],
/// or the previously saved URL if one exists.
#[component]
pub fn LoginOverlay(
    on_sign_in: EventHandler<String>,
    #[props(default = false)] signing_in: bool,
    /// Previously saved custom URL (from config). Pre-fills the URL input
    /// when the user clicks "Custom URL sign in...". The link is always
    /// shown first — the input only appears after clicking.
    #[props(default)]
    saved_studio_url: Option<String>,
    /// Error message to display (e.g. invalid URL, auth failure).
    #[props(default)]
    error_message: Option<String>,
) -> Element {
    let mut custom_url = use_signal(move || {
        saved_studio_url
            .clone()
            .unwrap_or_else(|| AuthManager::DEFAULT_API_URL.to_string())
    });
    let mut show_custom_url = use_signal(|| false);

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

    let url_val = custom_url.read().clone();
    let custom_visible = *show_custom_url.read();

    rsx! {
        div { class: "login-overlay",
            Strike48Logo { width: "180px" }

            h1 { class: "login-title", "StrikeHub" }

            if let Some(ref msg) = error_message {
                p { class: "login-error", role: "alert", "{msg}" }
            }

            button {
                class: "{btn_class}",
                disabled: signing_in,
                onclick: move |_| {
                    if !signing_in {
                        if *show_custom_url.peek() {
                            on_sign_in.call(custom_url.read().clone());
                        } else {
                            on_sign_in.call(String::new());
                        }
                    }
                },
                "{btn_label}"
            }

            if custom_visible {
                div { class: "login-url-group",
                    label { class: "login-url-label", r#for: "login-studio-url", "Studio URL" }
                    input {
                        id: "login-studio-url",
                        class: "login-url-input",
                        r#type: "text",
                        placeholder: "{AuthManager::DEFAULT_API_URL}",
                        value: "{url_val}",
                        disabled: signing_in,
                        oninput: move |e| {
                            custom_url.set(e.value().clone());
                        },
                    }
                }
            } else {
                a {
                    class: "login-custom-url-link",
                    href: "#",
                    onclick: move |e| {
                        e.prevent_default();
                        show_custom_url.set(true);
                    },
                    "Custom URL sign in\u{2026}"
                }
            }
        }
    }
}
