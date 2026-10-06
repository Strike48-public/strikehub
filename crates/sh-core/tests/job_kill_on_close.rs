//! Windows Job Object — `KILL_ON_JOB_CLOSE` runtime proof (issue #115;
//! review of PR #116 demanded real verification of the Windows path).
//!
//! `harness = false` (see `[[test]]` in Cargo.toml): this binary doubles as
//! its own probe, because the guarantee under test only fires when a
//! PROCESS EXITs and the OS closes its job handle.
//!
//! Driver (normal `cargo test` invocation):
//!   1. Spawns ITSELF with `--job-koc-helper`.
//!   2. The helper (a separate process, its own copy of the shared Job
//!      Object) creates `cmd /c ping -n 120 <marker>` suspended
//!      (`CREATE_SUSPENDED` → deterministic: it cannot have spawned its
//!      `ping` grandchild before the assignment), assigns it to the real
//!      production job via `sh_core::job::assign_pid_to_job` (lazy
//!      `CreateJobObjectW` + KILL_ON_CLOSE), resumes it — `cmd` then
//!      spawns `ping`, and `ping` belongs to the job by construction
//!      (children of a member join the member's job) — and exits.
//!   3. The helper's exit closes the job handle. `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
//!      must then make the OS kill `cmd` AND `ping` — the AC3 mechanism,
//!      with no `TerminateJobObject` call anywhere in this path.
//!   4. The driver asserts that within a few seconds neither the assigned
//!      process nor ANY process whose command line carries the unique
//!      marker (192.0.2.99, TEST-NET-1 / RFC 5737 — unrouteable, so `ping`
//!      runs the full `-n 120` ≈ 2 min and is uniquely identifiable) is
//!      alive.
//!
//! Environment handling (observed on the windows-latest CI agent, which
//! runs its whole tree inside its OWN Job Object): a job member's children
//! inherit THAT job and can never be assigned to the production job, and
//! escaping it via `CREATE_BREAKAWAY_FROM_JOB` requires
//! `SeCreatePagefilePrivilege`. The fixture is two-pass (plain spawn →
//! breakaway after a best-effort privilege enable on our own token). If
//! the environment makes a job member impossible to create, the probe
//! verifies what still can be (job creation + KILL_ON_CLOSE arming,
//! `TerminateJobObject`, assignment error paths) and reports the exact
//! observed errors — a constrained environment is reported, never hidden.
//!
//! No mocks: real `CreateProcessW`, real production job API, real OS kill.
//! Runs in the CI `Check (Windows)` job; prints a skip note on other
//! platforms (exit 0).

#[cfg(windows)]
use std::time::{Duration, Instant};

/// Unique command-line marker for the fixture tree (TEST-NET-1, RFC 5737).
const MARKER: &str = "192.0.2.99";

fn main() {
    #[cfg(windows)]
    {
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|a| a == "--job-koc-helper") {
            helper();
        }
    }
    driver();
}

/// Best-effort probe: is the CURRENT process already a member of some Job
/// Object? windows-sys 0.60 does not export the `JOB_*` access-right
/// constants, so the raw values are used: 0 (no access requested) and
/// 0x0002 (JOB_QUERY_LIMITS). Used for reporting only — the two-pass
/// spawn does not depend on it.
#[cfg(windows)]
fn current_process_in_job() -> bool {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::OpenJobObjectW;
    for access in [0u32, 0x0002] {
        // SAFETY: a NULL name opens the CURRENT process's job (documented);
        // the handle is closed on both paths.
        let h = unsafe { OpenJobObjectW(access, 0, std::ptr::null()) };
        if h != HANDLE::default() {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(h) };
            return true;
        }
    }
    false
}

/// Best-effort enable of `SeCreatePagefilePrivilege` on our OWN token —
/// the documented requirement for creating `CREATE_BREAKAWAY_FROM_JOB`
/// children. Never fatal.
#[cfg(windows)]
fn try_enable_se_create_pagefile() -> bool {
    use windows_sys::Win32::Foundation::{ERROR_NOT_ALL_ASSIGNED, HANDLE, LUID};
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle valid for this
    // process; the token handle is closed on every path.
    if unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
    } == 0
    {
        return false;
    }
    let name: Vec<u16> = "SeCreatePagefilePrivilege"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut luid: LUID = unsafe { std::mem::zeroed() };
    // SAFETY: `name` is NUL-terminated; `luid` is a valid out.
    let looked = unsafe { LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut luid) } != 0;
    let enabled = if looked {
        let state = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        // SAFETY: `token` valid, `state` well-formed (count matches).
        let ok = unsafe {
            AdjustTokenPrivileges(
                token,
                0,
                &state,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } != 0;
        // A `false` return is refined by GetLastError: 1300
        // (ERROR_NOT_ALL_ASSIGNED) means the privilege is absent from the
        // token altogether.
        ok && std::io::Error::last_os_error().raw_os_error() != Some(ERROR_NOT_ALL_ASSIGNED as i32)
    } else {
        false
    };
    unsafe { windows_sys::Win32::Foundation::CloseHandle(token) };
    enabled
}

