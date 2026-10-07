// Prevent a console window from appearing on Windows.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(feature = "desktop")]
fn main() {
    // Initialize Sentry before tracing so panics are captured.
    #[cfg(feature = "sentry")]
    let sentry_guard = sh_core::sentry_init::init_sentry(sh_core::sentry_init::AppMode::Desktop);

    // Set up file logging so diagnostics are available even when there is no
    // console (Windows GUI).  Logs are written to:
    //   Windows: %LOCALAPPDATA%\StrikeHub\logs\
    //   macOS:   ~/Library/Application Support/StrikeHub/logs/
    //   Linux:   ~/.local/share/StrikeHub/logs/
    let log_dir = sh_core::log_dir().expect("could not determine local app-data directory");

    let file_appender = tracing_appender::rolling::daily(&log_dir, "strikehub.log");

    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

    // Build the tracing registry with optional Sentry layer
    #[cfg(feature = "sentry")]
    let registry = tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        // File logs must stay clean for grep/tail/grep -c: no ANSI escapes.
        .with(fmt::layer().with_ansi(false).with_writer(file_appender))
        // The console layer keeps its default (colored) output.
        .with(fmt::layer().with_writer(std::io::stderr))
        // The layer's default event_filter already maps only error! records to
        // Sentry events (warn!/info! become breadcrumbs only), and
        // span_filter keeps Dioxus/library spans out. If we ever need to
        // capture warn-level events as Sentry events, add `.event_filter(...)`
        // here alongside the span allow-list. Whatever does get captured is
        // additionally deduped by message in sh_core::sentry_init::before_send
        // (issue #71).
        .with(sentry_tracing::layer().span_filter(sh_core::sentry_init::instrumented_spans_only));

    #[cfg(not(feature = "sentry"))]
    let registry = tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        // File logs must stay clean for grep/tail: no ANSI escapes.
        .with(fmt::layer().with_ansi(false).with_writer(file_appender))
        // The console layer keeps its default (colored) output.
        .with(fmt::layer().with_writer(std::io::stderr));

    registry.init();

    tracing::info!("StrikeHub starting — logs at {}", log_dir.display());

    // Windows: pre-emptively probe for the WebView2 host prerequisite so a
    // machine without the runtime OR Edge (e.g. a clean Windows 10 VM) gets
    // an actionable log line BEFORE the first window fails to render. The
    // loader itself is statically linked into this binary; only the runtime
    // is external. Check-and-warn: we still attempt to launch, because Edge
    // in unusual locations is possible and wry's own error stays authoritative.
    #[cfg(target_os = "windows")]
    {
        if let sh_core::webview2::WebView2State::Missing = sh_core::webview2::probe() {
            tracing::error!(
                "WebView2 runtime NOT found (no WebView2 Runtime and no Microsoft Edge \
                 installation): the StrikeHub window will not render. Install the \
                 WebView2 Evergreen Runtime from \
                 https://developer.microsoft.com/en-us/microsoft-edge/webview2/ and \
                 relaunch StrikeHub. The VC++ runtime is NOT the issue — it is \
                 bundled app-locally with this install."
            );
        }
    }

    // Leave the launching session's process group (Unix) BEFORE anything is
    // spawned: when launched from a desktop launcher (GNOME app grid, dock,
    // ...) the process starts in the LAUNCHER's process group —
    // project-management#380 (377 finding 7), HIGH. Since #115 every
    // connector is additionally spawned in its OWN process group (see
    // sh_core::process::spawn_tracked) and teardown signals those tracked
    // groups on close, so the launcher's group is never signalled — but we
    // still detach so the hub's own group stays private (terminal SIGINT
    // delivery, defensive killpg paths).
    #[cfg(unix)]
    sh_core::detach_process_group();

    // Install a Ctrl+C / SIGTERM / SIGTRAP handler so the process shuts
    // down cleanly. Since #115 the handler SIGTERMs every tracked child
    // process group (see sh_core::process::signal_kill_tracked_groups),
    // waits ~2 s, then SIGKILLs survivors — children are no longer reached
    // via the hub's own group because each connector leads its own group.
    // On all platforms, kill_on_drop(true) on the tokio::process::Child
    // handles provides a further safety net when a Child handle is dropped.
    install_signal_handler();

    // #115 RC-remainder QUIT-ROUTE WIRING: every GUI exit route funnels
    // through std::process::exit — dioxus 0.6.3's launch() blocks in tao's
    // EventLoop::run, and tao 0.30.8's run() is DIVERGING on all three
    // platforms: it returns from the OS run loop (last-window-close
    // ControlFlow::Exit, OR macOS applicationShouldTerminate — the
    // application-terminate event fired by `osascript quit` / Cmd-Q / dock
    // Quit) and then calls std::process::exit directly. That skips BOTH
    // the explicit teardown after launch() below AND every Rust Drop impl
    // (ProcessTreeGuard, kill_on_drop) — the exact route the RC
    // verification REPRO orphaned a self-updated pentest-agent on. The
    // C-level atexit hook is what delivers teardown_process_tree() there
    // (process::exit runs atexit handlers; Rust Drops never run for it).
    // Idempotent with the explicit call below and the signal-handler path.
    #[cfg(unix)]
    sh_core::process::install_exit_handler();

    // Exit fallback (#115): if we ever leave main without the explicit
    // teardown below (panic, early return), the guard drops and tears the
    // spawned tree down. Idempotent with the explicit call.
    let _process_tree_guard = sh_core::process::ProcessTreeGuard::new();

    // Extract bundled connector binaries (Windows: next to exe; other: no-op).
    sh_core::embedded::extract_bundled_binaries();
    // "Newest wins": if the connectors bundled with THIS install are newer than
    // whatever is in the per-user cache (~/.strike48/strikehub/bin), refresh the
    // cache from the bundle now — before anything resolves or launches a
    // connector. Fixes installing fresh over an old StrikeHub running the old
    // connector. No-op on dev builds (bundle timestamp is epoch-0).
    sh_core::seed_bundled_connectors(std::env::current_exe().ok().as_deref());

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
                .with_inner_size(dioxus::desktop::LogicalSize::new(
                    sh_ui::window_fit::DEFAULT_WIDTH,
                    sh_ui::window_fit::DEFAULT_HEIGHT,
                ))
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

    // Normal close (last window closed / quit): tear down the ENTIRE
    // spawned process tree before we exit — every tracked connector
    // process group (SIGTERM → ~2 s grace → SIGKILL) plus the descendant
    // walk for anything that escaped its group via setsid, plus the
    // managed-root fallback sweep for self-update-respawned processes
    // outside the registry.
    //
    // NOTE: in dioxus 0.6.3 this line is normally NOT the teardown that
    // fires — tao's EventLoop::run ends in std::process::exit (see the
    // QUIT-ROUTE WIRING comment above), and the atexit hook performs the
    // teardown instead. This call covers any exit route that DOES return
    // through main, and it is idempotent with the atexit pass. On Linux a
    // WebKitGTK window close can kill the hub with SIGTRAP before any
    // destructor runs — the SIGTRAP handler covers that death. (Windows:
    // TerminateJobObject + the KILL_ON_JOB_CLOSE guarantee, same call
    // site.)
    sh_core::process::teardown_process_tree();

    // Window close: end the Release Health session and flush pending data
    // (including the final session update) before the guard is dropped. A
    // killed process never gets this far, which is why explicit end-and-
    // flush on the graceful path matters for release health.
    #[cfg(feature = "sentry")]
    sh_core::sentry_init::shutdown_sentry(sentry_guard.as_ref(), std::time::Duration::from_secs(5));
}

