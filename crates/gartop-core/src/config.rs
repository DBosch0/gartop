//! XDG paths, the optional `config.toml`, and API-key lookup.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::Deserialize;

/// Errors raised while resolving paths or loading configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not determine a home directory for XDG paths")]
    NoHome,
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid config file {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error(
        "GARMIN_API_KEY not found: set it in the environment, ./.env, or {}",
        .0.display()
    )]
    MissingApiKey(PathBuf),
}

/// Filesystem locations used by gartop.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    /// Resolve the XDG directories (`~/.config/gartop`, `~/.local/share/gartop`, `~/.cache/gartop`).
    pub fn discover() -> Result<Self, ConfigError> {
        let dirs = ProjectDirs::from("", "", "gartop").ok_or(ConfigError::NoHome)?;
        Ok(Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_dir().to_path_buf(),
            cache_dir: dirs.cache_dir().to_path_buf(),
        })
    }

    /// Paths rooted in a single directory; handy for tests.
    #[must_use]
    pub fn rooted(root: &Path) -> Self {
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
        }
    }

    #[must_use]
    pub fn db_file(&self) -> PathBuf {
        self.data_dir.join("gartop.db")
    }

    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.data_dir.join("fetch.lock")
    }

    #[must_use]
    pub fn token_file(&self) -> PathBuf {
        self.cache_dir.join("token")
    }

    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    #[must_use]
    pub fn env_file(&self) -> PathBuf {
        self.config_dir.join(".env")
    }
}

/// User configuration from `~/.config/gartop/config.toml`. Every field is optional.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Override for the database path.
    pub db_path: Option<PathBuf>,
    /// Number of recent days re-fetched by `gartop-fetch sync`.
    pub sync_days: u32,
    /// API base URL (useful for testing against a mock server).
    pub base_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db_path: None,
            sync_days: 7,
            base_url: "https://personal.connectapi.garmin.com".to_owned(),
        }
    }
}

impl Config {
    /// Load the config file if it exists, otherwise return defaults.
    pub fn load(paths: &Paths) -> Result<Self, ConfigError> {
        let path = paths.config_file();
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|source| ConfigError::Parse { path, source }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Read { path, source }),
        }
    }

    /// The database path: the override from the config file, or the XDG default.
    #[must_use]
    pub fn db_path(&self, paths: &Paths) -> PathBuf {
        self.db_path.clone().unwrap_or_else(|| paths.db_file())
    }
}

/// Find the Garmin API key: environment, then `./.env`, then `~/.config/gartop/.env`.
pub fn api_key(paths: &Paths) -> Result<String, ConfigError> {
    const VAR: &str = "GARMIN_API_KEY";
    if let Ok(key) = std::env::var(VAR)
        && !key.trim().is_empty()
    {
        return Ok(key.trim().to_owned());
    }
    for file in [PathBuf::from(".env"), paths.env_file()] {
        if let Ok(iter) = dotenvy::from_path_iter(&file) {
            for item in iter.flatten() {
                if item.0 == VAR && !item.1.trim().is_empty() {
                    return Ok(item.1.trim().to_owned());
                }
            }
        }
    }
    Err(ConfigError::MissingApiKey(paths.env_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_no_config_file() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(tmp.path());
        let cfg = Config::load(&paths).unwrap();
        assert_eq!(cfg.sync_days, 7);
        assert_eq!(cfg.db_path(&paths), paths.db_file());
    }

    #[test]
    fn parses_partial_config() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(tmp.path());
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(paths.config_file(), "sync_days = 3\n").unwrap();
        let cfg = Config::load(&paths).unwrap();
        assert_eq!(cfg.sync_days, 3);
        assert!(cfg.base_url.starts_with("https://"));
    }

    #[test]
    fn rejects_unknown_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(tmp.path());
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(paths.config_file(), "nope = 1\n").unwrap();
        assert!(matches!(
            Config::load(&paths),
            Err(ConfigError::Parse { .. })
        ));
    }
}
