use dioxus::prelude::*;
use sh_core::{AggregatePreflightResult, CheckStatus, PreflightCheck, PreflightResult};

/// Wizard step in the preflight flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WizardStep {
    /// Step 1: local device posture (Docker, kubectl, etc.)
    DevicePosture,
    /// Step 2: connector registration with StrikeHub / Strike48
    Registration,
}

#[component]
pub fn PreflightOverlay(
    result: AggregatePreflightResult,
    on_recheck: EventHandler<()>,
    on_continue: EventHandler<()>,
    #[props(default = false)] checking: bool,
) -> Element {
    // Split results: device-posture groups vs registration groups (prefixed "reg-").
    let device_groups: Vec<PreflightResult> = result
        .results
        .iter()
        .filter(|r| !r.connector_id.starts_with("reg-"))
        .cloned()
        .collect();
    let reg_groups: Vec<PreflightResult> = result
        .results
        .iter()
        .filter(|r| r.connector_id.starts_with("reg-"))
        .cloned()
        .collect();

    // When there are no device-posture groups (e.g. on Windows), skip straight
    // to the registration step.
    let initial_step = if device_groups.is_empty() {
        WizardStep::Registration
    } else {
        WizardStep::DevicePosture
    };
    let mut step = use_signal(move || initial_step);
    // Tracks groups whose collapsed state has been manually toggled by the user.
    let mut toggled: Signal<Vec<String>> = use_signal(Vec::new);

    let current_step = *step.read();

    let device_all_passed = device_groups.iter().all(|g| g.all_passed());
    let reg_all_passed = !reg_groups.is_empty() && reg_groups.iter().all(|g| g.all_passed());
    let has_reg = !reg_groups.is_empty();
    let has_device = !device_groups.is_empty();
    let all_passed = device_all_passed && reg_all_passed;

    let is_device_step = current_step == WizardStep::DevicePosture;
    let is_reg_step = current_step == WizardStep::Registration;

    // Per-step checking state: step 1 shows spinner when checking and no device results yet,
    // step 2 shows spinner when checking and no reg results yet.
    let device_checking = checking && !has_device;
    let reg_checking = checking && !has_reg;

    // Auto-poll every 5s on the registration step until all pass.
    use_effect(move || {
        let on_reg = *step.read() == WizardStep::Registration;
        if on_reg {
            let on_recheck = on_recheck;
            spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    if *step.peek() != WizardStep::Registration {
                        break;
                    }
                    on_recheck.call(());
                }
            });
        }
    });

    // Auto-continue when all registration checks pass.
    let mut auto_continued = use_signal(|| false);
    if *step.read() == WizardStep::Registration && reg_all_passed && !*auto_continued.peek() {
        auto_continued.set(true);
        spawn(async move {
            on_continue.call(());
        });
    }

    // Step indicator
    let step_num: u8 = if is_device_step { 1 } else { 2 };

    let step_label: &str = if is_device_step {
        "Device Posture"
    } else {
        "Connector Registration"
    };

    let pill1_class = if is_device_step {
        "step-pill active"
    } else if device_all_passed {
        "step-pill done"
    } else {
        "step-pill"
    };

    let pill2_class = if is_reg_step {
        "step-pill active"
    } else if reg_all_passed {
        "step-pill done"
    } else {
        "step-pill"
    };

    rsx! {
        div { class: "preflight-overlay",
            // Fixed header
            div { class: "preflight-header",
                svg {
                    class: "preflight-icon",
                    width: "36",
                    height: "36",
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: "1.5",
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    path { d: "M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" }
                    rect { x: "8", y: "2", width: "8", height: "4", rx: "1", ry: "1" }
                }
                div { class: "preflight-header-text",
                    h1 { class: "preflight-heading", "Preflight Check" }
                    p { class: "preflight-step-label", "Step {step_num} of 2: {step_label}" }
                }

                // Step pills
                div { class: "preflight-steps",
                    button {
                        class: pill1_class,
                        onclick: move |_| step.set(WizardStep::DevicePosture),
                        "1"
                    }
                    div { class: "step-connector" }
                    button {
                        class: pill2_class,
                        onclick: move |_| step.set(WizardStep::Registration),
                        "2"
                    }
                }
            }

            // Step 1: Device Posture
            if is_device_step {
                div { class: "preflight-scroll",
                    div { class: "preflight-body",
                        p { class: "preflight-intro",
                            "Verifying system requirements are met."
                        }
                        if device_checking {
                            div { class: "preflight-step-spinner",
                                span { class: "preflight-spinner" }
                                "Checking system requirements\u{2026}"
                            }
                        }
                        for group in device_groups.iter() {
                            {
                                let gid = group.connector_id.clone();
                                let group_passed = group.all_passed();
                                let was_toggled = toggled.read().contains(&gid);
                                // Default: collapsed if passed, expanded if failed. Toggle inverts.
                                let is_collapsed = if was_toggled { !group_passed } else { group_passed };
                                rsx! {
                                    PreflightGroup {
                                        group: group.clone(),
                                        collapsed: is_collapsed,
                                        on_toggle: {
                                            let gid = gid.clone();
                                            move |_: ()| {
                                                let gid = gid.clone();
                                                let mut t = toggled.write();
                                                if let Some(pos) = t.iter().position(|x| *x == gid) {
                                                    t.remove(pos);
                                                } else {
                                                    t.push(gid);
                                                }
                                            }
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "preflight-footer",
                    div { class: "preflight-buttons",
                        button {
                            class: "preflight-btn-recheck",
                            disabled: checking,
                            onclick: move |_| on_recheck.call(()),
                            if checking { "Checking\u{2026}" } else { "Re-check" }
                        }
                        button {
                            class: if device_all_passed { "preflight-btn-continue" } else { "preflight-btn-skip" },
                            onclick: move |_| {
                                step.set(WizardStep::Registration);
                                on_recheck.call(());
                            },
                            if device_all_passed { "Next" } else { "Skip" }
                        }
                    }
                }
            }

            // Step 2: Registration
            if is_reg_step {
                div { class: "preflight-scroll",
                    div { class: "preflight-body",
                        p { class: "preflight-intro",
                            "Verifying connectors are running and registered with Strike48."
                        }
                        if reg_checking {
                            div { class: "preflight-step-spinner",
                                span { class: "preflight-spinner" }
                                "Waiting for connectors to register\u{2026}"
                            }
                        }
                        if has_reg {
                            for rg in reg_groups.iter() {
                                {
                                    let gid = rg.connector_id.clone();
                                    let group_passed = rg.all_passed();
                                    let was_toggled = toggled.read().contains(&gid);
                                    let is_collapsed = if was_toggled { !group_passed } else { group_passed };
                                    rsx! {
                                        PreflightGroup {
                                            group: rg.clone(),
                                            collapsed: is_collapsed,
                                            on_toggle: {
                                                let gid = gid.clone();
                                                move |_: ()| {
                                                    let gid = gid.clone();
                                                    let mut t = toggled.write();
                                                    if let Some(pos) = t.iter().position(|x| *x == gid) {
                                                        t.remove(pos);
                                                    } else {
                                                        t.push(gid);
                                                    }
                                                }
                                            },
                                        }
                                    }
                                }
                            }
                        }

                        // Only show troubleshooting hint if checks are failing
                        if has_reg && !reg_all_passed {
                            div { class: "preflight-hint-box",
                                p { class: "preflight-hint-title", "Not seeing your connectors?" }
                                ol { class: "preflight-hint-steps",
                                    li { "Go to the ",
                                        strong { "Gateways" }
                                        " page in Strike48 Studio"
                                    }
                                    li { "Approve any pending connector registrations" }
                                    li { "Click ",
                                        strong { "Re-check" }
                                        " below to refresh the status"
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "preflight-footer",
                    if checking {
                        div { class: "preflight-poll-status",
                            span { class: "preflight-spinner" }
                            "Polling\u{2026}"
                        }
                    }
                    div { class: "preflight-buttons",
                        button {
                            class: "preflight-btn-recheck",
                            disabled: checking,
                            onclick: move |_| on_recheck.call(()),
                            "Re-check"
                        }
                        button {
                            class: "preflight-btn-continue",
                            onclick: move |_| on_continue.call(()),
                            if all_passed { "Continue" } else { "Next" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn PreflightGroup(group: PreflightResult, collapsed: bool, on_toggle: EventHandler<()>) -> Element {
    let check_count = group.checks.len();
    let pass_count = group
        .checks
        .iter()
        .filter(|c| c.status == CheckStatus::Passed)
        .count();
    let summary = format!("{}/{} passed", pass_count, check_count);
    let summary_class = if pass_count == check_count {
        "group-summary passed"
    } else {
        "group-summary"
    };

    rsx! {
        div { class: "preflight-group",
            div {
                class: "preflight-group-header",
                onclick: move |_| on_toggle.call(()),
                span { class: "preflight-group-chevron",
                    if collapsed { "\u{25b6}" } else { "\u{25bc}" }
                }
                h3 { class: "preflight-group-name", "{group.connector_name}" }
                span { class: summary_class, "{summary}" }
            }
            if !collapsed {
                div { class: "preflight-group-body",
                    for check in group.checks.iter() {
                        PreflightCheckItem { check: check.clone() }
                    }
                }
            }
        }
    }
}

#[component]
fn PreflightCheckItem(check: PreflightCheck) -> Element {
    let (status_class, status_icon) = match check.status {
        CheckStatus::Checking => ("checking", "\u{23f3}"),
        CheckStatus::Passed => ("passed", "\u{2714}"),
        CheckStatus::Failed => ("failed", "\u{2718}"),
    };

    let mut installing = use_signal(|| false);
    let mut install_output = use_signal(|| Option::<String>::None);

    let has_install_cmd = check.install_command.is_some();
    let install_cmd = check.install_command.clone();

    rsx! {
        div { class: "preflight-check-item {status_class}",
            span { class: "preflight-check-status", "{status_icon}" }
            div { class: "preflight-check-content",
                div { class: "preflight-check-name", "{check.name}" }
                div { class: "preflight-check-desc", "{check.description}" }
                if check.status == CheckStatus::Failed && !check.install_hint.is_empty() {
                    pre { class: "preflight-install-hint", "{check.install_hint}" }
                }
                if check.status == CheckStatus::Failed && has_install_cmd {
                    div { class: "preflight-install-action",
                        button {
                            class: "preflight-btn-install",
                            disabled: *installing.read(),
                            onclick: move |_| {
                                if let Some(ref cmd) = install_cmd {
                                    let cmd = cmd.clone();
                                    installing.set(true);
                                    install_output.set(None);
                                    spawn(async move {
                                        let result = run_install_command(&cmd).await;
                                        install_output.set(Some(result));
                                        installing.set(false);
                                    });
                                }
                            },
                            if *installing.read() {
                                span { class: "preflight-spinner" }
                                "Installing\u{2026}"
                            } else {
                                "Install"
                            }
                        }
                    }
                }
                if let Some(ref output) = *install_output.read() {
                    pre { class: "preflight-install-output", "{output}" }
                }
            }
        }
    }
}

/// Run an install command in a background thread and return the output.
async fn run_install_command(command: &str) -> String {
    let cmd = command.to_string();
    let result = tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            std::process::Command::new("powershell")
                .args(["-NoProfile", "-Command", &cmd])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
        }
        #[cfg(not(target_os = "windows"))]
        {
            std::process::Command::new("sh").args(["-c", &cmd]).output()
        }
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{}{}", stdout, stderr);
            let lower = combined.to_lowercase();

            // Treat "already installed" / "successfully installed" as success
            // even if the exit code is non-zero (winget does this).
            if output.status.success()
                || lower.contains("successfully installed")
                || lower.contains("already installed")
            {
                let msg = combined.trim().to_string();
                if msg.is_empty() {
                    "Installed successfully.".into()
                } else {
                    msg
                }
            } else {
                format!("Install failed:\n{}", combined.trim())
            }
        }
        Ok(Err(e)) => format!("Failed to run command: {}", e),
        Err(e) => format!("Failed to run command: {}", e),
    }
}
