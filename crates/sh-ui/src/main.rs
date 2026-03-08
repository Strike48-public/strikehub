// Prevent a console window from appearing on Windows.
// TODO: uncomment before release — commented out for debugging
// #![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(feature = "desktop")]
fn main() {
    tracing_subscriber::fmt::init();

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

    #[allow(unused_mut)]
    let mut config = dioxus::desktop::Config::new().with_window(
        dioxus::desktop::WindowBuilder::new()
            .with_title("StrikeHub")
            .with_window_icon(Some(icon))
            .with_always_on_top(false)
            .with_inner_size(dioxus::desktop::LogicalSize::new(1024.0, 768.0))
            .with_min_inner_size(dioxus::desktop::LogicalSize::new(800.0, 600.0)),
    );

    // Remove the default File/Edit/Help menu bar on Windows.
    #[cfg(target_os = "windows")]
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

#[cfg(not(feature = "desktop"))]
fn main() {
    panic!("This binary requires the 'desktop' feature. Use 'cargo run --features desktop'.");
}
