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
}
