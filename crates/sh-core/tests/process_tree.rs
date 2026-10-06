//! Issue #115 — whole-tree teardown on close (Unix).
//!
//! Spawns fixture process trees through the REAL spawn + registry API
//! (`sh_core::process::spawn_tracked`) and asserts the teardown routines
//! leave ZERO survivors:
//!
//! * grandchildren in the child's process group (the group kill must reach
//!   them),
//! * a "respawn" successor with a NEW pid but the same group (the #115
//!   self-update/eviction pid churn, without setsid),
//! * (Linux) a `setsid`-escaped descendant that only the descendant walk
//!   can catch,
//! * the signal-handler primitive (`signal_kill_tracked_groups`), which is
//!   what the SIGINT/SIGTERM/SIGTRAP handler in `sh-ui/src/main.rs` calls.
//!
//! The two tests share the process-global registry, so they serialize on a
//! static lock (cargo runs tests in parallel threads by default).

#![cfg(unix)]

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

static SERIAL: Mutex<()> = Mutex::new(());

fn tmpdir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "strikehub-115-{}-{}-{}",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn wait_for_file(path: &Path, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn read_pid(path: &Path) -> u32 {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("marker {path:?} unreadable: {e}"))
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("marker {path:?} not a pid: {e}"))
}

fn pid_alive(pid: u32) -> bool {
    match unsafe { libc::kill(pid as libc::pid_t, 0) } {
        0 => true,
        _ => std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH),
    }
}

#[cfg(target_os = "linux")]
fn is_zombie(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    // state is the first char after the last ')'
    let close = match stat.rfind(')') {
        Some(c) => c,
        None => return false,
    };
    stat.get(close + 1..close + 2) == Some(" ") && stat.get(close + 2..close + 3) == Some("Z")
}

#[cfg(not(target_os = "linux"))]
fn is_zombie(_pid: u32) -> bool {
    false
}

