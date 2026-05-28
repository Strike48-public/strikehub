use std::path::Path;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let config_path = Path::new(&manifest_dir)
        .join("..")
        .join("..")
        .join("build-defaults.toml");

    println!("cargo:rerun-if-changed={}", config_path.display());

    let contents = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        panic!(
            "Failed to read build-defaults.toml at {}: {}",
            config_path.display(),
            e
        )
    });

    let table: toml::Table = contents
        .parse()
        .unwrap_or_else(|e| panic!("Failed to parse build-defaults.toml: {}", e));

    let default_url = table
        .get("studio")
        .and_then(|v| v.get("default_url"))
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("build-defaults.toml: missing studio.default_url"));

    println!(
        "cargo:rustc-env=STRIKEHUB_DEFAULT_STUDIO_URL={}",
        default_url
    );

    // Emit default allowed sources for the connector allowlist.
    // The value is a comma-separated string of org/repo patterns.
    let allowed_sources: Vec<String> = table
        .get("connectors")
        .and_then(|v| v.get("allowed_sources"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    println!(
        "cargo:rustc-env=STRIKEHUB_DEFAULT_ALLOWED_SOURCES={}",
        allowed_sources.join(",")
    );

    // Sentry configuration (optional)
    if let Some(sentry) = table.get("sentry") {
        if let Some(dsn) = sentry.get("dsn").and_then(|v| v.as_str())
            && !dsn.is_empty()
        {
            println!("cargo:rustc-env=STRIKEHUB_SENTRY_DSN={}", dsn);
        }
        if let Some(env) = sentry.get("environment").and_then(|v| v.as_str())
            && !env.is_empty()
        {
            println!("cargo:rustc-env=STRIKEHUB_SENTRY_ENVIRONMENT={}", env);
        }
        if let Some(rate) = sentry.get("traces_sample_rate").and_then(|v| v.as_str()) {
            println!(
                "cargo:rustc-env=STRIKEHUB_SENTRY_TRACES_SAMPLE_RATE={}",
                rate
            );
        }
    }
}
