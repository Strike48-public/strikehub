# Process-tree teardown (issue #115)

Closing StrikeHub used to leave spawned connector processes — and everything
they spawned — alive. This document describes the teardown API
(`sh_core::process`, plus `sh_core::job` on Windows), which code path uses
which mechanism, and what CI verifies on each platform.

## The guarantee

On close — normal or signal — StrikeHub escalates **TERM → ~2 s → KILL**
across the ENTIRE spawned tree, and `kill_on_drop` + (Linux)
`PR_SET_PDEATHSIG(SIGHUP)` remain as direct-child backstops.

| Platform | Mechanism | Where it runs |
|---|---|---|
| Linux / macOS | per-child process group (`process_group(0)` at spawn) + process-lifetime registry + `kill(-pgid)` escalation; bounded best-effort descendant walk (`/proc` on Linux, `ps` on macOS) catches `setsid` escapers | `teardown_process_tree()` on normal close; `signal_kill_tracked_groups()` (async-signal-safe, groups only) in the SIGINT/SIGTERM/**SIGTRAP** handler |
| Windows | ONE shared Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; every child assigned at spawn (`assign_pid_to_job`), `TerminateJobObject` on close, and the OS kills the job when the handle closes on **any** exit path | `teardown_process_tree()` on normal close; KILL_ON_JOB_CLOSE covers every other path (crash, signal, `_exit`) |

## Public API

- `sh_core::process::spawn_tracked(&mut tokio::process::Command) -> Child`
  — the only spawn path that gets the guarantees (unix: child leads its own
  group, pgid re-verified after spawn; windows: child assigned to the job
  immediately, before any `await`).
- `sh_core::process::track_child(pid, [pgid])` / `tracked_children_snapshot()`
  — the append-only process-lifetime registry.
- `sh_core::process::teardown_process_tree()` — normal-close teardown
  (synchronous, idempotent, not signal-handler-safe).
- `sh_core::process::ProcessTreeGuard` — drop-guard fallback for exits that
  skip the explicit call (panic, early return).
- `sh_core::process::signal_kill_tracked_groups(sig)` — **unix only**,
  async-signal-safe, called by the `sh-ui` handler (SIGINT/SIGTERM/SIGTRAP).
- `sh_core::process::collect_descendants(roots)` — bounded descendant walk
  (≤ 4096 nodes, depth ≤ 32); unix only.
- `sh_core::job::{assign_pid_to_job, terminate_job, terminate_pid, pid_alive, job_is_armed}`
  — **windows only**, the Job Object primitives used by the above (`job_is_armed` reports whether the shared job exists with KILL_ON_CLOSE armed).

## Known boundary (signal path, unix)

The signal handler may only call async-signal-safe functions, so it does
**groups-only** teardown: a descendant that escaped via `setsid` at the
moment of a signal death is not walked. No observed impact — the agent
stays in-group in both issue #115 REPROs — but it is a documented boundary,
not a gap that was missed.

## Verification (per platform, in CI)

| What | Where |
|---|---|
| unix fixture-tree integration test (`tests/process_tree.rs`, real `spawn_tracked` + registry, TERM→KILL escalation, respawn + setsid cases) + registry/walk unit tests | `Test` job (ubuntu) |
| **compile** of the whole `sh-core` crate (incl. all `cfg(windows)` code) for `x86_64-pc-windows-msvc` | `Check (Windows)` job (`windows-latest`, every PR — the MSI installer job is `main`-gated) |
| **runtime** of the Job Object path: `CreateJobObjectW`/KILL_ON_CLOSE setup, `AssignProcessToJobObject` (success + dead-pid error path), `TerminateJobObject`, and an end-to-end `KILL_ON_JOB_CLOSE` probe (a `cmd → ping` tree assigned to the job must be dead within seconds of the job owner's exit — no `TerminateJobObject` on that path). Fixtures are two-pass (plain spawn → `CREATE_BREAKAWAY_FROM_JOB` after a best-effort `SeCreatePagefilePrivilege` enable) because the `windows-latest` agent runs its tree inside its own Job Object; where a job member is impossible to create, a documented constrained mode verifies everything the environment still allows and reports the exact Win32 errors | `Check (Windows)` job — `cargo test -p sh-core --target x86_64-pc-windows-msvc job` |
| full MSVC build + link of the app (MSI) | `Build MSI x86_64` job (gated to `main`) |
| macOS `ps` walk runtime | not in CI (DMG job is `main`-gated); same bounded walk as the Linux path |