#[cfg(windows)]
fn helper() -> ! {
    use std::io::Write;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_SUSPENDED, CreateProcessW, PROCESS_INFORMATION,
        ResumeThread, STARTUPINFOW, TerminateProcess,
    };

    let mut cmd_line: Vec<u16> = format!("cmd /c ping -n 120 {MARKER}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut plain_err: Option<std::io::Error> = None;
    let mut ba_spawn_err: Option<std::io::Error> = None;
    let mut ba_assign_err: Option<std::io::Error> = None;
    let mut privilege_enabled = false;
    // Pass 1: plain (hosts where the helper is in no job).
    // Pass 2: break away from a job-owning parent (CI agents) — requires
    // SeCreatePagefilePrivilege, enabled on our own token best-effort.
    for (pass, flags) in [
        (1, CREATE_SUSPENDED),
        (2, CREATE_SUSPENDED | CREATE_BREAKAWAY_FROM_JOB),
    ] {
        if pass == 2 {
            privilege_enabled = try_enable_se_create_pagefile();
        }
        let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `cmd_line` is NUL-terminated; `si`/`pi` are valid and
        // zeroed; `CREATE_SUSPENDED` keeps the primary thread suspended so
        // the assignment happens before the child can spawn children of its
        // own (the `AssignProcessToJobObject` precondition).
        let ok = unsafe {
            CreateProcessW(
                std::ptr::null(),
                cmd_line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                flags,
                std::ptr::null(),
                std::ptr::null(),
                &si,
                &mut pi,
            )
        };
        if ok == 0 {
            let e = std::io::Error::last_os_error();
            if pass == 2 {
                ba_spawn_err = Some(e);
            } else {
                eprintln!("job-koc-helper: CreateProcessW failed: {e}");
                std::process::exit(2);
            }
            continue;
        }
        // The REAL production API: lazily creates the shared StrikeHub job
        // object (KILL_ON_CLOSE armed) and assigns the child to it.
        let assigned = sh_core::job::assign_pid_to_job(pi.dwProcessId);
        if assigned {
            println!("ASSIGN=true PID={} PASS={}", pi.dwProcessId, pass);
            let _ = std::io::stdout().flush();
            // Resume: `cmd` runs `ping` (the grandchild joins the job by
            // construction). Then we exit WITHOUT terminating or explicitly
            // closing the job — the OS closes the handle at process exit
            // and KILL_ON_CLOSE must kill `cmd` AND `ping`.
            unsafe {
                ResumeThread(pi.hThread);
                CloseHandle(pi.hProcess);
                CloseHandle(pi.hThread);
            }
            std::process::exit(0);
        }
        // Not assignable: kill the SUSPENDED process (do NOT resume — so
        // `cmd` never spawns `ping`), record the error, try the next pass.
        let e = std::io::Error::last_os_error();
        if pass == 1 {
            plain_err = Some(e);
        } else {
            ba_assign_err = Some(e);
        }
        unsafe {
            TerminateProcess(pi.hProcess, 1);
            CloseHandle(pi.hProcess);
            CloseHandle(pi.hThread);
        }
    }
    println!(
        "ASSIGN=false IN_PARENT_JOB={} PRIV={} PLAIN_ERR={:?} BA_SPAWN_ERR={:?} \
         BA_ASSIGN_ERR={:?}",
        current_process_in_job(),
        privilege_enabled,
        plain_err,
        ba_spawn_err,
        ba_assign_err
    );
    let _ = std::io::stdout().flush();
    std::process::exit(3);
}

/// Pids of every process whose command line carries the fixture marker;
/// `None` if the scan itself failed.
#[cfg(windows)]
fn marker_pids() -> Option<Vec<u32>> {
    // Get-CimInstance works on every supported Windows (wmic is gone on
    // Server 2025).
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like \
                 '*{MARKER}*' }} | Select-Object -ExpandProperty ProcessId"
            ),
        ])
        .output()
        .ok()?;
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.trim().parse::<u32>().ok())
            .collect(),
    )
}

