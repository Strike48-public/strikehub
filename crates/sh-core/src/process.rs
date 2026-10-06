//! Process-group management, whole-tree teardown (Unix) and Job Object
//! wiring (Windows).
//!
//! # Process-group containment (#110)
//!
//! The desktop app terminates its child connector processes by signalling
//! the process group(s) they live in. That is only safe if StrikeHub
//! actually LEADS its own group — which is not true when the app is
//! launched from a desktop launcher (GNOME app grid, dock, file manager,
//! ...): the launcher's spawn leaves the child in the LAUNCHER's process
//! group. Signalling that inherited group would SIGTERM the launcher
//! itself (e.g. GNOME Shell) and every other member of that group —
//! project-management#380 (377 finding 7), HIGH.
//!
//! The fix is to call [`detach_process_group`] early at startup, before any
//! child is spawned. Everything StrikeHub launches afterwards (connectors,
//! helpers) inherits the private group, so a `killpg(getpgrp(), ...)` on
//! shutdown reaches only StrikeHub and its own children.
//!
//! # Tree teardown (#115)
//!
//! #110 kept signals from escaping *upward*; #115 is the mirror: children
//! (and their descendants) must not survive *downward* when StrikeHub
//! exits. `kill_on_drop` only ever signals the DIRECT child's pid, and it
//! does not run at all when the hub dies from a signal (on Linux, a
//! WebKitGTK window close kills the hub with SIGTRAP before destructors
//! run — issue #115 Linux repro, exit 133). So the hub additionally:
//!
//! 1. spawns every child in its OWN process group (`process_group(0)`,
//!    see [`spawn_tracked`]) and registers it for the process lifetime
//!    ([`track_child`]),
//! 2. on normal close ([`teardown_process_tree`]) or on SIGINT/SIGTERM/
//!    SIGTRAP (handler in `sh-ui/src/main.rs`) sends SIGTERM to every
//!    tracked group, waits ~2 s, then SIGKILLs whatever survives —
//!    `kill(-pgid)` reaches grandchildren too, because they inherit their
//!    parent's group unless they detach via `setsid`,
//! 3. as a best effort for descendants that DID detach, walks the
//!    descendant tree from each tracked pid before killing
//!    ([`collect_descendants`] — `/proc` on Linux, `ps` on macOS).
//!
//! On Windows the same guarantee comes from one shared Job Object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (see [`crate::job`]): every child
//! is assigned at spawn, the whole job is terminated on close, and the OS
//! kills the job when the hub's handle closes on any exit path.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// Put the calling process in a new process group of which it is the leader.
///
/// Must be called early at startup, BEFORE any child process is spawned, so
/// that everything StrikeHub launches inherits the private group.
///
/// Failure is not fatal: when we are already a session leader (e.g. launched
/// under `setsid`) `setpgid(0, 0)` returns `EPERM`, but we already own a
/// process group and nothing needs to be done. (Since #115 the shutdown path
/// signals the children's OWN groups — tracked at spawn — rather than this
/// one, but the group must stay private so terminal SIGINT delivery and any
/// defensive killpg never reach the launcher.)
#[cfg(unix)]
pub fn detach_process_group() {
    // SAFETY: setpgid(0, 0) only affects the calling process; it takes no
    // arguments that alias shared state and is safe at startup.
    let rc = unsafe { libc::setpgid(0, 0) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        // EPERM (already a session leader) is expected and benign; anything
        // else means we may still be in the launcher's group — warn, because
        // the shutdown handler then refuses to killpg the inherited group.
        tracing::warn!(
            error = %err,
            "setpgid(0,0) failed; shutdown will not signal the process group"
        );
    }
}

/// True if the calling process is the leader of its own process group, i.e.
/// whether `killpg(getpgrp(), ...)` would only reach this process and its
/// descendants.
#[cfg(unix)]
#[must_use]
pub fn is_process_group_leader() -> bool {
    // SAFETY: both calls operate on the calling process and take no shared
    // state; both are async-signal-safe.
    unsafe { libc::getpgrp() == libc::getpid() }
}

// ── Process-tree registry (#115) ─────────────────────────────────────

