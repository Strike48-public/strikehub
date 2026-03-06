use thiserror::Error;

#[derive(Debug, Error)]
pub enum HubError {
    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("runner error: {0}")]
    Runner(String),
}
