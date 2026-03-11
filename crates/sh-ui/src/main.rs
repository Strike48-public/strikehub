// Prevent a console window from appearing on Windows.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(feature = "desktop")]
fn main() {
    // Set up file logging so diagnostics are available even when there is no
    // console (Windows GUI).  Logs are written to:
    //   Windows: %LOCALAPPDATA%\StrikeHub\logs\
    //   macOS:   ~/Library/Application Support/StrikeHub/logs/
    //   Linux:   ~/.local/share/strikehub/logs/
    let log_dir = dirs::data_local_dir()
        .expect("could not determine local app-data directory")
        .join("StrikeHub")
        .join("logs");

    let file_appender = tracing_appender::rolling::daily(&log_dir, "strikehub.log");

    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with(fmt::layer().with_writer(file_appender))
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();

    tracing::info!("StrikeHub starting — logs at {}", log_dir.display());

    // Install a Ctrl+C / SIGTERM handler so the process shuts down cleanly.
    // On Unix this sends SIGTERM to our entire process group, which kills any
    // child connector processes that are still in our group. On all platforms,
    // kill_on_drop(true) on the tokio::process::Child handles provides a
    // secondary safety net when the Child handle is dropped.
    install_signal_handler();

    // Extract bundled connector binaries (Windows: next to exe; other: no-op).
    sh_core::embedded::extract_bundled_binaries();

    // Create shared bridge state before launching Dioxus so the custom
    // protocol handler can reference it from day one.
    let bridge_state = sh_core::new_bridge_state();
    sh_ui::set_bridge_state(bridge_state);

    let icon = dioxus::desktop::tao::window::Icon::from_rgba(
        include_bytes!("../../../assets/icon_256x256.rgba").to_vec(),
        256,
        256,
    )
    .expect("failed to load window icon");

    // Place the WebView2 data directory in a user-writable location so the
    // app works when installed under C:\Program Files (which is read-only for
    // normal users).
    let data_dir = dirs::data_local_dir()
        .expect("could not determine local app-data directory")
        .join("StrikeHub");

    #[allow(unused_mut)]
    let mut config = dioxus::desktop::Config::new()
        .with_data_directory(data_dir)
        .with_window(
            dioxus::desktop::WindowBuilder::new()
                .with_title("StrikeHub")
                .with_window_icon(Some(icon))
                .with_always_on_top(false)
                .with_inner_size(dioxus::desktop::LogicalSize::new(1024.0, 768.0))
                .with_min_inner_size(dioxus::desktop::LogicalSize::new(800.0, 600.0)),
        );

    // Remove the default File/Edit/Help menu bar on Windows and Linux.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        config = config.with_menu(None::<dioxus::desktop::muda::Menu>);
    }

    dioxus::LaunchBuilder::desktop()
        .with_cfg(config.with_asynchronous_custom_protocol(
            "connector",
            move |request, responder| {
                // Spawn into the tokio runtime so we can do async I/O
                // (Unix socket HTTP calls to the connector process).
                let uri = request.uri().to_string();
                tokio::spawn(async move {
                    let Some(state) = sh_ui::get_bridge_state() else {
                        let resp = http::Response::builder()
                            .status(500)
                            .body(Vec::from("bridge state not initialised"))
                            .unwrap();
                        responder.respond(resp);
                        return;
                    };

                    let (status, headers, body) =
                        sh_core::bridge::handle_bridge_request(state, &uri).await;

                    let mut builder = http::Response::builder().status(status);
                    for (k, v) in &headers {
                        builder = builder.header(k.as_str(), v.as_str());
                    }
                    let resp = builder.body(body).unwrap();
                    responder.respond(resp);
                });
            },
        ))
        .launch(sh_ui::App);
}

#[cfg(feature = "desktop")]
fn install_signal_handler() {
    // On Unix, send SIGTERM to our process group so all child connector
    // processes are terminated, then exit. This covers Ctrl+C and external
    // SIGTERM delivery where Dioxus might not get a chance to run Drop impls.
    #[cfg(unix)]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

        // SAFETY: signal handler only calls async-signal-safe functions
        // (getpgrp, killpg, _exit).
        unsafe {
            libc::signal(
                libc::SIGINT,
                signal_handler as *const () as libc::sighandler_t,
            );
            libc::signal(
                libc::SIGTERM,
                signal_handler as *const () as libc::sighandler_t,
            );
        }

        extern "C" fn signal_handler(sig: libc::c_int) {
            // Guard against re-entry if a second signal arrives while we're
            // shutting down.
            if SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
                unsafe { libc::_exit(128 + sig) };
            }
            unsafe {
                // Kill every process in our process group. Child connectors
                // spawned without setsid() share our group, so they receive
                // SIGTERM and can exit cleanly.
                let pgrp = libc::getpgrp();
                libc::killpg(pgrp, libc::SIGTERM);
                // Give children a brief moment to exit, then force-exit.
                // sleep() is async-signal-safe per POSIX.
                libc::sleep(1);
                libc::_exit(128 + sig);
            }
        }
    }

    // On Windows, the default Ctrl+C behaviour terminates the process.
    // kill_on_drop(true) on the tokio Child handles ensures connector
    // processes are cleaned up when the runtime tears down.
}

#[cfg(not(feature = "desktop"))]
fn main() {
    panic!("This binary requires the 'desktop' feature. Use 'cargo run --features desktop'.");
}
