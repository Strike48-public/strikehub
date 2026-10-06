//! Windows Job Object teardown (issue #115).
//!
//! Every child StrikeHub spawns on Windows is assigned to ONE shared Job
//! Object created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`:
//!
//! * on close, [`terminate_job`] calls `TerminateJobObject` — an
//!   OS-guaranteed kill of every process in the job (the whole tree,
//!   grandchildren included),
//! * on ANY other exit path (crash, `_exit`, external kill), the OS closes
//!   the job handle when the hub process exits and kills every member —
//!   the KILL_ON_JOB_CLOSE guarantee, which `kill_on_drop` (direct pid
//!   only) cannot provide.
//!
//! Assignment happens immediately after `CreateProcess` returns, inside
//! [`assign_pid_to_job`], called from `sh_core::process::spawn_tracked`
//! BEFORE any `await`. The residual window — the connector spawning a
//! grandchild between `CreateProcess` and the assignment — is
//! microseconds and does not occur in practice: the Windows repro of
//! issue #115 observed zero connector children over 90+ s of runtime, and
//! the connectors only spawn (PTY) children on user action. If the
//! assignment ever fails anyway, `kill_on_drop` plus the direct
//! `terminate_pid` passes in `teardown_process_tree` still cover the
//! direct child.

use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE, TerminateProcess,
};

/// A Job Object handle that may live in a process-wide static. The handle
/// is only ever used within this process (never passed to another process
/// or thread as data), which is exactly the guarantee `CreateJobObjectW`
/// provides — so `Send` is sound to assert.
struct SendJobHandle(HANDLE);

unsafe impl Send for SendJobHandle {}

/// The shared StrikeHub Job Object: created lazily, held open for the
/// process lifetime so KILL_ON_JOB_CLOSE stays armed on every exit path.
static JOB: OnceLock<Mutex<Option<SendJobHandle>>> = OnceLock::new();

fn job_handle() -> Option<HANDLE> {
    let slot = JOB.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().ok()?;
    if let Some(h) = guard.as_ref().map(|s| s.0) {
        return Some(h);
    }
    // The job name is cosmetic (task manager / job-object tooling).
    let name: Vec<u16> = "StrikeHub"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `name` is a NUL-terminated wide string; a null attribute
    // structure is the documented default.
    let h = unsafe { CreateJobObjectW(std::ptr::null(), name.as_ptr()) };
    if h == HANDLE::default() {
        tracing::warn!(
            error = %std::io::Error::last_os_error(),
            "CreateJobObjectW failed; Windows teardown falls back to kill_on_drop"
        );
        return None;
    }
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `h` is valid; `limits` outlives the call; the class and size
    // match `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`.
    let ok = unsafe {
        SetInformationJobObject(
            h,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        let err = std::io::Error::last_os_error();
        unsafe { CloseHandle(h) };
        tracing::warn!(
            error = %err,
            "SetInformationJobObject failed; Windows teardown falls back to kill_on_drop"
        );
        return None;
    }
    *guard = Some(SendJobHandle(h));
    tracing::info!("StrikeHub job object created (KILL_ON_JOB_CLOSE armed)");
    Some(h)
}

/// Assign a just-spawned pid to the shared Job Object. Returns `true` on
/// success.
#[must_use]
pub fn assign_pid_to_job(pid: u32) -> bool {
    let Some(job) = job_handle() else {
        return false;
    };
    // SAFETY: `pid` is a live pid we just spawned; the returned handle is
    // closed below on every path.
    let hproc = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION, 0, pid) };
    if hproc == HANDLE::default() {
        let err = std::io::Error::last_os_error();
        tracing::warn!(pid, %err, "could not open connector process for job assignment");
        return false;
    }
    // SAFETY: both handles are valid (above and `job_handle`).
    let ok = unsafe { AssignProcessToJobObject(job, hproc) };
    unsafe { CloseHandle(hproc) };
    if ok == 0 {
        // A Job Object can only accept a process that has not yet spawned
        // children of its own. Backstops remain: kill_on_drop (direct
        // child) + the direct-terminate pass in teardown_process_tree.
        let err = std::io::Error::last_os_error();
        tracing::warn!(
            pid,
            %err,
            "AssignProcessToJobObject failed; falling back to kill_on_drop + direct teardown"
        );
        return false;
    }
    tracing::info!(pid, "connector assigned to StrikeHub job object");
    true
}

