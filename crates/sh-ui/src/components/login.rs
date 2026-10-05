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
    /// Login URL of the in-flight OAuth flow (when `signing_in`), for the
    /// "Open sign-in page again" / "Copy sign-in link" waiting-state actions.
    /// `None` while the flow is still computing it.
    #[props(default)]
    sign_in_url: Option<String>,
    /// Cancel the in-flight OAuth flow (waiting state, #379).
    #[props(default)]
    on_cancel: EventHandler<()>,
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
    let mut cache_msg = use_signal(|| Option::<String>::None);

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

    // Waiting-state actions for the in-flight flow (#379): the login URL as
    // two owned copies, one per button closure (rsx event closures must be
    // 'static, so each closure takes ownership of its copy).
    let (waiting_url_open, waiting_url_copy) = match sign_in_url {
        Some(url) => (Some(url.clone()), Some(url)),
        None => (None, None),
    };

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

            // Waiting state for an in-flight OAuth flow: the browser is
            // doing the work, so make the app side actionable instead of a
            // dead disabled button — re-open the login page, copy the link,
            // or cancel (Strike48/project-management#379).
            if signing_in {
                div { class: "login-waiting",
                    p { class: "login-waiting-text",
                        "Complete sign-in in the browser window that opened. First run can take a few minutes."
                    }
                    div { class: "login-waiting-actions",
                        if let Some(url) = waiting_url_open {
                            button {
                                class: "login-waiting-btn",
                                onclick: move |_| {
                                    #[cfg(feature = "desktop")]
                                    {
                                        if let Err(e) = open::that(&url) {
                                            tracing::error!("Failed to open sign-in URL: {}", e);
                                        }
                                    }
                                    #[cfg(not(feature = "desktop"))]
                                    {
                                        let js = format!(
                                            "window.open('{}', '_blank')",
                                            sh_core::js_string_escape(&url)
                                        );
                                        let _ = document::eval(&js);
                                    }
                                },
                                "Open sign-in page again"
                            }
                        }
                        if let Some(url) = waiting_url_copy {
                            button {
                                class: "login-waiting-btn",
                                onclick: move |_| {
                                    let payload = serde_json::to_string(&url)
                                        .unwrap_or_else(|_| "null".to_string());
                                    let js = format!(
                                        "navigator.clipboard.writeText({payload}).catch(function(){{var t=document.createElement('textarea');t.value={payload};document.body.appendChild(t);t.select();document.execCommand('copy');t.remove();}})"
                                    );
                                    let _ = document::eval(&js);
                                },
                                "Copy sign-in link"
                            }
                        }
                        button {
                            class: "login-waiting-btn login-waiting-cancel",
                            onclick: move |_| {
                                on_cancel.call(());
                            },
                            "Cancel"
                        }
                    }
                }
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

            a {
                class: "login-clear-cache-link",
                href: "#",
                onclick: move |e| {
                    e.prevent_default();
                    let url = custom_url.read().clone();
                    let deleted = sh_core::ott::clear_credentials_for_url(&url);
                    if deleted.is_empty() {
                        cache_msg.set(Some(format!("No cached credentials for {}.", url)));
                    } else {
                        cache_msg.set(Some(format!("Cleared for {}.", url)));
                    }
                },
                "Clear cached credentials"
            }
            if let Some(msg) = &*cache_msg.read() {
                p { class: "login-cache-msg", "{msg}" }
            }

            p {
                class: "login-telemetry-notice",
                "By signing in, you agree to the collection of anonymous usage data to help improve this product."
            }
        }
    }
}
