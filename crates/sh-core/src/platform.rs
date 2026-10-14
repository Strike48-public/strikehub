//! Supported host platforms and native CPU detection.
//!
//! [`SUPPORTED_PLATFORMS`] is the single list of platforms StrikeHub ships and
//! tests. It must match the release matrix in `.github/workflows/release.yml`;
//! the preflight Platform check and the installers read from it.

/// `(os, arch)` pairs StrikeHub ships, using `std::env::consts` names.
pub const SUPPORTED_PLATFORMS: &[(&str, &str)] = &[
    ("macos", "x86_64"),
    ("macos", "aarch64"),
    ("linux", "x86_64"),
    ("linux", "aarch64"),
    ("windows", "x86_64"),
];

/// True when StrikeHub ships a build for this OS and CPU.
pub fn is_supported(os: &str, arch: &str) -> bool {
    SUPPORTED_PLATFORMS.contains(&(os, arch))
}

/// The host's CPU architecture, which can differ from the binary's.
///
/// The Windows build is x64 and runs under emulation on ARM64, where
/// `std::env::consts::ARCH` and the process's `PROCESSOR_ARCHITECTURE` both
/// report x86_64. The machine-wide value in the registry is not emulated.
pub fn native_arch() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        if let Some(arch) = windows_native_arch() {
            return arch;
        }
    }
    std::env::consts::ARCH
}

#[cfg(target_os = "windows")]
fn windows_native_arch() -> Option<&'static str> {
    let output = crate::preflight::hidden_command("reg")
        .args([
            "query",
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            "/v",
            "PROCESSOR_ARCHITECTURE",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_reg_processor_architecture(&String::from_utf8_lossy(&output.stdout))
}

/// Parse `reg query ... /v PROCESSOR_ARCHITECTURE` output into a
/// `std::env::consts::ARCH` name.
///
/// Output format: "    PROCESSOR_ARCHITECTURE    REG_SZ    ARM64"
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_reg_processor_architecture(text: &str) -> Option<&'static str> {
    let value = text
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("PROCESSOR_ARCHITECTURE"))?
        .split_whitespace()
        .nth(2)?;
    match value.to_ascii_uppercase().as_str() {
        "AMD64" => Some("x86_64"),
        "ARM64" => Some("aarch64"),
        "X86" => Some("x86"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_arm64_is_not_supported() {
        assert!(!is_supported("windows", "aarch64"));
    }

    #[test]
    fn every_released_platform_is_supported() {
        for (os, arch) in SUPPORTED_PLATFORMS {
            assert!(is_supported(os, arch));
        }
    }

    #[test]
    fn parses_arm64_from_reg_query_output() {
        let out = "\r\nHKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment\r\n    PROCESSOR_ARCHITECTURE    REG_SZ    ARM64\r\n\r\n";
        assert_eq!(parse_reg_processor_architecture(out), Some("aarch64"));
    }

    #[test]
    fn parses_amd64_from_reg_query_output() {
        let out = "    PROCESSOR_ARCHITECTURE    REG_SZ    AMD64\n";
        assert_eq!(parse_reg_processor_architecture(out), Some("x86_64"));
    }

    #[test]
    fn unknown_or_missing_value_is_none() {
        assert_eq!(
            parse_reg_processor_architecture("    PROCESSOR_ARCHITECTURE    REG_SZ    IA64\n"),
            None
        );
        assert_eq!(parse_reg_processor_architecture("ERROR: not found\n"), None);
    }
}
