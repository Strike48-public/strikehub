//! End-to-end pin for issue #62's Unix file routing (review item 4a): a
//! child spawned through the real [`sh_core::IpcConnectorRunner::start`]
//! path must have its stdout land in `connector-<id>.log` in StrikeHub's
//! log directory — not an inherited terminal. Reverting the Unix branch of
//! `start()` to `Stdio::inherit()` (the pre-#62 behavior) fails this test.
//!
//! This lives in its own test target (its own process) so it can point
//! `XDG_DATA_HOME` at a private temp dir — `dirs::data_local_dir()` honours
//! it on Linux — without clobbering that variable for the parallel unit
//! tests in the lib target.
//!
//! The test takes ~15 s: the fake connector dies immediately, and
//! `start()` only returns once its 15 s readiness deadline passes. That is
//! the price of driving the real spawn path.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Private `XDG_DATA_HOME` for this test process; set exactly once so
/// `sh_core::log_dir()` (`dirs::data_local_dir()/StrikeHub/logs`) resolves
/// into a temp dir the test owns.
fn private_data_home() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("strikehub-log-route-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Process-private: this target runs in its own test binary, and the
        // OnceLock guarantees the write happens exactly once — no other
        // reader of XDG_DATA_HOME races it (SAFETY for the unsafe set_var).
        unsafe { std::env::set_var("XDG_DATA_HOME", &dir) };
        dir
    })
}

#[tokio::test]
async fn start_routes_child_output_to_the_connector_log_file() {
    let dir = private_data_home();

    // A fake connector that writes one line of "activity", then crashes.
    let script = dir.join("fake-connector");
    std::fs::write(&script, "#!/bin/sh\necho 'downloading 42%'\nexit 3\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let id = "pick-log-route";
    let runner = sh_core::IpcConnectorRunner::start(id, &script, &[])
        .await
        .unwrap();
    drop(runner);

    let log = sh_core::log_dir()
        .expect("log dir")
        .join(format!("connector-{id}.log"));
    let body = std::fs::read_to_string(&log)
        .unwrap_or_else(|e| panic!("connector log file missing at {}: {e}", log.display()));
    assert!(
        body.contains("downloading 42%"),
        "the child's stdout must land in the per-connector log file, got: {body:?}"
    );
}
