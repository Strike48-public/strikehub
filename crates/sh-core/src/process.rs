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
//!    ([`collect_descendants`] — `/proc` on Linux, `ps` on macOS),
//! 4. as a fallback for processes that are neither tracked nor reachable
//!    from a tracked pid — a self-update-respawned connector running from
//!    our managed roots (`~/.strike48/strikehub/bin/`, the app bundle's
//!    `MacOS` dir on macOS) whose tracked ancestor already died, so the
//!    group is empty and the walk has no live anchor — a bounded
//!    MANAGED-ROOT SWEEP: every process whose RESOLVED executable path
//!    lives under one of those roots is collected the same way
//!    ([`sweep_managed_roots`]) — TOGETHER WITH ITS FULL DESCENDANT
//!    SUBTREE: the matched pids are run through [`collect_descendants`]
//!    before the TERM pass, because a matched root's children/grandchildren
//!    exec binaries OUTSIDE the roots (the path whitelist is blind to
//!    them) and the issue's AC is zero descendants (children AND
//!    grandchildren). The path-prefix whitelist stays the safety property:
//!    nothing else (argv, env, names) is ever matched.
//!
//! # Exit-route coverage (issue #115 RC remainder)
//!
//! dioxus 0.6.3's `launch()` blocks in tao's `EventLoop::run`, and tao
//! 0.30.8's `run` is DIVERGING on every platform: when the OS run loop
//! ends (last window closed, or on macOS the application-terminate event
//! — AppleEvent quit via `osascript`, Cmd-Q, dock Quit — through
//! `applicationShouldTerminate`), tao calls `std::process::exit` directly.
//! That skips the code after `launch()` in `sh-ui/src/main.rs` AND every
//! Rust Drop impl (`ProcessTreeGuard`, `kill_on_drop`) — which is exactly
//! how the RC verification REPRO orphaned a self-updated `pentest-agent`
//! on a clean `osascript quit`. So teardown is additionally delivered by a
//! C-level `atexit` hook ([`install_exit_handler`], unix) — `process::exit`
//! runs `atexit` handlers — idempotent with the explicit call and with the
//! signal-handler path (dead pids/groups are ESRCH no-ops).
//!
//! On Windows the same guarantee comes from one shared Job Object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (see [`crate::job`]): every child
//! is assigned at spawn, the whole job is terminated on close, and the OS
//! kills the job when the hub's handle closes on any exit path.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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

// ── Managed-root fallback sweep (issue #115 RC remainder) ────────────────
//
// The RC verification (issue #115) caught a self-update-respawned
// `pentest-agent` running from `~/.strike48/strikehub/bin/` survive a
// clean quit: when the tracked original dies during the ~30 s
// self-update and the successor detaches (setsid) and is reparented to
// init/launchd, neither the group kill (the tracked group is empty) nor
// the descendant walk (anchor pids dead, successor's ppid no longer
// tracked) can see it. The sweep below closes that hole with a
// PATH-PREFIX WHITELIST: it only ever matches processes whose RESOLVED
// executable path lives under a directory StrikeHub itself manages.

/// Bound on how many system pids the sweep inspects (normal desktops are
/// far below this; the bound keeps a pathological system from stalling
/// teardown).
const MAX_SWEEP_PIDS: usize = 4096;

/// Bound on how many processes the sweep can return.
const MAX_SWEEP_TARGETS: usize = 64;

/// Extra roots for the fallback sweep, registered by tests.
/// Production startup registers none: [`managed_roots`] already covers
/// every location StrikeHub manages executables in.
#[cfg(unix)]
static EXTRA_MANAGED_ROOTS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Register an additional root for the fallback sweep.
///
/// Intended for tests: the production sweep's root is the USER's
/// `~/.strike48/strikehub/bin` (via [`managed_roots`]), which a test must
/// not treat as disposable. The path-prefix whitelist in [`is_managed_exe`]
/// is what keeps pointing the sweep at a test directory safe.
#[cfg(unix)]
pub fn add_managed_root(p: PathBuf) {
    if let Ok(mut v) = EXTRA_MANAGED_ROOTS.lock()
        && !v.contains(&p)
    {
        v.push(p);
    }
}