/// Grace period between the SIGTERM and the SIGKILL escalation.
pub const TEARDOWN_GRACE_SECS: u64 = 2;

/// Bounded re-verification of the child's process group after spawn
/// (review of #116, nit T1): std runs `setpgid` in the child's `pre_exec`,
/// so a `getpgid` read in the fork→exec window can still see the
/// INHERITED group. Re-read until the child is observed leading its own
/// group; 25 × 2 ms = ≤ ~50 ms worst case, and connector spawns are rare
/// (health-check churn ~18 s), so the stall is not observable in practice.
#[cfg(unix)]
const PGID_VERIFY_MAX_ATTEMPTS: u32 = 25;

#[cfg(unix)]
const PGID_VERIFY_RETRY: std::time::Duration = std::time::Duration::from_millis(2);

/// A child process StrikeHub spawned and must collect on exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackedChild {
    /// Direct child pid as returned at spawn.
    pub pid: u32,
    /// (Unix) the process group the child leads. `process_group(0)` at
    /// spawn makes this the child's own pid.
    #[cfg(unix)]
    pub pgid: i32,
}

/// Registry of every child spawned since startup, for whole-tree teardown.
///
/// Append-only for the process lifetime: evicted/respawned connectors add
/// entries and old ones are kept (their groups are dead by then — `kill`
/// simply returns ESRCH). That keeps the registry trivially safe to read
/// from the signal handler (a single `try_lock`, no allocation).
static TRACKED_CHILDREN: Mutex<Vec<TrackedChild>> = Mutex::new(Vec::new());

/// Register a child (spawned via [`spawn_tracked`], or from any other spawn
/// site) for whole-tree teardown on close.
#[inline]
pub fn track_child(pid: u32, #[cfg(unix)] pgid: i32) {
    if let Ok(mut tracked) = TRACKED_CHILDREN.lock() {
        tracked.push(TrackedChild {
            pid,
            #[cfg(unix)]
            pgid,
        });
    }
}

/// Snapshot of all tracked children.
#[must_use]
pub fn tracked_children_snapshot() -> Vec<TrackedChild> {
    TRACKED_CHILDREN
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default()
}

/// Spawn `cmd` with StrikeHub's child-lifecycle guarantees applied and
/// register the child for whole-tree teardown.
///
/// * **Unix** — the child is started in its OWN new process group
///   (`process_group(0)`), so `kill(-pgid)` reaches the child *and*
///   everything it spawned (grandchildren inherit the group unless they
///   detach via `setsid` — covered by the descendant walk in
///   [`teardown_process_tree`]).
/// * **Windows** — the child is assigned to the shared StrikeHub Job
///   Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, see [`crate::job`]) the
///   moment it exists.
///
/// The caller keeps `kill_on_drop(true)` as the direct-child backstop.
#[cfg(unix)]
pub fn spawn_tracked(cmd: &mut tokio::process::Command) -> std::io::Result<tokio::process::Child> {
    cmd.process_group(0);
    let child = cmd.spawn()?;
    let Some(pid) = child.id() else {
        return Err(std::io::Error::other(
            "spawned child has no usable pid; cannot register for tree teardown",
        ));
    };
    // process_group(0) => the new group is the child's own pid, but std
    // runs the setpgid in the child's `pre_exec` — a getpgid read in the
    // fork→exec window can race it and still see the inherited (parent's)
    // group (review of #116, nit T1). Re-verify until the child is
    // observed leading its own group; if it still hasn't after the bound
    // (child preempted the whole window, or re-parented itself via
    // setpgid/setsid), track the last observed group — the direct-pid
    // passes plus the descendant walk in teardown cover the rest.
    let mut pgid = pid as libc::pid_t;
    for _ in 0..PGID_VERIFY_MAX_ATTEMPTS {
        // SAFETY: getpgid on a pid we just spawned; the child is alive
        // (we hold its handle) for the whole loop.
        let g = unsafe { libc::getpgid(pid as libc::pid_t) };
        if g > 0 {
            pgid = g;
            if g == pid as libc::pid_t {
                break; // child leads its own group — the expected steady state
            }
        }
        std::thread::sleep(PGID_VERIFY_RETRY);
    }
    track_child(pid, pgid);
    Ok(child)
}