#[cfg(feature = "desktop")]
fn install_signal_handler() {
    // On Unix, send SIGTERM to every tracked child process group, wait ~2 s,
    // SIGKILL survivors, then exit. This covers Ctrl+C, external SIGTERM,
    // and (issue #115) the SIGTRAP death a WebKitGTK window close causes on
    // Linux — all paths where Dioxus/tokio never get to run Drop impls.
    #[cfg(unix)]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

        // SAFETY: signal handler only calls async-signal-safe functions
        // (kill, sleep, _exit) plus the bounded try_lock in
        // sh_core::process::signal_kill_tracked_groups (no allocation on
        // the hot path).
        unsafe {
            libc::signal(
                libc::SIGINT,
                signal_handler as *const () as libc::sighandler_t,
            );
            libc::signal(
                libc::SIGTERM,
                signal_handler as *const () as libc::sighandler_t,
            );
            // #115: on Linux (WebKitGTK) a NORMAL window close kills the
            // hub with an intentional SIGTRAP after the webview teardown —
            // a signal death runs no destructors, so kill_on_drop never
            // fired and the connectors were orphaned (issue #115 Linux
            // repro, exit 133). Route that death through the same group
            // teardown as SIGINT/SIGTERM. (If the webview stack installs
            // its own SIGTRAP handler after us, it wins and this is a
            // no-op.)
            libc::signal(
                libc::SIGTRAP,
                signal_handler as *const () as libc::sighandler_t,
            );
        }

        extern "C" fn signal_handler(sig: libc::c_int) {
            // Guard against re-entry if a second signal arrives while
            // we're shutting down.
            if SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
                unsafe { libc::_exit(128 + sig) };
            }
            unsafe {
                // Since #115 every connector is spawned in its OWN process
                // group (sh_core::process::spawn_tracked) and registered in
                // the process-tree registry, and the hub detached from the
                // launcher's group at startup (#110) — so the hub's own
                // group now contains only the hub itself. We therefore do
                // NOT killpg(our own group) here: that would signal
                // ourselves and race the escalation below, and signalling
                // an inherited (launcher's) group is exactly the #110
                // hazard. The tracked groups below reach every child.
                sh_core::process::signal_kill_tracked_groups(libc::SIGTERM);
                // Grace: ~2 s for connectors (and their own children) to
                // exit. sleep() is async-signal-safe per POSIX.
                libc::sleep(sh_core::process::TEARDOWN_GRACE_SECS as libc::c_uint);
                // Escalation: SIGKILL any tracked group still alive.
                sh_core::process::signal_kill_tracked_groups(libc::SIGKILL);
                libc::_exit(128 + sig);
            }
        }
    }

    // On Windows, the default Ctrl+C behaviour terminates the process.
    // The Job Object (KILL_ON_JOB_CLOSE, see sh_core::job) kills every
    // connector tree when the hub's process handle closes on ANY exit
    // path, and kill_on_drop(true) on the tokio Child handles is the
    // direct-child backstop when the runtime tears down.
}

#[cfg(not(feature = "desktop"))]
fn main() {
    panic!("This binary requires the 'desktop' feature. Use 'cargo run --features desktop'.");
}
