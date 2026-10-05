//! Process-group management helpers (Unix).
//!
//! The desktop app terminates its child connector processes by signalling
//! the process group it leads. That is only safe if StrikeHub actually
//! LEADS its own group — which is not true when the app is launched from a
//! desktop launcher (GNOME app grid, dock, file manager, ...): the
//! launcher's spawn leaves the child in the LAUNCHER's process group.
//! Signalling that inherited group would SIGTERM the launcher itself (e.g.
//! GNOME Shell) and every other member of that group —
//! project-management#380 (377 finding 7), HIGH.
//!
//! The fix is to call [`detach_process_group`] early at startup, before any
//! child is spawned. Everything StrikeHub launches afterwards (connectors,
//! helpers) inherits the private group, so a `killpg(getpgrp(), ...)` on
//! shutdown reaches only StrikeHub and its own children.

/// Put the calling process in a new process group of which it is the leader.
///
/// Must be called early at startup, BEFORE any child process is spawned, so
/// that everything StrikeHub launches inherits the private group.
///
/// Failure is not fatal: when we are already a session leader (e.g. launched
/// under `setsid`) `setpgid(0, 0)` returns `EPERM`, but we already own a
/// process group and nothing needs to be done. Callers that rely on the
/// group being private (the shutdown `killpg` path) additionally verify
/// group-leadership before signalling.
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
}