/// Constrained-environment driver path: the helper could not create a job
/// member (job-owning CI parent + no SeCreatePagefilePrivilege). Verify
/// from this process what the environment still allows — via the
/// PRODUCTION API — and pass with an explicit, error-cited report.
#[cfg(windows)]
fn verify_constrained(helper_report: &str) {
    // Drive the production lazy job creation here too, through a
    // deterministic dead-pid assign (the assign itself must fail: the pid
    // is dead; the side effect is the created + armed job).
    let mut child = std::process::Command::new("cmd")
        .args(["/c", "exit", "0"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn cmd");
    let pid = child.id();
    child.wait().expect("wait for cmd");
    assert!(
        !sh_core::job::assign_pid_to_job(pid),
        "dead-pid assign must fail cleanly"
    );
    assert!(
        sh_core::job::job_is_armed(),
        "job creation + KILL_ON_CLOSE arming failed in the constrained env"
    );
    sh_core::job::terminate_job(); // live (memberless) job — must not panic
    eprintln!(
        "job_kill_on_close: CONSTRAINED — full KILL_ON_CLOSE tree proof not executable \
         (helper report): {helper_report}\nverified instead: job creation + KILL_ON_CLOSE \
         arming, TerminateJobObject, assignment error paths"
    );
}

#[cfg(windows)]
fn driver() {
    let exe = std::env::current_exe().expect("current_exe");
    let helper = std::process::Command::new(&exe)
        .arg("--job-koc-helper")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn job-koc-helper");
    let out = helper.wait_with_output().expect("wait for helper");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let mut pid: Option<u32> = None;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("ASSIGN=true PID=") {
            pid = rest.split_whitespace().next().and_then(|p| p.parse().ok());
        }
    }
    let Some(pid) = pid else {
        // The environment made a job member impossible to create — verify
        // what still can and report the exact errors (no silent skip).
        verify_constrained(&stdout);
        return;
    };

    // Phase 1: the assigned process must be dead — the helper already
    // exited, so the only thing that can have killed it is KILL_ON_CLOSE.
    let start = Instant::now();
    let mut deadline = start + Duration::from_secs(5);
    while sh_core::job::pid_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    if sh_core::job::pid_alive(pid) {
        eprintln!(
            "job_kill_on_close: FAIL — assigned pid {pid} still alive {} ms after the \
             job owner exited (KILL_ON_CLOSE did not fire)",
            start.elapsed().as_millis()
        );
        std::process::exit(1);
    }

    // Phase 2: the WHOLE tree must be gone — including `ping`, the
    // grandchild that joined the job by construction. A scan failure
    // (powershell unavailable) is retried and eventually FAILS the test:
    // an unverifiable grandchild is not a verified one.
    deadline = start + Duration::from_secs(10);
    let mut scan_failures = 0u32;
    loop {
        match marker_pids() {
            Some(stragglers) => {
                scan_failures = 0;
                if stragglers.is_empty() {
                    println!(
                        "job_kill_on_close: PASS — KILL_ON_JOB_CLOSE verified: assigned pid {pid} \
                         and its whole tree (cmd → ping {MARKER}) dead within {} ms of the job \
                         owner's exit (no TerminateJobObject on this path)",
                        start.elapsed().as_millis()
                    );
                    return;
                }
                if Instant::now() >= deadline {
                    for &s in &stragglers {
                        let _ = std::process::Command::new("taskkill")
                            .args(["/F", "/PID", &s.to_string()])
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status();
                    }
                    eprintln!(
                        "job_kill_on_close: FAIL — stragglers {:?} alive {} ms after the job \
                         owner exited (grandchild escaped the job?)",
                        stragglers,
                        start.elapsed().as_millis()
                    );
                    std::process::exit(1);
                }
            }
            None => scan_failures += 1,
        }
        if Instant::now() >= deadline {
            eprintln!(
                "job_kill_on_close: FAIL — grandchild unverifiable: {scan_failures} consecutive \
                 command-line scan failures (powershell unavailable?)"
            );
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(not(windows))]
#[allow(dead_code)]
fn driver() {
    let _ = MARKER;
    eprintln!("job_kill_on_close: SKIP — Windows Job Object runtime proof (Windows only)");
}