/// The directory trees StrikeHub owns executables in:
///
/// * `~/.strike48/strikehub/bin/` — the self-update/fetch cache; after the
///   ~30 s self-update the LIVE connector generation runs from here (RC
///   evidence, issue #115: `pentest-agent` spawned from
///   `~/.strike48/strikehub/bin/pentest-agent`),
/// * (macOS) the app bundle's `Contents/MacOS/` directory when running
///   from a bundle — bundled connector siblings live there
///   (`connector_seed::bundled_binary_path`).
///
/// Deliberately NOT included: the running executable's directory on
/// non-bundle layouts (a dev `target/debug` tree is not "managed" — the
/// prefix whitelist must stay narrow), and anything merely on a PATH.
///
/// Every root is CANONICALIZED (symlinks resolved): the matcher is fed
/// RESOLVED exe paths (see [`process_exe_path`]), so a `$HOME` — or the
/// bundle location — containing a symlink component must resolve
/// identically, or the whitelist would silently never match on such hosts
/// (fail-safe under-kill: the RC scenario stays open). A root that does
/// not exist yet (first run, before the first self-update) cannot be
/// canonicalized and keeps its raw path as the fallback.
#[cfg(unix)]
#[must_use]
pub fn managed_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = vec![crate::connector_fetch::bin_cache_dir()];
    if let Some(exe) = std::env::current_exe().ok().as_deref()
        && let Some(macos_dir) = exe.parent()
    {
        // macOS bundle layout: <...>/StrikeHub.app/Contents/MacOS/<exe>
        let is_bundle_macos = macos_dir.file_name().is_some_and(|n| n == "MacOS")
            && macos_dir
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|n| n == "Contents");
        if is_bundle_macos {
            roots.push(macos_dir.to_path_buf());
        }
    }
    if let Ok(extra) = EXTRA_MANAGED_ROOTS.lock() {
        roots.extend(extra.iter().cloned());
    }
    roots.dedup();
    roots
        .into_iter()
        .map(|r| std::fs::canonicalize(&r).unwrap_or(r))
        .collect()
}

/// Strict component-wise prefix match: `path` must live UNDER one of
/// `roots`. Lookalikes (`…/bin2/…`, `…/bin-evil/…`, a different user's
/// home) are rejected because `Path::starts_with` compares whole path
/// components. Symlink escapes are rejected by construction: callers pass
/// the RESOLVED exe path (see [`process_exe_path`]), so a process whose
/// real executable lives outside `roots` never matches, however it was
/// launched.
#[cfg(unix)]
#[must_use]
pub fn is_managed_exe(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|r| path.starts_with(r))
}