#[cfg(windows)]
pub fn spawn_tracked(cmd: &mut tokio::process::Command) -> std::io::Result<tokio::process::Child> {
    let child = cmd.spawn()?;
    // Assign to the shared Job Object IMMEDIATELY — no await between spawn
    // and assignment; see `crate::job` for the residual-race analysis.
    if let Some(pid) = child.id() {
        let _ = crate::job::assign_pid_to_job(pid);
        track_child(pid);
    }
    Ok(child)
}

#[cfg(not(any(unix, windows)))]
pub fn spawn_tracked(cmd: &mut tokio::process::Command) -> std::io::Result<tokio::process::Child> {
    let child = cmd.spawn()?;
    if let Some(pid) = child.id() {
        track_child(pid);
    }
    Ok(child)
}

// ── Descendant walk (best effort) ────────────────────────────────────

const MAX_WALK_NODES: usize = 4096;
const MAX_WALK_DEPTH: usize = 32;

/// Best-effort descendant walk: every pid reachable from `roots` through
/// the parent→child (ppid) relation, excluding the roots themselves.
///
/// Bounded (≤ [`MAX_WALK_NODES`] nodes, depth ≤ [`MAX_WALK_DEPTH`]) and
/// best-effort by design: it exists only to add coverage for descendants
/// that escaped their process group via `setsid` (e.g. a respawned
/// connector that detaches); everyone else is caught by the group kill.
/// On Linux it reads the `/proc` ppid table; on macOS it shells out to
/// `ps -axo pid=,ppid=`. NOT for use in signal handlers.
#[must_use]
pub fn collect_descendants(roots: &[u32]) -> Vec<u32> {
    let Some(children) = ppid_children_table() else {
        return Vec::new();
    };
    descendants_from(&children, roots)
}

fn descendants_from(children: &HashMap<u32, Vec<u32>>, roots: &[u32]) -> Vec<u32> {
    let mut seen: HashSet<u32> = roots.iter().copied().collect();
    let mut found: Vec<u32> = Vec::new();
    let mut queue: Vec<(u32, usize)> = roots.iter().copied().map(|r| (r, 0)).collect();
    while let Some((pid, depth)) = queue.pop() {
        if depth >= MAX_WALK_DEPTH {
            continue;
        }
        let Some(kids) = children.get(&pid) else {
            continue;
        };
        for &kid in kids {
            if seen.insert(kid) && found.len() < MAX_WALK_NODES {
                found.push(kid);
                queue.push((kid, depth + 1));
            }
        }
    }
    found
}

/// ppid → children table for the whole system.
#[cfg(target_os = "linux")]
fn ppid_children_table() -> Option<HashMap<u32, Vec<u32>>> {
    let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
    let entries = std::fs::read_dir("/proc").ok()?;
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let name = entry.file_name();
        let Ok(pid) = name.to_string_lossy().parse::<u32>() else {
            continue;
        };
        // The process may exit mid-scan — that entry just drops out.
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // Field 2 (comm) is parenthesised and may itself contain spaces or
        // ')' — anchor on the LAST ')' before the numeric fields. ppid is
        // the first field after it.
        let Some(open) = stat.find('(') else {
            continue;
        };
        let Some(close) = stat.rfind(')') else {
            continue;
        };
        if close <= open {
            continue;
        }
        let mut fields = stat[close + 1..].split_whitespace();
        let _state = fields.next();
        let Some(ppid_s) = fields.next() else {
            continue;
        };
        let Ok(ppid) = ppid_s.parse::<u32>() else {
            continue;
        };
        map.entry(ppid).or_default().push(pid);
    }
    Some(map)
}

