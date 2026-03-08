//! Extract bundled connector binaries next to the running executable.
//!
//! On Windows the connector binaries (ks-connector.exe, pentest-agent.exe) are
//! embedded into the StrikeHub executable at build time using `include_bytes!`.
//! On first run (or when the embedded version is newer) they are extracted to
//! the same directory as `strikehub.exe`.
//!
//! On non-Windows platforms this module is a no-op — connectors are expected to
//! be installed separately (via ~/bin, PATH, or sibling workspace targets).

use std::path::PathBuf;

/// Extract all bundled connector binaries next to the running executable.
///
/// Returns the directory they were extracted to (or None if not applicable).
pub fn extract_bundled_binaries() -> Option<PathBuf> {
    #[cfg(not(target_os = "windows"))]
    {
        None
    }

    #[cfg(target_os = "windows")]
    {
        extract_on_windows()
    }
}

#[cfg(target_os = "windows")]
fn extract_on_windows() -> Option<PathBuf> {
    let exe_path = std::env::current_exe().ok()?;
    let exe_dir = exe_path.parent()?;

    // The binaries will be placed here by the release build / packaging step.
    // For now we just ensure the directory is returned so resolve_binary() finds them.
    // In development, connector binaries are found via the sibling workspace scan.
    //
    // Future: when CI bundles the connectors into the installer / zip, they will
    // be placed next to strikehub.exe. This function can be extended to extract
    // from embedded bytes if we choose to bake them in with include_bytes!.

    tracing::debug!("connector binary dir: {}", exe_dir.display());
    Some(exe_dir.to_path_buf())
}