/// Wait until `pid` is gone: ESRCH from kill, OR a zombie (already dead,
/// awaiting reap by its parent — the test process for direct children,
/// init for reparented grandchildren). A zombie must count as dead: the
/// teardown under test sent the killing blow; reaping is bookkeeping.
fn wait_gone(pid: u32, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !pid_alive(pid) || is_zombie(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("{what} pid {pid} survived teardown");
}

/// The fixture: a child shell (its OWN process group, via spawn_tracked)
/// that
///   1. spawns a grandchild `sleep` (LEAF) — same group as the child,
///   2. spawns a middle shell which spawns a successor `sleep` (SUCCESSOR —
///      a NEW pid, simulating the #115 eviction/self-update respawn pid
///      churn without setsid) and waits,
///   3. (Linux, when `setsid` exists) spawns a `setsid`'d shell that leaves
///      the group entirely (ESCAPED) and can only be caught by the
///      descendant walk.
///
/// Pids are written to marker files under `dir`.
fn fixture_script(dir: &Path, with_setsid: bool) -> String {
    let leaf = dir.join("leaf.pid");
    let succ = dir.join("succ.pid");
    let esc = dir.join("esc.pid");
    let linux_part = if with_setsid {
        format!(
            "setsid sh -c 'echo $$ > {esc}; sleep 300' &\n",
            esc = esc.display()
        )
    } else {
        String::new()
    };
    format!(
        "sleep 300 & echo $! > {leaf}\n\
         sh -c 'sleep 301 & echo $! > {succ}; wait' &\n\
         {linux_part}\
         wait\n",
        leaf = leaf.display(),
        succ = succ.display(),
    )
}

fn setsid_available() -> bool {
    std::process::Command::new("setsid")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread")]
async fn teardown_process_tree_kills_every_node_including_respawn_and_setsid() {
    let _lock = SERIAL.lock().unwrap();
    let dir = tmpdir("tree");
    let with_setsid = cfg!(target_os = "linux") && setsid_available();
    let script = fixture_script(&dir, with_setsid);

    // Spawn through the REAL API: own process group + registry entry.
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(&script);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    let mut child = sh_core::process::spawn_tracked(&mut cmd).unwrap();
    let child_pid = child.id().unwrap();

    let leaf_path = dir.join("leaf.pid");
    let succ_path = dir.join("succ.pid");
    let esc_path = dir.join("esc.pid");
    assert!(wait_for_file(&leaf_path, 5), "leaf marker never appeared");
    assert!(
        wait_for_file(&succ_path, 5),
        "successor marker never appeared"
    );
    if with_setsid {
        assert!(wait_for_file(&esc_path, 5), "escaped marker never appeared");
    }

    // The registry must have recorded the child, with its own pgid.
    let tracked = sh_core::process::tracked_children_snapshot();
    let me = tracked
        .iter()
        .find(|c| c.pid == child_pid)
        .expect("spawn_tracked did not register the child");
    assert_eq!(
        me.pgid as u32, child_pid,
        "process_group(0) => pgid == child pid"
    );

    let leaf_pid = read_pid(&leaf_path);
    let succ_pid = read_pid(&succ_path);
    let esc_pid = if with_setsid {
        Some(read_pid(&esc_path))
    } else {
        None
    };

    // Sanity: everything alive; the leaf/successor are NOT the tracked pid
    // (they are the nodes pid-based kill_on_drop would miss).
    assert!(
        pid_alive(child_pid) && pid_alive(leaf_pid) && pid_alive(succ_pid),
        "fixture tree not fully up"
    );
    assert_ne!(leaf_pid, child_pid);
    assert_ne!(succ_pid, child_pid);
    if let Some(p) = esc_pid {
        assert!(pid_alive(p) && p != child_pid);
    }

    // The descendant walk (the setsid-escaper net) sees the whole subtree.
    let descendants = sh_core::process::collect_descendants(&[child_pid]);
    assert!(
        descendants.contains(&leaf_pid),
        "walk missed the grandchild leaf"
    );
    assert!(
        descendants.contains(&succ_pid),
        "walk missed the respawn successor"
    );
    if let Some(p) = esc_pid {
        assert!(
            descendants.contains(&p),
            "walk missed the setsid-escaped descendant"
        );
    }

    // ── TEARDOWN — the function under test (normal-close path) ──
    sh_core::process::teardown_process_tree();

    // AC: zero survivors (teardown is bounded to a 2 s grace).
    //
    // Reap the direct child first (our zombie until the reaper gets to
    // it); grandchildren are reparented to init and reaped there.
    for _ in 0..60 {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    wait_gone(child_pid, "child");
    wait_gone(leaf_pid, "grandchild leaf");
    wait_gone(succ_pid, "respawn successor (new pid, same group)");
    if let Some(p) = esc_pid {
        wait_gone(p, "setsid-escaped descendant");
    }
    // The group is empty once its last member is reaped (direct child by
    // us, the rest by init after reparenting) — poll for ESRCH.
    let group_dead = (|| {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if unsafe { libc::kill(-(me.pgid), 0) } == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    })();
    assert!(
        group_dead,
        "tracked process group must become empty after teardown"
    );

    // Idempotent: a second teardown (e.g. ProcessTreeGuard after the
    // explicit call) is a fast no-op, not an error.
    let again = std::time::Instant::now();
    sh_core::process::teardown_process_tree();
    assert!(
        again.elapsed() < Duration::from_secs(3),
        "second teardown must be a no-op"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn signal_kill_tracked_groups_is_the_handler_path_and_reaches_grandchildren() {
    let _lock = SERIAL.lock().unwrap();
    let dir = tmpdir("sig");

    // Simpler fixture: child shell + one grandchild sleep in its group.
    let leaf_path = dir.join("leaf.pid");
    let script = format!(
        "sleep 300 & echo $! > {leaf}; wait",
        leaf = leaf_path.display()
    );
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(&script);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    let mut child = sh_core::process::spawn_tracked(&mut cmd).unwrap();
    let child_pid = child.id().unwrap();
    assert!(wait_for_file(&leaf_path, 5), "leaf marker never appeared");
    let leaf_pid = read_pid(&leaf_path);
    assert!(pid_alive(child_pid) && pid_alive(leaf_pid));

    // The EXACT primitive the SIGINT/SIGTERM/SIGTRAP handler in
    // sh-ui/src/main.rs calls: TERM the tracked groups, grace, KILL.
    sh_core::process::signal_kill_tracked_groups(libc::SIGTERM);
    std::thread::sleep(Duration::from_millis(500));
    sh_core::process::signal_kill_tracked_groups(libc::SIGKILL);

    for _ in 0..60 {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    wait_gone(child_pid, "child (signal-handler path)");
    wait_gone(leaf_pid, "grandchild (signal-handler path)");

    // A stale second spawn (eviction/respawn churn) must also be tracked:
    // spawn another fixture, kill it via the group, and confirm BOTH the
    // old and the new entry behave (old = ESRCH no-op, new = killed).
    let leaf2 = dir.join("leaf2.pid");
    let script2 = format!(
        "sleep 300 & echo $! > {leaf2}; wait",
        leaf2 = leaf2.display()
    );
    let mut cmd2 = tokio::process::Command::new("sh");
    cmd2.arg("-c").arg(&script2);
    cmd2.stdout(std::process::Stdio::null());
    cmd2.stderr(std::process::Stdio::null());
    let mut child2 = sh_core::process::spawn_tracked(&mut cmd2).unwrap();
    let child2_pid = child2.id().unwrap();
    assert!(wait_for_file(&leaf2, 5), "leaf2 marker never appeared");
    let leaf2_pid = read_pid(&leaf2);
    let tracked = sh_core::process::tracked_children_snapshot();
    assert!(
        tracked.iter().any(|c| c.pid == child_pid) && tracked.iter().any(|c| c.pid == child2_pid),
        "registry must keep both (respawned) children"
    );
    sh_core::process::teardown_process_tree();
    for _ in 0..60 {
        if child2.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    wait_gone(child2_pid, "child2 (respawn entry)");
    wait_gone(leaf2_pid, "leaf2 (respawn entry)");

    let _ = std::fs::remove_dir_all(&dir);
}