/// ppid → children table via `ps` (macOS — no /proc there).
#[cfg(target_os = "macos")]
fn ppid_children_table() -> Option<HashMap<u32, Vec<u32>>> {
    let out = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid="])
        .output()
        .ok()?;
    let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut it = line.split_whitespace();
        let (Some(pid_s), Some(ppid_s)) = (it.next(), it.next()) else {
            continue;
        };
        let (Ok(pid), Ok(ppid)) = (pid_s.parse::<u32>(), ppid_s.parse::<u32>()) else {
            continue;
        };
        map.entry(ppid).or_default().push(pid);
    }
    Some(map)
}

/// Windows has no /proc or portable ppid table; the Job Object makes the
/// walk unnecessary (it owns the whole tree by construction).
#[cfg(target_os = "windows")]
fn ppid_children_table() -> Option<HashMap<u32, Vec<u32>>> {
    None
}

// ── Teardown ─────────────────────────────────────────────────────────

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // kill(pid, 0) is a probe: success => alive; ESRCH => gone; EPERM =>
    // alive but not ours (count it as alive — conservative).
    match unsafe { libc::kill(pid as libc::pid_t, 0) } {
        0 => true,
        _ => std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH),
    }
}

#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    crate::job::pid_alive(pid)
}

#[cfg(not(any(unix, windows)))]
fn pid_alive(_pid: u32) -> bool {
    false
}

/// Tear down every tracked child's process tree (normal-close path).
///
/// 1. Snapshot the registry.
/// 2. Walk the descendants of each tracked pid (catches `setsid`-escaped
///    descendants that a group kill would miss).
/// 3. `SIGTERM` every tracked process group (`kill(-pgid)`) and every
///    walked descendant (Windows: `TerminateJobObject` + direct terminate).
/// 4. Poll for up to [`TEARDOWN_GRACE_SECS`] (~2 s) for the whole set to
///    die; return early when it does.
/// 5. Escalate survivors to `SIGKILL` (Windows: terminate again).
///
/// Synchronous and idempotent — safe to call from `main` after the window
/// closes, from the [`ProcessTreeGuard`] exit fallback, or both. NOT
/// signal-handler-safe (it sleeps, reads /proc, and on macOS spawns
/// `ps`): the signal handler uses [`signal_kill_tracked_groups`] instead.
pub fn teardown_process_tree() {
    let tracked = tracked_children_snapshot();
    if tracked.is_empty() {
        return;
    }

    let pids: Vec<u32> = tracked.iter().map(|c| c.pid).collect();
    let mut targets: Vec<u32> = pids.clone();
    for d in collect_descendants(&pids) {
        if !targets.contains(&d) {
            targets.push(d);
        }
    }

    #[cfg(unix)]
    let groups: Vec<i32> = tracked.iter().map(|c| c.pgid).collect();

    // Graceful first pass.
    #[cfg(unix)]
    {
        kill_groups(&groups, libc::SIGTERM);
        for &pid in &targets {
            let _ = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        }
    }
    #[cfg(windows)]
    {
        crate::job::terminate_job();
        for &pid in &targets {
            crate::job::terminate_pid(pid);
        }
    }

    // Poll for the grace period.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(TEARDOWN_GRACE_SECS);
    let clean = loop {
        // (unix additionally requires every tracked group to be dead)
        #[cfg(unix)]
        let ok = targets.iter().all(|p| !pid_alive(*p)) && groups.iter().all(|g| !group_alive(*g));
        #[cfg(not(unix))]
        let ok = targets.iter().all(|p| !pid_alive(*p));
        if ok || std::time::Instant::now() >= deadline {
            break ok;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };

    if clean {
        tracing::info!(
            count = targets.len(),
            "process tree teardown: all tracked processes exited within grace"
        );
        return;
    }

    // Escalation.
    #[cfg(unix)]
    {
        kill_groups(&groups, libc::SIGKILL);
        for &pid in &targets {
            if pid_alive(pid) {
                let _ = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            }
        }
    }
    #[cfg(windows)]
    {
        crate::job::terminate_job();
        for &pid in &targets {
            if pid_alive(pid) {
                crate::job::terminate_pid(pid);
            }
        }
    }
    tracing::warn!(
        count = targets.len(),
        "process tree teardown: escalation sent to survivors after grace"
    );
}

#[cfg(unix)]
fn kill_groups(groups: &[i32], sig: libc::c_int) {
    for &pgid in groups {
        // ESRCH (group already gone) is the expected steady state — ignored.
        let _ = unsafe { libc::kill(-pgid, sig) };
    }
}

#[cfg(unix)]
fn group_alive(pgid: i32) -> bool {
    match unsafe { libc::kill(-pgid, 0) } {
        0 => true,
        _ => std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH),
    }
}

