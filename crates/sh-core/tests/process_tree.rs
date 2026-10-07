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

use std::path::{Path, PathBuf};
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
    let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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
    let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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

// ── #115 RC remainder — self-update respawn outside the registry ─────────
//
// The RC verification (issue #115, josh-devo's REPRO table) caught a
// `pentest-agent` (pid 87125, pgid=own, ppid→launchd) running from
// `~/.strike48/strikehub/bin/` — the self-update cache — surviving a clean
// quit. When the tracked original dies during a self-update and its
// successor detaches (setsid) and is reparented to init/launchd, NEITHER
// the group kill (the tracked group is empty) NOR the descendant walk
// (anchor pids are dead, the successor's ppid is no longer tracked) can
// see it. This test builds exactly that shape through the REAL teardown
// entry point:
//
//   tracked child (OWN pgid via spawn_tracked; `sh -c '... & wait'`)
//     └── [setsid] successor — a REAL executable copied into
//         bin_cache_dir() (the self-update root), reparented to
//         init/launchd the moment the tracked child dies
//
// AC: `teardown_process_tree()` still leaves ZERO survivors.

/// Resolve a real standalone `perl` executable via PATH. The fixture must
/// be a REAL single-purpose binary, not a shell script (for a script,
/// `/proc/<pid>/exe` (Linux) and `proc_pidpath` (macOS) report the
/// INTERPRETER, not the script) and not a multicall/coreutils dispatch
/// binary (those exec based on argv[0]/PATH and fail when copied under an
/// arbitrary name). `perl -e 'select(…)'` sleeps without spawning children
/// and exists standalone on both the NixOS dev box and the ubuntu CI
/// runner (perl-base is Essential on Ubuntu).
fn real_perl_binary() -> PathBuf {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v perl")
        .output()
        .expect("sh not available");
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!p.is_empty(), "no `perl` on PATH");
    PathBuf::from(p)
}

/// (pid, resolved-exe-path) for every live process whose executable lives
/// under `root`. Test-local enumeration: /proc on Linux, `ps comm` on
/// macOS. (The production sweep's own enumeration is what is under test.)
fn pids_exe_under(root: &Path) -> Vec<(u32, PathBuf)> {
    let root = match std::fs::canonicalize(root) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for e in entries.flatten() {
                let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else {
                    continue;
                };
                if let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe"))
                    && exe.starts_with(&root)
                {
                    out.push((pid, exe));
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let root_s = root.to_string_lossy().to_string();
        if let Ok(o) = std::process::Command::new("ps")
            .args(["-axo", "pid=,comm="])
            .output()
        {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                let mut it = line.split_whitespace();
                let (Some(p), Some(c)) = (it.next(), it.next()) else {
                    continue;
                };
                if let Ok(pid) = p.parse::<u32>()
                    && c.starts_with(&root_s)
                {
                    out.push((pid, PathBuf::from(c.to_string())));
                }
            }
        }
    }
    out
}

