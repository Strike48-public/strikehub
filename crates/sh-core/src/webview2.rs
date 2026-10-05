//! WebView2 runtime detection for the Windows desktop build.
//!
//! strikehub.exe statically links the WebView2 *loader* (webview2-com ships
//! `WebView2LoaderStatic.lib` for MSVC targets), so no loader DLL is needed
//! at install time. The WebView2 *runtime* (or a non-stable Microsoft Edge
//! installation) is still a host prerequisite: without either,
//! `CreateCoreWebView2Environment` fails when the first window is created and
//! the user gets a blank/unexplained failure. A clean Windows 10 VM is
//! exactly such a host, so the app probes at startup and logs an actionable,
//! pre-emptive error when both are absent (check-and-warn, never block: Edge
//! installs in odd locations are possible, and wry's own error remains the
//! source of truth).
//!
//! The check is deliberately std-only (directory + executable existence) so
//! it adds no dependency and its decision core is unit-testable on any host.

use std::path::{Path, PathBuf};

/// Outcome of the WebView2 host-prerequisite probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebView2State {
    /// A WebView2 Runtime (Evergreen or Fixed) is installed.
    RuntimePresent,
    /// No dedicated runtime, but a Microsoft Edge installation is present;
    /// wry's `CreateCoreWebView2Environment` can use it as the browser
    /// executable folder fallback.
    EdgePresent,
    /// Neither a runtime nor Edge was found: the webview cannot be created.
    Missing,
}

/// Runtime install roots probed for WebView2 Runtime version directories
/// (per-machine, then per-user). Each root contains one subdirectory per
/// installed runtime version (e.g. `.../Application/1.0.2210.55/`).
pub fn runtime_roots(
    program_files_x86: Option<&Path>,
    local_app_data: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(pf) = program_files_x86 {
        roots.push(pf.join(r"Microsoft\EdgeWebView\Application"));
    }
    if let Some(lad) = local_app_data {
        roots.push(lad.join(r"Microsoft\EdgeWebView\Application"));
    }
    roots
}

/// Candidate Microsoft Edge executable locations (the non-stable-Edge
/// fallback that wry's runtime lookup also accepts).
pub fn edge_exe_candidates(
    program_files_x86: Option<&Path>,
    program_files: Option<&Path>,
) -> Vec<PathBuf> {
    let mut cands = Vec::new();
    for dir in [program_files_x86, program_files].into_iter().flatten() {
        cands.push(dir.join(r"Microsoft\Edge\Application\msedge.exe"));
    }
    cands
}

/// Pure decision core: given how many runtime version directories exist and
/// whether an Edge executable exists, what state is the host in?
pub fn classify(runtime_version_dirs: usize, edge_exe_present: bool) -> WebView2State {
    if runtime_version_dirs > 0 {
        WebView2State::RuntimePresent
    } else if edge_exe_present {
        WebView2State::EdgePresent
    } else {
        WebView2State::Missing
    }
}

/// Count installed runtime version directories under one `...\Application`
/// root. Missing root, unreadable root, and file entries all count as zero.
pub fn count_version_dirs(root: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .count()
}

/// Probe the local machine: WebView2 Runtime version directories first
/// (per-machine, then per-user), then a Microsoft Edge executable.
///
/// On non-Windows hosts the env vars are absent and this returns
/// [`WebView2State::Missing`]; call sites gate on `target_os = "windows"`.
pub fn probe() -> WebView2State {
    let pf_x86 = std::env::var("ProgramFiles(x86)").ok().map(PathBuf::from);
    let pf = std::env::var("ProgramFiles").ok().map(PathBuf::from);
    let lad = std::env::var("LOCALAPPDATA").ok().map(PathBuf::from);

    let mut version_dirs = 0usize;
    for root in runtime_roots(pf_x86.as_deref(), lad.as_deref()) {
        version_dirs += count_version_dirs(&root);
    }

    let edge_present = edge_exe_candidates(pf_x86.as_deref(), pf.as_deref())
        .iter()
        .any(|c| c.is_file());

    classify(version_dirs, edge_present)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_runtime_wins_over_edge() {
        assert_eq!(classify(1, true), WebView2State::RuntimePresent);
        assert_eq!(classify(3, false), WebView2State::RuntimePresent);
    }

    #[test]
    fn classify_edge_when_no_runtime() {
        assert_eq!(classify(0, true), WebView2State::EdgePresent);
    }

    #[test]
    fn classify_missing_when_neither() {
        assert_eq!(classify(0, false), WebView2State::Missing);
    }

    #[test]
    fn count_version_dirs_counts_only_dirs() {
        let dir = std::env::temp_dir().join(format!("wv2-roots-{}", std::process::id()));
        let app = dir.join(r"Microsoft\EdgeWebView\Application");
        std::fs::create_dir_all(app.join("1.0.2210.55")).unwrap();
        std::fs::create_dir_all(app.join("1.0.2510.6")).unwrap();
        std::fs::write(app.join("stray-file"), b"x").unwrap();
        assert_eq!(count_version_dirs(&app), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn count_version_dirs_zero_for_missing_root() {
        let dir = std::env::temp_dir().join(format!("wv2-none-{}", std::process::id()));
        assert_eq!(count_version_dirs(&dir.join("does-not-exist")), 0);
    }

    #[test]
    fn runtime_roots_and_edge_candidates_layout() {
        let roots = runtime_roots(
            Some(Path::new(r"C:\Program Files (x86)")),
            Some(Path::new(r"C:\Users\me\AppData\Local")),
        );
        assert_eq!(roots.len(), 2);
        assert!(
            roots[0]
                .to_string_lossy()
                .ends_with(r"Microsoft\EdgeWebView\Application")
        );
        assert!(
            roots[1]
                .to_string_lossy()
                .ends_with(r"Microsoft\EdgeWebView\Application")
        );

        let cands = edge_exe_candidates(
            Some(Path::new(r"C:\Program Files (x86)")),
            Some(Path::new(r"C:\Program Files")),
        );
        assert_eq!(cands.len(), 2);
        assert!(
            cands[0]
                .to_string_lossy()
                .ends_with(r"Microsoft\Edge\Application\msedge.exe")
        );
    }

    #[test]
    fn empty_roots_when_env_absent() {
        assert!(runtime_roots(None, None).is_empty());
        assert!(edge_exe_candidates(None, None).is_empty());
    }
}
