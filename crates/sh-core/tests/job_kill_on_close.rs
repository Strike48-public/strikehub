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

#[cfg(windows)]
fn helper() -> ! {
    use std::io::Write;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CREATE_SUSPENDED, CreateProcessW, PROCESS_INFORMATION, ResumeThread, STARTUPINFOW,
        TerminateProcess,
    };

    let mut cmd_line: Vec<u16> = format!("cmd /c ping -n 120 {MARKER}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
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
            CREATE_SUSPENDED,
            std::ptr::null(),
            std::ptr::null(),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        eprintln!(
            "job-koc-helper: CreateProcessW failed: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(2);
    }
    // The REAL production API: lazily creates the shared StrikeHub job
    // object (KILL_ON_CLOSE armed) and assigns the child to it.
    let assigned = sh_core::job::assign_pid_to_job(pi.dwProcessId);
    if !assigned {
        // Deterministic fail without leaking: do NOT resume (so `cmd`
        // never spawns `ping`), kill the suspended process, report.
        unsafe {
            TerminateProcess(pi.hProcess, 1);
            CloseHandle(pi.hProcess);
            CloseHandle(pi.hThread);
        }
        println!("ASSIGN=false PID={}", pi.dwProcessId);
        let _ = std::io::stdout().flush();
        std::process::exit(3);
    }
    println!("ASSIGN=true PID={}", pi.dwProcessId);
    let _ = std::io::stdout().flush();
    // Resume: `cmd` runs `ping` (the grandchild joins the job by
    // construction). Then we exit WITHOUT terminating or explicitly
    // closing the job — the OS closes the handle at process exit and
    // KILL_ON_CLOSE must kill `cmd` AND `ping`.
    unsafe {
        ResumeThread(pi.hThread);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
    }
    std::process::exit(0);
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
            pid = rest.trim().parse().ok();
        }
    }
    let Some(pid) = pid else {
        eprintln!(
            "job_kill_on_close: FAIL — helper did not report a successful job assignment\n\
             stdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        std::process::exit(1);
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