/// Signal-handler-safe: send `sig` to every tracked child process group.
///
/// Only async-signal-safe calls (`kill`) plus a single `try_lock` on the
/// registry — if a spawn is holding the lock, the loop is skipped (the
/// in-flight child is about to be registered; the re-entry `_exit` in the
/// handler bounds the damage to that one child).
#[cfg(unix)]
pub fn signal_kill_tracked_groups(sig: libc::c_int) {
    if let Ok(tracked) = TRACKED_CHILDREN.try_lock() {
        for c in tracked.iter() {
            let _ = unsafe { libc::kill(-c.pgid, sig) };
        }
    }
}

/// Drop-guard that tears down the whole spawned tree when it goes out of
/// scope — the exit fallback for any path that skips the explicit
/// [`teardown_process_tree`] call in `main` (panic, early return).
/// Idempotent with the explicit call (dead groups/pids are ESRCH no-ops).
#[must_use]
#[derive(Debug, Default)]
pub struct ProcessTreeGuard;

impl ProcessTreeGuard {
    pub fn new() -> Self {
        Self
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        teardown_process_tree();
    }
}

#[cfg(all(unix, test))]
mod tests {
    use super::*;

    /// Fork a child, have it call [`detach_process_group`], and assert the
    /// child now leads its own process group (`getpgid == getpid`).
    ///
    /// The child must never run the test harness (double-fork, double-assert
    /// would corrupt the parent's state), so it reports via its exit code.
    #[test]
    fn detach_process_group_makes_process_group_leader() {
        // SAFETY: fork/waitpid on a child we create; the child runs only
        // async-signal-safe libc calls and _exit()s immediately.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed: {}", std::io::Error::last_os_error());
        if pid == 0 {
            // Child: detach, verify, report via exit code.
            detach_process_group();
            let is_leader = is_process_group_leader();
            unsafe { libc::_exit(if is_leader { 0 } else { 1 }) };
        }
        let mut status = 0;
        let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
        assert_eq!(
            waited,
            pid,
            "waitpid failed: {}",
            std::io::Error::last_os_error()
        );
        assert!(libc::WIFEXITED(status), "child did not exit cleanly");
        assert_eq!(
            libc::WEXITSTATUS(status),
            0,
            "child is not the leader of its own process group after detach_process_group"
        );
    }

    /// The `/proc` ppid table must map a parent to a live child.
    #[cfg(target_os = "linux")]
    #[test]
    fn ppid_table_maps_parent_to_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let me = std::process::id();
        let mut found = false;
        for _ in 0..100 {
            if let Some(map) = ppid_children_table()
                && map.get(&me).is_some_and(|kids| kids.contains(&child.id()))
            {
                found = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(found, "ppid table did not map this process to its child");
    }

    /// The `collect_descendants` unit test builds a 2-level tree (sh → sleep)
    /// and checks the walk sees the grandchild — and nothing else.
    #[test]
    fn collect_descendants_finds_subtree_only() {
        let mut root = std::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 30 & wait")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let root_pid = root.id();
        // Wait for the grandchild to appear in the table.
        let mut leaf = None;
        for _ in 0..100 {
            if let Some(map) = ppid_children_table()
                && let Some(kids) = map.get(&root_pid)
                && let Some(&k) = kids.first()
            {
                leaf = Some(k);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let leaf = leaf.expect("grandchild never appeared");
        let desc = collect_descendants(&[root_pid]);
        assert!(
            desc.contains(&leaf),
            "descendants of {root_pid} = {desc:?}, expected to contain {leaf}"
        );
        assert!(
            !desc.contains(&root_pid),
            "roots must not be in their own descendants"
        );
        let _ = root.kill();
        let _ = root.wait();
    }
}