/// Kill every process in the job (the whole tree — OS-guaranteed).
pub fn terminate_job() {
    let Some(slot) = JOB.get() else {
        return;
    };
    let Ok(guard) = slot.lock() else {
        return;
    };
    let Some(h) = guard.as_ref().map(|s| s.0) else {
        return;
    };
    // SAFETY: `h` is the job handle stored by `job_handle`.
    if unsafe { TerminateJobObject(h, 1) } == 0 {
        tracing::warn!(
            error = %std::io::Error::last_os_error(),
            "TerminateJobObject failed; direct-terminate pass follows"
        );
    }
    // NOTE: the handle is deliberately kept open — KILL_ON_JOB_CLOSE must
    // stay armed for anything assigned after this call, and the OS closes
    // it (killing stragglers) when the hub exits.
}

/// Terminate one pid directly (backstop for a child not in the job).
pub fn terminate_pid(pid: u32) {
    // SAFETY: `pid` is a tracked child pid; handle closed below.
    let h = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_INFORMATION, 0, pid) };
    if h == HANDLE::default() {
        return; // already gone
    }
    unsafe {
        TerminateProcess(h, 1);
        CloseHandle(h);
    }
}

/// True if `pid` is a live process (used by the teardown poll loop).
#[must_use]
pub fn pid_alive(pid: u32) -> bool {
    // SAFETY: handle closed below on every path.
    let h = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid) };
    if h == HANDLE::default() {
        return false;
    }
    unsafe { CloseHandle(h) };
    true
}

#[cfg(test)]
mod tests {
    // Runtime tests for the Job Object teardown — exercised by the CI
    // `Check (Windows)` job (`cargo test -p sh-core --target
    // x86_64-pc-windows-msvc job`). No mocks: every test drives the real
    // Windows API through the production functions above (`CreateJobObjectW`
    // / `SetInformationJobObject(KILL_ON_CLOSE)` / `AssignProcessToJobObject`
    // / `TerminateJobObject`), with real child processes.
    use super::*;

    /// Spawn a long-lived leaf process (no children of its own, so the
    /// assignment is deterministic — `AssignProcessToJobObject` refuses a
    /// process that has already spawned children).
    fn spawn_long_lived_leaf() -> std::process::Child {
        std::process::Command::new("ping")
            .args(["-n", "120", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn ping")
    }

    fn wait_for_death(pid: u32, secs: u64) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        while std::time::Instant::now() < deadline {
            if !pid_alive(pid) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        !pid_alive(pid)
    }

    /// End-to-end: the lazy job creation (KILL_ON_CLOSE armed) +
    /// `assign_pid_to_job` + `terminate_job` must kill the assigned process.
    #[test]
    fn terminate_job_kills_assigned_process() {
        let mut child = spawn_long_lived_leaf();
        let pid = child.id();
        // Sanity: the child is actually alive before we touch the job.
        assert!(pid_alive(pid), "child died before assignment");
        assert!(
            assign_pid_to_job(pid),
            "assignment failed — job creation or OpenProcess/Assign broke"
        );
        terminate_job();
        assert!(
            wait_for_death(pid, 5),
            "TerminateJobObject did not kill the assigned process within 5 s"
        );
        let _ = child.wait();
    }

    /// Error path: assigning a pid that no longer exists must fail cleanly
    /// (OpenProcess error) and return `false` — never panic.
    #[test]
    fn assign_pid_to_job_rejects_dead_pid() {
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "exit", "0"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn cmd");
        let pid = child.id();
        child.wait().expect("wait for cmd to exit");
        assert!(
            !assign_pid_to_job(pid),
            "assign_pid_to_job must return false for a dead pid (OpenProcess failure path)"
        );
    }

    /// `terminate_job` must be safe to call any number of times (empty job,
    /// populated job, concurrent teardown passes) — it must never panic,
    /// and a second call after a kill is a no-op.
    #[test]
    fn terminate_job_is_idempotent_and_safe() {
        terminate_job();
        let mut child = spawn_long_lived_leaf();
        let pid = child.id();
        if assign_pid_to_job(pid) {
            terminate_job();
            assert!(wait_for_death(pid, 5), "child survived terminate_job");
        }
        terminate_job(); // no-op: nothing left in the job (or empty job)
        let _ = child.wait();
    }
}