/// Sweep-safety guard: the production teardown sweep kills EVERYTHING whose
/// resolved exe lives under the managed root. A dev box may have a live
/// connector running from `~/.strike48/strikehub/bin/` (a real
/// self-update generation) — never run a sweep test against that. CI
/// boxes do not have the directory populated, so the guard passes there.
fn assert_no_live_managed_procs(root: &Path) {
    let foreign = pids_exe_under(root);
    assert!(
        foreign.is_empty(),
        "live process(es) running from managed root {:?} ({foreign:?}) — \n         refusing to run a managed-root teardown test on this box; kill them first",
        root
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn respawned_successor_outside_registry_is_still_collected() {
    let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());

    // The managed root the real self-update writes to: ~/.strike48/strikehub/bin
    let root = sh_core::bin_cache_dir();
    let fixture_name = format!("sh-115b-sweep-fixture-{}", std::process::id());
    let fixture = root.join(&fixture_name);

    assert_no_live_managed_procs(&root);
    std::fs::create_dir_all(&root).unwrap();

    // A REAL executable under the managed root — the self-update
    // generation. (A copied `perl` that sleeps without children: see
    // `real_perl_binary` for why not `sleep`/a script.)
    std::fs::copy(real_perl_binary(), &fixture).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&fixture).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&fixture, p).unwrap();
    }

    let dir = tmpdir("respawn");
    let with_setsid = cfg!(target_os = "linux") && setsid_available();
    // The tracked child: spawn the successor DETACHED (setsid => the
    // successor leaves into its own session AND group) and wait. The
    // successor's ppid becomes init/launchd (or the nearest subreaper)
    // the moment this child dies — the RC shape (ppid=1 on the lab box).
    let script = if with_setsid {
        format!(
            "setsid {} -e 'select(undef,undef,undef,300)' & wait",
            fixture.display()
        )
    } else {
        // macOS: no setsid utility — perl fork + setsid + exec. The helper
        // parent waits out the successor, so it exits with it (it is not an
        // extra survivor: /usr/bin/perl is not under the managed root).
        // (`use POSIX;` is mandatory — `POSIX::setsid()` is not
        // auto-loaded by perl.)
        format!(
            "perl -e 'use POSIX; $p=fork() or POSIX::_exit(98); \n\
             if ($p==0) {{ POSIX::setsid(); exec @ARGV or POSIX::_exit(99); }} \n\
             waitpid($p, 0);' {} -e 'select(undef,undef,undef,300)' & wait\n",
            fixture.display()
        )
    };

    // Spawn through the REAL API: own process group + registry entry.
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(&script);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    let mut child = sh_core::process::spawn_tracked(&mut cmd).unwrap();
    let child_pid = child.id().unwrap();

    // Find the successor: a live process running the fixture binary, which
    // is NOT the tracked child. (Unique fixture name => no ambiguity.)
    let mut succ_pid: Option<u32> = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while succ_pid.is_none() && Instant::now() < deadline {
        succ_pid = pids_exe_under(&root)
            .into_iter()
            .map(|(pid, _)| pid)
            .find(|p| *p != child_pid && pid_alive(*p));
        if succ_pid.is_none() {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let succ_pid = succ_pid.expect("successor (self-update generation) never appeared");
    assert!(pid_alive(child_pid), "tracked child must be alive pre-kill");
    assert_ne!(succ_pid, child_pid);

    // ── Simulate the self-update original dying (the RC shape) ──
    // (sync start_kill + try_wait: no await points while holding SERIAL)
    child.start_kill().unwrap();
    for _ in 0..60 {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    wait_gone(child_pid, "tracked original (self-update died)");

    // Wait until the descendant walk from ALL tracked pids can no longer
    // see the successor: it has been reparented (ppid is init/launchd or a
    // subreaper — never a tracked anchor) and it leads its OWN group
    // (setsid), so the group kill cannot reach it either.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut invisible_to_walk = false;
    while Instant::now() < deadline {
        let tracked_pids: Vec<u32> = sh_core::process::tracked_children_snapshot()
            .iter()
            .map(|c| c.pid)
            .collect();
        if !sh_core::process::collect_descendants(&tracked_pids).contains(&succ_pid) {
            invisible_to_walk = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        invisible_to_walk,
        "descendant walk still sees the successor — test premise (reparented + setsid) not met"
    );
    assert!(pid_alive(succ_pid), "successor must be alive pre-teardown");

    // ── TEARDOWN — the function under test (normal-close / atexit path) ──
    sh_core::process::teardown_process_tree();

    // AC (issue #115 RC verification): zero survivors — the self-update
    // generation that escaped the registry must be collected too.
    wait_gone(succ_pid, "self-update successor (respawn outside registry)");

    let _ = std::fs::remove_file(&fixture);
    let _ = std::fs::remove_dir_all(&dir);
}

// ── #115 AC — zero DESCENDANTS: the sweep must collect the full subtree ──
//
// Issue #115's acceptance criterion is ZERO descended processes — "children
// and grandchildren, including self-update-respawned ...". A self-update
// successor is not a leaf: it spawns children, and those descendants exec
// ordinary system binaries that live OUTSIDE every managed root, so the
// path whitelist alone can never see them — and, being untracked and
// reparented after the original's death, the tracked-anchor walk misses
// them too. Only the descendant walk from the sweep's MATCHED roots
// reaches them. This test builds exactly that shape:
//
//   tracked child (OWN pgid via spawn_tracked; `sh -c '... & wait'`)
//     └── [setsid] successor — a REAL executable copied into
//         bin_cache_dir() (the self-update root)
//           └── child (a fork of the successor; its exe is still the
//               fixture, so the whitelist matches it)
//                 └── grandchild (exec'd to `sleep` — a system binary
//                     OUTSIDE every root: whitelist-blind, walk-only)
//
// The tracked original dies (the self-update death): successor, child and
// grandchild are outside the tracked tree. AC: `teardown_process_tree()`
// leaves ZERO survivors.

#[tokio::test(flavor = "multi_thread")]
async fn sweep_collects_full_subtree_under_managed_root_including_grandchildren() {
    let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());

    // The managed root the real self-update writes to: ~/.strike48/strikehub/bin
    let root = sh_core::bin_cache_dir();
    let fixture_name = format!("sh-115c-sweep-subtree-{}", std::process::id());
    let fixture = root.join(&fixture_name);

    assert_no_live_managed_procs(&root);
    std::fs::create_dir_all(&root).unwrap();

    // A REAL executable under the managed root — the self-update
    // generation (see `real_perl_binary` for why perl, not a script).
    std::fs::copy(real_perl_binary(), &fixture).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&fixture).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&fixture, p).unwrap();
    }

    let dir = tmpdir("subtree");
    let child_marker = dir.join("child.pid");
    let grand_marker = dir.join("grand.pid");

    // The successor's program ($ARGV[0] = child marker, $ARGV[1] = grand
    // marker): fork a child; the child forks a grandchild that execs
    // `sleep` (a system binary OUTSIDE every managed root — the whitelist
    // can never see it) and records the pids as they appear. No single
    // quotes in the program: it is embedded in a single-quoted shell arg.
    let program = r#"
        my $c = fork();
        if ($c == 0) {
            my $g = fork();
            if ($g == 0) {
                exec("sleep", "300") or POSIX::_exit(99);
            }
            open(my $gfh, ">", $ARGV[1]) or POSIX::_exit(97);
            print $gfh $g;
            close $gfh;
            waitpid($g, 0);
            POSIX::_exit(0);
        }
        open(my $cfh, ">", $ARGV[0]) or POSIX::_exit(96);
        print $cfh $c;
        close $cfh;
        select(undef, undef, undef, 300);
    "#;

    let with_setsid = cfg!(target_os = "linux") && setsid_available();
    // The tracked child: spawn the successor DETACHED (setsid => the
    // successor leaves into its own session AND group) and wait. The
    // successor's ppid becomes init/launchd (or the nearest subreaper)
    // the moment this child dies — the RC shape.
    let script = if with_setsid {
        format!(
            "setsid {fixture} -e '{program}' {child} {grand} & wait",
            fixture = fixture.display(),
            child = child_marker.display(),
            grand = grand_marker.display(),
        )
    } else {
        // macOS: no setsid utility — a perl helper fork + setsid + exec's
        // the fixture. The helper is a child of the tracked shell and
        // waits out the successor; its exe is the REAL perl (outside the
        // managed root), so it is an ancestor of the sweep roots, never a
        // sweep target, and is out of the AC's descendant set here.
        // (`use POSIX;` is mandatory — `POSIX::setsid()` is not
        // auto-loaded by perl.)
        format!(
            "perl -e 'use POSIX; $p=fork() or POSIX::_exit(98); \n\n\
             if ($p==0) {{ POSIX::setsid(); exec @ARGV or POSIX::_exit(99); }} \n\n\
             waitpid($p, 0);' {fixture} -e '{program}' {child} {grand} & wait\n",
            fixture = fixture.display(),
            child = child_marker.display(),
            grand = grand_marker.display(),
        )
    };

    // Spawn through the REAL API: own process group + registry entry.
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(&script);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    let mut child = sh_core::process::spawn_tracked(&mut cmd).unwrap();
    let child_pid = child.id().unwrap();

    // The pids as the FIXTURE reports them (not the shell's `$!` —
    // setsid may fork when it is a group leader).
    assert!(
        wait_for_file(&child_marker, 10),
        "child marker never appeared"
    );
    assert!(
        wait_for_file(&grand_marker, 10),
        "grandchild marker never appeared"
    );
    let sub_child_pid = read_pid(&child_marker);
    let grand_pid = read_pid(&grand_marker);

    // The successor: a live process running the fixture binary (under the
    // managed root) that is neither the tracked child nor the recorded
    // child (the child fork keeps the fixture's exe too — it is the
    // recorded one). Unique fixture name => no ambiguity.
    let mut succ_pid: Option<u32> = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while succ_pid.is_none() && Instant::now() < deadline {
        succ_pid = pids_exe_under(&root)
            .into_iter()
            .map(|(pid, _)| pid)
            .find(|p| *p != child_pid && *p != sub_child_pid && pid_alive(*p));
        if succ_pid.is_none() {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let succ_pid = succ_pid.expect("successor (self-update generation) never appeared");

    // Sanity: the whole 3-level shape is up, and the premise holds — the
    // grandchild's resolved exe is OUTSIDE every managed root, so the
    // path whitelist alone can NEVER collect it; only the walk from the
    // sweep's matched roots can.
    assert!(
        pid_alive(child_pid)
            && pid_alive(succ_pid)
            && pid_alive(sub_child_pid)
            && pid_alive(grand_pid),
        "fixture subtree not fully up"
    );
    assert_ne!(grand_pid, succ_pid);
    assert_ne!(grand_pid, sub_child_pid);
    assert!(
        !pids_exe_under(&root)
            .iter()
            .any(|(pid, _)| *pid == grand_pid),
        "premise broken: the grandchild's exe must be OUTSIDE the managed root (exec'd to a system binary)"
    );

    // ── Simulate the self-update original dying (the RC shape) ──
    // (sync start_kill + try_wait: no await points while holding SERIAL)
    child.start_kill().unwrap();
    for _ in 0..60 {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    wait_gone(child_pid, "tracked original (self-update died)");

    // Wait until the descendant walk from ALL tracked pids can no longer
    // see ANY node of the subtree: the successor has been reparented
    // (ppid is init/launchd or a subreaper — never a tracked anchor) and
    // leads its OWN group (setsid), so the group kill cannot reach it
    // either.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut invisible_to_walk = false;
    while Instant::now() < deadline {
        let tracked_pids: Vec<u32> = sh_core::process::tracked_children_snapshot()
            .iter()
            .map(|c| c.pid)
            .collect();
        let desc = sh_core::process::collect_descendants(&tracked_pids);
        if !desc.contains(&succ_pid) && !desc.contains(&sub_child_pid) && !desc.contains(&grand_pid)
        {
            invisible_to_walk = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        invisible_to_walk,
        "descendant walk still sees the subtree — test premise (reparented + setsid) not met"
    );
    assert!(
        pid_alive(succ_pid) && pid_alive(sub_child_pid) && pid_alive(grand_pid),
        "subtree must be alive pre-teardown"
    );

    // ── TEARDOWN — the function under test (normal-close / atexit path) ──
    sh_core::process::teardown_process_tree();

    // AC (issue #115): ZERO descended processes — children AND
    // grandchildren of the self-update generation, including the
    // grandchild whose exe the whitelist can never see.
    wait_gone(succ_pid, "self-update successor (managed root)");
    wait_gone(sub_child_pid, "successor's child");
    wait_gone(
        grand_pid,
        "successor's grandchild (exe outside every managed root — walk-only coverage)",
    );

    let _ = std::fs::remove_file(&fixture);
    let _ = std::fs::remove_dir_all(&dir);
}

// ── #115 RC remainder — quit-event route (std::process::exit) ────────────
//
// tao 0.30.8's EventLoop::run (what dioxus 0.6.3's launch() blocks in) is
// diverging: on EVERY GUI exit route (window close, macOS
// application-terminate — AppleEvent quit / Cmd-Q / dock Quit) it ends in
// std::process::exit. That runs C-level atexit handlers but no Rust Drop
// impls, and none of the code after launch() in sh-ui/src/main.rs. The
// tests below prove the atexit hook is what delivers the teardown there.

/// The EXACT entry the atexit hook runs (`sh_core::process::atexit_teardown`,
/// wired in sh-ui/src/main.rs via `install_exit_handler` and registered
/// with libc::atexit): calling it directly must leave zero survivors.
#[tokio::test(flavor = "multi_thread")]
async fn atexit_teardown_entry_is_the_quit_route_tear_down() {
    let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // The entry point runs the full teardown INCLUDING the managed-root
    // sweep — guard against a dev box with a live connector running from
    // the real managed root.
    assert_no_live_managed_procs(&sh_core::bin_cache_dir());
    let dir = tmpdir("atexit-entry");

    // Fixture: tracked child shell + grandchild sleep in its group — the
    // same shape as the close/TERM-route tests.
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
    assert!(
        pid_alive(child_pid) && pid_alive(leaf_pid),
        "fixture tree not fully up"
    );

    // The quit-route entry point, called directly.
    sh_core::process::atexit_teardown();

    for _ in 0..60 {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    wait_gone(child_pid, "child (atexit entry path)");
    wait_gone(leaf_pid, "grandchild (atexit entry path)");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The atexit hook must actually FIRE on `std::process::exit` — the exit
/// tao's EventLoop::run performs on every GUI close/quit route — where
/// Rust Drop impls (ProcessTreeGuard, kill_on_drop) never run.
///
/// Mechanism: spawn a fixture through the REAL registry API, then FORK.
/// The forked child inherits the registry state; it installs the hook
/// (the exact wiring sh-ui/src/main.rs does) and exits the way tao does
/// (`std::process::exit(42)`). If the hook fires, its teardown kills the
/// fixture's group; the parent asserts exit code 42 AND zero survivors.
///
/// The waitpid is bounded: if the forked child deadlocked on a lock held
/// by a parallel test at the instant of fork, this fails loudly instead
/// of hanging CI.
#[tokio::test(flavor = "multi_thread")]
async fn atexit_hook_fires_on_process_exit() {
    let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // The atexit teardown inside the forked child runs the FULL teardown
    // (incl. the managed-root sweep) — guard against a dev box with a
    // live connector running from the real managed root.
    assert_no_live_managed_procs(&sh_core::bin_cache_dir());
    let dir = tmpdir("atexit-fire");
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
    assert!(
        pid_alive(child_pid) && pid_alive(leaf_pid),
        "fixture tree not fully up"
    );

    // SAFETY: fork on a test thread we control; the child performs only
    // libc::atexit + std::process::exit (no Rust allocation paths beyond
    // the once-init in install_exit_handler, no tokio, no Drop impls —
    // the process exits instead of unwinding).
    let forked = unsafe { libc::fork() };
    assert!(
        forked >= 0,
        "fork failed: {}",
        std::io::Error::last_os_error()
    );
    if forked == 0 {
        // Child: the EXACT startup wiring from sh-ui/src/main.rs, then
        // the EXACT exit tao's EventLoop::run performs on the quit route.
        sh_core::process::install_exit_handler();
        std::process::exit(42);
    }

    // Parent: bounded wait for the child's exit code.
    let mut status: libc::c_int = 0;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let w = unsafe { libc::waitpid(forked, &mut status, libc::WNOHANG) };
        if w == forked {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "forked atexit child hung (lock held at fork? teardown deadlock?)"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 42,
        "forked child did not exit cleanly via std::process::exit(42): status={status}"
    );

    // The atexit teardown (run inside the forked child) must have killed
    // the fixture — zero survivors, even though NO code in this process
    // ever called teardown and the Child handle's Drop never ran.
    for _ in 0..60 {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    wait_gone(child_pid, "child (atexit-fired on process::exit)");
    wait_gone(leaf_pid, "grandchild (atexit-fired on process::exit)");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── #115 RC remainder — sweep safety (whitelist is the safety property) ──

/// Adversarial: a process launched THROUGH a symlink that sits inside a
/// managed root but resolves to an executable OUTSIDE every root must be
/// REJECTED by the sweep (it survives), while a process whose resolved
/// exe IS under the root is collected. This is the "never touch unrelated
/// processes" guarantee — the whitelist matches RESOLVED paths only.
#[tokio::test(flavor = "multi_thread")]
async fn sweep_rejects_symlink_escape_outside_roots() {
    let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let base = tmpdir("sweep-safe");

    // Two trees: `root` (registered as a managed root) and `outside`
    // (not managed).
    let root = base.join("root");
    let outside = base.join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    sh_core::process::add_managed_root(root.clone());

    let perl = real_perl_binary();
    let victim = outside.join("agent"); // resolved OUTSIDE all roots
    let legit = root.join("agent"); // resolved INSIDE a root
    for f in [&victim, &legit] {
        std::fs::copy(&perl, f).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(f).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(f, p).unwrap();
        }
    }
    // The escape hatch: a symlink INSIDE the managed root whose target
    // lives outside it.
    std::os::unix::fs::symlink(&victim, root.join("evil-link")).unwrap();

    let sleep_script = "-e 'select(undef,undef,undef,300)'";
    // Launched through the symlink — its resolved exe is `outside/agent`.
    let mut esc_cmd = tokio::process::Command::new("sh");
    esc_cmd.arg("-c").arg(format!(
        "{link} {script}",
        link = root.join("evil-link").display(),
        script = sleep_script
    ));
    esc_cmd.stdout(std::process::Stdio::null());
    esc_cmd.stderr(std::process::Stdio::null());
    let mut esc = esc_cmd.spawn().unwrap();
    // Launched directly from inside the root — its resolved exe is
    // `root/agent` (a controlled victim: proves the sweep is live in this
    // test, so the rejection above is not vacuous).
    let mut leg_cmd = tokio::process::Command::new("sh");
    leg_cmd.arg("-c").arg(format!(
        "{leg} {script}",
        leg = legit.display(),
        script = sleep_script
    ));
    leg_cmd.stdout(std::process::Stdio::null());
    leg_cmd.stderr(std::process::Stdio::null());
    let mut leg = leg_cmd.spawn().unwrap();

    // Wait for both to be exec'd (a single-command `sh -c 'cmd'` execs in
    // place on the common shells, so the Child pids ARE the perl pids —
    // but we track by resolved exe path anyway, which is also how the
    // production sweep identifies them).
    let victim_canon = std::fs::canonicalize(&victim).unwrap();
    let legit_canon = std::fs::canonicalize(&legit).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut esc_pid = None;
    let mut leg_pid = None;
    while (esc_pid.is_none() || leg_pid.is_none()) && Instant::now() < deadline {
        for (pid, exe) in pids_exe_under(&base) {
            if esc_pid.is_none() && exe == victim_canon {
                esc_pid = Some(pid);
            }
            if leg_pid.is_none() && exe == legit_canon {
                leg_pid = Some(pid);
            }
        }
        if esc_pid.is_none() || leg_pid.is_none() {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let esc_pid = esc_pid.expect("symlink-escaped process never appeared");
    let leg_pid = leg_pid.expect("managed-root process never appeared");
    assert!(
        pid_alive(esc_pid) && pid_alive(leg_pid),
        "fixture not fully up"
    );

    // The sweep under the REAL managed roots (incl. the registered test
    // root) must see ONLY the legit process.
    let swept = sh_core::process::sweep_managed_roots(&sh_core::process::managed_roots());
    assert!(
        swept.contains(&leg_pid),
        "sweep missed the process whose resolved exe is under the managed root"
    );
    assert!(
        !swept.contains(&esc_pid),
        "sweep matched a symlink-escaped process (resolved exe outside all roots) — whitelist violated"
    );

    // Full teardown: the legit process dies, the escapee survives.
    sh_core::process::teardown_process_tree();
    wait_gone(leg_pid, "managed-root process (swept)");
    assert!(
        pid_alive(esc_pid),
        "symlink-escaped process must SURVIVE the sweep (safety: never touch unrelated procs)"
    );

    // Test hygiene: the escapee is not ours to sweep — reap it explicitly.
    // (sync start_kill + try_wait: no await points while holding SERIAL)
    esc.start_kill().unwrap();
    leg.start_kill().unwrap();
    for h in [&mut esc, &mut leg] {
        for _ in 0..60 {
            if h.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let _ = std::fs::remove_dir_all(&base);
}
