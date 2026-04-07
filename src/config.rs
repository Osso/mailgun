use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    #[default]
    Us,
    Eu,
}

impl Region {
    pub fn base_url(&self) -> &'static str {
        match self {
            Region::Us => "https://api.mailgun.net/v3",
            Region::Eu => "https://api.eu.mailgun.net/v3",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Config {
    pub api_key: Option<String>,
    pub domain: Option<String>,
    #[serde(default)]
    pub region: Region,
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mailgun-cli")
}

fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn load_config() -> Result<Config> {
    load_config_from(&config_path())
}

pub fn save_config(config: &Config) -> Result<()> {
    save_config_to(config, &config_path())
}

pub fn load_config_from(path: &PathBuf) -> Result<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let content = fs::read_to_string(path)?;
    Ok(toml::from_str(&content)?)
}

pub fn save_config_to(config: &Config, path: &PathBuf) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, toml::to_string_pretty(config)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.api_key, None);
        assert_eq!(config.domain, None);
        assert_eq!(config.region, Region::Us);
    }

    #[test]
    fn test_region_base_url() {
        assert_eq!(Region::Us.base_url(), "https://api.mailgun.net/v3");
        assert_eq!(Region::Eu.base_url(), "https://api.eu.mailgun.net/v3");
    }

    #[test]
    fn test_load_missing_file_returns_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.toml");
        let config = load_config_from(&path).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn test_save_and_load_config() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let config = Config {
            api_key: Some("key-abc123".to_string()),
            domain: Some("mg.example.com".to_string()),
            region: Region::Eu,
        };

        save_config_to(&config, &path).unwrap();
        let loaded = load_config_from(&path).unwrap();

        assert_eq!(loaded.api_key, Some("key-abc123".to_string()));
        assert_eq!(loaded.domain, Some("mg.example.com".to_string()));
        assert_eq!(loaded.region, Region::Eu);
    }

    #[test]
    fn test_save_creates_parent_dirs() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join("config.toml");

        let config = Config {
            api_key: Some("test".to_string()),
            domain: None,
            region: Region::Us,
        };

        save_config_to(&config, &path).unwrap();
        assert!(path.exists());
    }
}