/// The resolved executable path of `pid` — symlinks resolved to the actual
/// file the kernel exec'd:
///
/// * Linux — `readlink /proc/<pid>/exe` (the kernel gives the resolved
///   target; a script reports its interpreter),
/// * macOS — `proc_pidpath(3)`.
///
/// `None` when the process vanished mid-scan or the OS cannot report the
/// path (zombies, kernel threads).
#[cfg(unix)]
fn process_exe_path(pid: u32) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/exe")).ok()
    }
    #[cfg(target_os = "macos")]
    {
        let mut buf = vec![0u8; 4096];
        // SAFETY: proc_pidpath writes at most `buffersize` bytes into the
        // caller buffer and returns the required length; `pid` is a live
        // pid from our own enumeration moments earlier (a dead pid just
        // yields 0/ENOENT and is skipped).
        let n = unsafe {
            libc::proc_pidpath(
                pid as libc::c_int,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                buf.len() as u32,
            )
        };
        if n <= 0 {
            return None;
        }
        let len = (n as usize).min(buf.len());
        buf.truncate(len);
        let s = String::from_utf8_lossy(&buf);
        Some(PathBuf::from(s.as_ref()))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

/// Every pid on the system, bounded to the first [`MAX_SWEEP_PIDS`] (Linux:
/// `/proc` readdir order; macOS: `kern.proc.pid` order).
#[cfg(unix)]
fn all_pids_bounded() -> Vec<u32> {
    let mut pids: Vec<u32> = Vec::new();
    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for e in entries.flatten() {
                if let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() {
                    pids.push(pid);
                    if pids.len() >= MAX_SWEEP_PIDS {
                        break;
                    }
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        // kern.proc.pid returns a NUL-separated list of pids (no KINFO
        // structs) — the same enumeration `ps` uses.
        let name = b"kern.proc.pid\0";
        let mut size: usize = 0;
        // SAFETY: sysctlbyname with a valid NUL-terminated name; the size
        // probe (oldp = null) only reads the required length, and the
        // second call fills a buffer of exactly that size.
        let rc = unsafe {
            libc::sysctlbyname(
                name.as_ptr() as *const libc::c_char,
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc == 0 && size > 0 {
            let mut buf = vec![0i8; size];
            let rc = unsafe {
                libc::sysctlbyname(
                    name.as_ptr() as *const libc::c_char,
                    buf.as_mut_ptr() as *mut std::ffi::c_void,
                    &mut size,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if rc == 0 {
                for chunk in buf[..size].chunks_exact(4) {
                    let pid = i32::from_ne_bytes([
                        chunk[0] as u8,
                        chunk[1] as u8,
                        chunk[2] as u8,
                        chunk[3] as u8,
                    ]);
                    if pid > 0 {
                        pids.push(pid as u32);
                        if pids.len() >= MAX_SWEEP_PIDS {
                            break;
                        }
                    }
                }
            }
        }
    }
    pids
}

/// The fallback sweep (issue #115 RC remainder): every process whose
/// RESOLVED executable path lives under one of `roots`.
///
/// Catches a self-update-respawned connector that
///
/// * was not spawned through [`spawn_tracked`] (the successor is created
///   by the connector's own self-update code, not by StrikeHub), and/or
/// * has been reparented to init/launchd (ppid = 1 or a subreaper) after
///   its tracked ancestor died — so the group kill (empty group) and the
///   descendant walk (no live tracked anchor) both miss it.
///
/// Returns only the processes whose OWN resolved exe matches a root.
/// [`teardown_process_tree`] extends each match with its FULL bounded
/// descendant set (via [`collect_descendants`]) before the TERM pass: a
/// matched root's descendants run arbitrary executables OUTSIDE the roots
/// and are invisible to the whitelist itself — the AC (issue #115) is
/// zero descendants, children AND grandchildren.
///
/// Safety: ONLY the path-prefix whitelist above matches — resolved paths
/// under `roots`, self excluded, enumeration and result bounded. Nothing
/// else (argv, env, process names) is ever considered. NOT for use in
/// signal handlers (it reads /proc; on macOS it calls `sysctlbyname`).
#[cfg(unix)]
#[must_use]
pub fn sweep_managed_roots(roots: &[PathBuf]) -> Vec<u32> {
    let me = std::process::id();
    let mut out: Vec<u32> = Vec::new();
    for pid in all_pids_bounded() {
        if pid == me {
            continue;
        }
        let Some(exe) = process_exe_path(pid) else {
            continue;
        };
        if is_managed_exe(&exe, roots) {
            out.push(pid);
            if out.len() >= MAX_SWEEP_TARGETS {
                break;
            }
        }
    }
    out
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
/// 3. Managed-root sweep: collect every process whose resolved exe lives
///    under a managed root, PLUS each match's full bounded descendant
///    subtree (a matched root's children/grandchildren exec binaries
///    OUTSIDE the roots — whitelist-blind, walk-only coverage; AC is zero
///    descendants, children AND grandchildren).
/// 4. `SIGTERM` every tracked process group (`kill(-pgid)`) and every
///    collected pid (Windows: `TerminateJobObject` + direct terminate).
/// 5. Poll for up to [`TEARDOWN_GRACE_SECS`] (~2 s) for the whole set to
///    die; return early when it does.
/// 6. Escalate survivors to `SIGKILL` (Windows: terminate again).
///
/// Synchronous and idempotent — safe to call from `main` after the window
/// closes, from the [`ProcessTreeGuard`] exit fallback, from the atexit
/// hook, or all of the above. NOT signal-handler-safe (it sleeps, reads
/// /proc, and on macOS calls `sysctlbyname`): the signal handler uses
/// [`signal_kill_tracked_groups`] instead.
pub fn teardown_process_tree() {
    let tracked = tracked_children_snapshot();

    let pids: Vec<u32> = tracked.iter().map(|c| c.pid).collect();
    let mut targets: Vec<u32> = pids.clone();
    for d in collect_descendants(&pids) {
        if !targets.contains(&d) {
            targets.push(d);
        }
    }
    // Fallback sweep (issue #115 RC remainder): self-update-respawned
    // processes run from our managed roots and may be BOTH untracked
    // (spawned by the connector's own self-update code, not by StrikeHub)
    // and reparented to init/launchd (tracked ancestor dead) — the group
    // kill and the descendant walk both miss them. Runs even when the
    // registry is empty: that is precisely the case where every tracked
    // anchor is gone and only the path whitelist can still find them.
    //
    // The sweep matches ROOT processes (resolved exe under a managed
    // root). Their OWN descendants run arbitrary executables OUTSIDE the
    // roots (shells, helpers — whitelist-blind) and, being untracked,
    // are invisible to the walk in step 2 as well. The AC (issue #115)
    // is ZERO descendants — children AND grandchildren — so the matched
    // pids are run through the same bounded descendant walk before the
    // TERM pass (review of #121, CQ concern): TERM → grace → KILL then
    // covers each matched root's full subtree. The walk is bounded
    // (MAX_WALK_NODES) and rooted ONLY at whitelist-matched pids.
    #[cfg(unix)]
    {
        let swept: Vec<u32> = sweep_managed_roots(&managed_roots());
        for s in &swept {
            if !targets.contains(s) {
                targets.push(*s);
            }
        }
        if !swept.is_empty() {
            for d in collect_descendants(&swept) {
                if !targets.contains(&d) {
                    targets.push(d);
                }
            }
        }
    }

    if targets.is_empty() {
        return;
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
///
/// NOTE: a Drop impl never runs for a `std::process::exit` death — and
/// that is exactly how tao's `EventLoop::run` ends the process on every
/// GUI close/quit route (see the module docs). The atexit hook
/// ([`install_exit_handler`]) covers that route; this guard covers panics
/// and early returns.
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

// ── atexit hook (issue #115 RC remainder — the quit-event route) ─────────
//
// tao 0.30.8's `EventLoop::run` — which dioxus 0.6.3's `launch()` blocks
// in — is DIVERGING on every platform: when the OS run loop ends (last
// window closed, or on macOS the application-terminate event — AppleEvent
// quit via `osascript`, Cmd-Q, dock Quit — routed through
// `applicationShouldTerminate`), tao calls `std::process::exit` DIRECTLY.
// That runs C-level `atexit` handlers but NONE of the Rust Drop impls
// (`ProcessTreeGuard`, `kill_on_drop`) and none of the code after
// `launch()` in `sh-ui/src/main.rs`. The hook below is therefore what
// actually delivers `teardown_process_tree()` on the quit-event route —
// the exact route the RC verification REPRO orphaned a self-updated
// `pentest-agent` on. Idempotent with the explicit call and the
// signal-handler path (dead pids/groups are ESRCH no-ops).

/// The exact teardown entry the atexit hook runs. Public so tests can
/// invoke the quit-route entry point directly without waiting for
/// `process::exit`. Declared `extern "C"` because it is registered with
/// the C `atexit` mechanism (its Rust-calling wrapper is
/// [`teardown_process_tree`]).
#[cfg(unix)]
pub extern "C" fn atexit_teardown() {
    teardown_process_tree();
}

/// Install the C-level `atexit` hook that runs [`atexit_teardown`] when
/// the process exits via `std::process::exit` — the exit used by tao's
/// `EventLoop::run` on EVERY GUI exit route (window close, macOS
/// quit-event). Call once at startup, before the first child is spawned.
/// Idempotent.
#[cfg(unix)]
pub fn install_exit_handler() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        // SAFETY: atexit_teardown is a valid `extern "C" fn` pointer;
        // atexit only stores it. The handler is re-entrancy-safe (teardown
        // is idempotent) and runs no async code.
        unsafe {
            let _ = libc::atexit(atexit_teardown);
        }
    });
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

    /// The sweep matcher must accept ONLY paths under the whitelisted
    /// roots, component-wise. Adversarial lookalikes — a suffixed root
    /// directory (`bin2`, `bin-evil`), a different user's home, an
    /// unrelated tree — are all rejected. (Symlink escapes are rejected
    /// by construction: the matcher is only ever fed RESOLVED exe paths,
    /// and the integration test `sweep_rejects_symlink_escape_outside_roots`
    /// proves a symlinked launch from outside the roots survives.)
    #[test]
    fn managed_root_matcher_accepts_only_whitelisted_prefixes() {
        let root = PathBuf::from("/home/u/.strike48/strikehub/bin");
        let roots = vec![root.clone()];

        // In-whitelist: direct child and nested paths.
        assert!(is_managed_exe(&root.join("pentest-agent"), &roots));
        assert!(is_managed_exe(&root.join("ks-connector"), &roots));
        assert!(is_managed_exe(
            &root.join("sub").join("ks-connector"),
            &roots
        ));

        // Adversarial lookalikes with a suffix on the root component.
        assert!(!is_managed_exe(
            Path::new("/home/u/.strike48/strikehub/bin2/pentest-agent"),
            &roots
        ));
        assert!(!is_managed_exe(
            Path::new("/home/u/.strike48/strikehub/bin-evil/pentest-agent"),
            &roots
        ));
        assert!(!is_managed_exe(
            Path::new("/home/u/.strike48/strikehub/binz/x"),
            &roots
        ));

        // Same relative path under a DIFFERENT user's home.
        assert!(!is_managed_exe(
            Path::new("/home/u2/.strike48/strikehub/bin/pentest-agent"),
            &roots
        ));

        // Unrelated trees.
        assert!(!is_managed_exe(Path::new("/usr/bin/pentest-agent"), &roots));
        assert!(!is_managed_exe(
            Path::new("/opt/strikehub/pentest-agent"),
            &roots
        ));

        // A second root is additive, not a prefix shortcut.
        let roots2 = vec![
            root,
            PathBuf::from("/Applications/StrikeHub.app/Contents/MacOS"),
        ];
        assert!(is_managed_exe(
            Path::new("/Applications/StrikeHub.app/Contents/MacOS/ks-connector"),
            &roots2
        ));
        assert!(!is_managed_exe(
            Path::new("/Applications/StrikeHub.app/Contents/MacOS2/x"),
            &roots2
        ));
    }

    /// The sweep's default root must include the self-update cache — the
    /// directory the RC verification showed the orphaned agent running
    /// from (`~/.strike48/strikehub/bin/pentest-agent`).
    ///
    /// `managed_roots()` canonicalizes (the matcher is fed RESOLVED exe
    /// paths), so compare in canonical form: a not-yet-existing cache dir
    /// is its own canonical fallback.
    #[test]
    fn managed_roots_include_the_self_update_cache() {
        let cache = crate::connector_fetch::bin_cache_dir();
        let expected = std::fs::canonicalize(&cache).unwrap_or(cache);
        let roots = managed_roots();
        assert!(
            roots.contains(&expected),
            "managed roots {roots:?} must include the self-update cache ({expected:?})"
        );
    }

    /// The sweep matcher is fed RESOLVED exe paths — a registered root
    /// whose path contains a SYMLINK (a `$HOME` under a symlinked home,
    /// a bundle under a symlinked mount point) must come back
    /// CANONICALIZED from [`managed_roots`], or the whitelist silently
    /// never matches on such hosts (review of #121, CQ nit).
    #[test]
    fn managed_roots_canonicalize_symlinked_roots() {
        let base = std::env::temp_dir().join(format!(
            "strikehub-canonical-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = base.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        // A SYMLINKED registration resolves to the real directory…
        add_managed_root(link.clone());
        let roots = managed_roots();
        assert!(
            roots.contains(&real),
            "managed roots {roots:?} must contain the canonical form of the symlinked root {link:?} (={real:?})"
        );
        // …and the matcher then accepts a RESOLVED exe path under it.
        assert!(
            is_managed_exe(&real.join("pentest-agent"), &roots),
            "a resolved exe path under the canonical root must match"
        );
        // A not-yet-existing root keeps its raw path (canonicalize
        // fallback — first run, before the first self-update).
        let missing = base.join("missing");
        add_managed_root(missing.clone());
        let roots2 = managed_roots();
        assert!(
            roots2.contains(&missing),
            "a non-existent root must keep its raw path (canonicalize fallback): got {roots2:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// macOS leg of the sweep (kern.proc.pid enumeration + proc_pidpath
    /// resolution): this process must enumerate itself and resolve its
    /// own executable path to the canonical current_exe. Runtime-verified
    /// on the mac lab box (RC QA); here it is a compile-time proof that
    /// the macOS code paths build.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_pid_enumeration_and_exe_resolution_see_self() {
        let me = std::process::id();
        assert!(
            all_pids_bounded().contains(&me),
            "kern.proc.pid enumeration must see this process"
        );
        let exe = process_exe_path(me).expect("proc_pidpath must resolve this process");
        let canonical = std::env::current_exe()
            .ok()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .unwrap();
        assert_eq!(
            exe, canonical,
            "proc_pidpath({me}) = {exe:?}, expected {canonical:?}"
        );
    }

    /// The atexit entry (the EXACT function the C-level atexit hook runs
    /// when tao's `process::exit` ends the process on the quit-event
    /// route) must be callable and must be a no-op — not an error — when
    /// there is nothing tracked and nothing under the managed roots.
    ///
    /// Self-guarded: if this lib test binary ever gains a parallel test
    /// that owns tracked processes (or a live agent runs from the
    /// managed root on a dev box), the call would CORRECTLY kill them —
    /// so skip in that case instead of racing.
    #[test]
    fn atexit_teardown_is_a_safe_noop_when_idle() {
        if !tracked_children_snapshot().is_empty()
            || !sweep_managed_roots(&managed_roots()).is_empty()
        {
            eprintln!("atexit no-op test skipped: registry or managed root not idle");
            return;
        }
        atexit_teardown();
        // Reaching here without a panic is the assertion.
    }
}
