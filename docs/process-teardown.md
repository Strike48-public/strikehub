# Process-tree teardown (issue #115)

Closing StrikeHub used to leave spawned connector processes — and everything
they spawned — alive. This document describes the teardown API
(`sh_core::process`, plus `sh_core::job` on Windows), which code path uses
which mechanism, and what CI verifies on each platform.

## The guarantee

On close — normal, quit-event, or signal — StrikeHub escalates
**TERM → ~2 s → KILL** across the ENTIRE spawned tree, plus a bounded
**managed-root fallback sweep** for self-update-respawned processes that
escaped the registry — including the **full descendant subtree** of every
matched root process (their children/grandchildren exec binaries OUTSIDE
the roots, so only the walk from the matched roots can see them) — and
`kill_on_drop` + (Linux)
`PR_SET_PDEATHSIG(SIGHUP)` remain as direct-child backstops.

| Platform | Mechanism | Where it runs |
|---|---|---|
| Linux / macOS | per-child process group (`process_group(0)` at spawn) + process-lifetime registry + `kill(-pgid)` escalation; bounded best-effort descendant walk (`/proc` on Linux, `ps` on macOS) catches `setsid` escapers; **managed-root fallback sweep** (resolved-exe path-prefix whitelist over `~/.strike48/strikehub/bin/` + the bundle `MacOS` dir on macOS) catches self-update-respawned processes that are untracked and/or reparented to init/launchd, **plus their full descendant subtrees** (children/grandchildren whose executables live outside the roots — walk-only coverage) | `teardown_process_tree()` on normal close AND via the **C-level `atexit` hook** (`install_exit_handler`) on every `std::process::exit` route; `signal_kill_tracked_groups()` (async-signal-safe, groups only) in the SIGINT/SIGTERM/**SIGTRAP** handler |
| Windows | ONE shared Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; every child assigned at spawn (`assign_pid_to_job`), `TerminateJobObject` on close, and the OS kills the job when the handle closes on **any** exit path | `teardown_process_tree()` on normal close; KILL_ON_JOB_CLOSE covers every other path (crash, signal, `_exit`) |

## Exit routes and who tears down (the RC-remainder wiring)

dioxus 0.6.3's `launch()` blocks in tao's `EventLoop::run`, and tao 0.30.8's
`run` is **diverging** on every platform: when the OS run loop ends — last
window closed (`ControlFlow::Exit`) or, on macOS, the application-terminate
event (AppleEvent quit via `osascript`, Cmd-Q, dock Quit, through
`applicationShouldTerminate`) — tao calls `std::process::exit` directly.
That skips the code after `launch()` in `sh-ui/src/main.rs` **and** every
Rust Drop impl (`ProcessTreeGuard`, `kill_on_drop`). Route map:

| Exit route | Teardown delivered by |
|---|---|
| macOS / Windows / Linux: last window closed (red button) | atexit hook (tao `process::exit`) |
| **macOS: quit-event — `osascript quit` / Cmd-Q / dock Quit (RC REPRO route)** | atexit hook (tao `process::exit`) |
| SIGINT / SIGTERM / SIGTRAP (Linux WebKitGTK close) | signal handler (`signal_kill_tracked_groups`, groups only) |
| panic / early return through `main` | `ProcessTreeGuard` Drop + atexit hook |
| Windows: any path incl. crash | Job Object `KILL_ON_JOB_CLOSE` (OS guarantee) + atexit/explicit call |

All routes are idempotent with each other (dead pids/groups are ESRCH
no-ops), so overlapping deliveries are harmless.

## Public API

- `sh_core::process::spawn_tracked(&mut tokio::process::Command) -> Child`
  — the only spawn path that gets the guarantees (unix: child leads its own
  group, pgid re-verified after spawn; windows: child assigned to the job
  immediately, before any `await`).
- `sh_core::process::track_child(pid, [pgid])` / `tracked_children_snapshot()`
  — the append-only process-lifetime registry.
- `sh_core::process::teardown_process_tree()` — normal-close / atexit
  teardown (synchronous, idempotent, not signal-handler-safe): tracked
  groups + descendant walk + **managed-root sweep (matched roots + their
  full bounded descendant subtrees)**, one TERM → grace → KILL escalation
  over the union.
- `sh_core::process::install_exit_handler()` — **unix only**, installs the
  C-level `atexit` hook that runs `teardown_process_tree()` on
  `std::process::exit` (called once at startup in `sh-ui/src/main.rs`;
  idempotent).
- `sh_core::process::atexit_teardown()` — **unix only**, the exact entry the
  atexit hook runs (public so tests can drive the quit-route entry point
  directly).
- `sh_core::process::managed_roots()` / `add_managed_root(path)` — **unix
  only**, the sweep's directory whitelist (`~/.strike48/strikehub/bin` +
  the macOS bundle `MacOS` dir; `add_managed_root` is a test hook —
  production startup registers nothing extra). Roots are **canonicalized**
  (the matcher is fed RESOLVED exe paths, so a symlinked `$HOME`/bundle
  location must resolve identically); a not-yet-existing root keeps its
  raw path as fallback.
- `sh_core::process::is_managed_exe(path, roots)` — **unix only**, the
  strict component-wise prefix matcher (lookalike prefixes rejected;
  callers pass RESOLVED exe paths, so symlink escapes never match).
- `sh_core::process::sweep_managed_roots(roots)` — **unix only**, the
  bounded sweep itself (`/proc` + `readlink` on Linux; `kern.proc.pid`
  `sysctlbyname` + `proc_pidpath` on macOS; self excluded; ≤ 4096 pids /
  ≤ 64 targets). Returns the MATCHED ROOT processes only —
  `teardown_process_tree` extends them with the bounded descendant walk
  before the TERM pass (AC: zero descendants — children AND grandchildren
  run exes outside the roots).
- `sh_core::process::ProcessTreeGuard` — drop-guard fallback for exits that
  skip the explicit call (panic, early return). NOTE: Drop never runs for a
  `std::process::exit` death — the atexit hook covers that route.
- `sh_core::process::signal_kill_tracked_groups(sig)` — **unix only**,
  async-signal-safe, called by the `sh-ui` handler (SIGINT/SIGTERM/SIGTRAP).
- `sh_core::process::collect_descendants(roots)` — bounded descendant walk
  (≤ 4096 nodes, depth ≤ 32); unix only.
- `sh_core::job::{assign_pid_to_job, terminate_job, terminate_pid, pid_alive, job_is_armed}`
  — **windows only**, the Job Object primitives used by the above (`job_is_armed` reports whether the shared job exists with KILL_ON_CLOSE armed).

## Known boundaries

- **Signal path (unix) is groups-only by necessity.** The signal handler may
  only call async-signal-safe functions, so it neither walks descendants
  (a `setsid` escaper at the moment of a signal death is not walked) nor
  runs the managed-root sweep (it reads /proc / calls sysctl). No observed
  impact — the agent stays in-group in both issue #115 REPROs — but it is a
  documented boundary, not a gap that was missed.
- **Sweep matches RESOLVED exe paths only.** A process launched through a
  symlink whose target lives outside the managed roots is NOT swept
  (safety over coverage; the registry + walk still cover tracked trees).
  A connector whose real binary was placed outside the managed roots by
  some non-standard self-update is likewise out of scope.
- **The running executable's directory is a sweep root only for a macOS
  bundle** (`…/StrikeHub.app/Contents/MacOS`). A dev `target/debug` tree or a
  Linux non-bundle install dir is deliberately NOT a root, so the sweep can
  never reach unrelated processes sharing a build directory.
- **macOS runtime leg is not in CI** (no macOS test runner; the DMG job is
  `main`-gated). The macOS enumeration/resolution code (`kern.proc.pid` +
  `proc_pidpath`) compiles for `aarch64-apple-darwin` and carries a
  cfg-gated self-test (`macos_pid_enumeration_and_exe_resolution_see_self`)
  that runs wherever a mac can execute the suite; the RC QA lab box is the
  runtime verification point for the quit-event + sweep combination.

## Verification (per platform, in CI)

| What | Where |
|---|---|
| unix fixture-tree integration tests (`tests/process_tree.rs`, real `spawn_tracked` + registry): TERM→KILL escalation, respawn + setsid cases, **respawned-successor-outside-the-registry sweep case (RED→GREEN for the RC REPRO)**, **managed-root sweep SUBTREE case — successor (fixture exe under the root) → child → grandchild exec'd to a system binary OUTSIDE every root (RED→GREEN for the post-review CQ finding)**, **atexit-hook-fires-on-`process::exit` proof (fork + `std::process::exit(42)` — the exact tao quit-route exit)**, **direct call of the atexit quit-route entry**, **symlink-escape whitelist rejection** | `Test` job (ubuntu) |
| sweep-matcher unit tests (whitelist prefix exactness; lookalike `bin2`/`bin-evil`/other-user rejection; cache dir always a root; **symlinked roots canonicalized + raw-path fallback for not-yet-existing roots**; atexit entry safe no-op) | `Test` job (ubuntu) — `sh-core` lib tests |
| **compile** of the whole `sh-core` crate (incl. all `cfg(windows)` and `cfg(target_os = "macos")` sweep code) for `x86_64-pc-windows-msvc` | `Check (Windows)` job (`windows-latest`, every PR — the MSI installer job is `main`-gated) |
| **runtime** of the Job Object path: `CreateJobObjectW`/KILL_ON_CLOSE setup, `AssignProcessToJobObject` (success + dead-pid error path), `TerminateJobObject`, and an end-to-end `KILL_ON_JOB_CLOSE` probe (a `cmd → ping` tree assigned to the job must be dead within seconds of the job owner's exit — no `TerminateJobObject` on that path). Fixtures are two-pass (plain spawn → `CREATE_BREAKAWAY_FROM_JOB` after a best-effort `SeCreatePagefilePrivilege` enable) because the `windows-latest` agent runs its tree inside its own Job Object; where a job member is impossible to create, a documented constrained mode verifies everything the environment still allows and reports the exact Win32 errors | `Check (Windows)` job — `cargo test -p sh-core --target x86_64-pc-windows-msvc job` |
| full MSVC build + link of the app (MSI) | `Build MSI x86_64` job (gated to `main`) |
| macOS `ps` walk + `kern.proc.pid`/`proc_pidpath` sweep runtime | not in CI (DMG job is `main`-gated); RC QA lab box (macOS 10.10.0.8) re-verifies the quit-event route per issue #115 |
