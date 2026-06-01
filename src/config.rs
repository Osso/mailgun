use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

/// Credentials for a single Mailgun account/domain (a "site").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SiteConfig {
    pub api_key: String,
    pub domain: String,
    #[serde(default)]
    pub region: Region,
}

/// Resolved credentials used to build a client.
pub struct ResolvedSite {
    pub api_key: String,
    pub domain: String,
    pub region: Region,
}

#[derive(Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_site: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub sites: HashMap<String, SiteConfig>,

    // Legacy single-site fields, kept for backward compatibility with old
    // config files. Treated as the fallback when no named site is selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "is_default_region")]
    pub region: Region,
}

fn is_default_region(region: &Region) -> bool {
    *region == Region::default()
}

impl Config {
    /// Resolve which site's credentials to use.
    ///
    /// Order: explicit `--site` flag → configured default site → legacy flat
    /// fields. Errors with the list of available sites when ambiguous.
    pub fn resolve_site(&self, site: Option<&str>) -> Result<ResolvedSite> {
        let name = site
            .map(str::to_string)
            .or_else(|| self.default_site.clone());

        if let Some(name) = name {
            return match self.sites.get(&name) {
                Some(cfg) => Ok(ResolvedSite {
                    api_key: cfg.api_key.clone(),
                    domain: cfg.domain.clone(),
                    region: cfg.region,
                }),
                None => bail!(
                    "Site '{}' not found. Available sites: {}",
                    name,
                    self.list_sites()
                ),
            };
        }

        // No named site selected: fall back to legacy flat config.
        match (&self.api_key, &self.domain) {
            (Some(api_key), Some(domain)) => Ok(ResolvedSite {
                api_key: api_key.clone(),
                domain: domain.clone(),
                region: self.region,
            }),
            _ => bail!(
                "No site selected and no default configured. \
                 Use -s <site> or set a default. Available sites: {}",
                self.list_sites()
            ),
        }
    }

    pub fn list_sites(&self) -> String {
        if self.sites.is_empty() {
            return "(none)".to_string();
        }
        let mut names: Vec<_> = self.sites.keys().cloned().collect();
        names.sort();
        names
            .into_iter()
            .map(|name| {
                if Some(&name) == self.default_site.as_ref() {
                    format!("{}*", name)
                } else {
                    name
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn set_site(&mut self, name: &str, site: SiteConfig, set_default: bool) {
        self.sites.insert(name.to_string(), site);
        if set_default || self.default_site.is_none() {
            self.default_site = Some(name.to_string());
        }
    }

    pub fn remove_site(&mut self, name: &str) -> bool {
        let removed = self.sites.remove(name).is_some();
        if self.default_site.as_deref() == Some(name) {
            self.default_site = self.sites.keys().next().cloned();
        }
        removed
    }
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mailgun-cli")
}

fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn config_path_display() -> String {
    config_path().display().to_string()
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
        assert!(config.sites.is_empty());
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
    fn test_save_and_load_sites() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut config = Config::default();
        config.set_site(
            "globalcomix",
            SiteConfig {
                api_key: "key-abc123".to_string(),
                domain: "mg.globalcomix.com".to_string(),
                region: Region::Us,
            },
            true,
        );

        save_config_to(&config, &path).unwrap();
        let loaded = load_config_from(&path).unwrap();

        assert_eq!(loaded.default_site.as_deref(), Some("globalcomix"));
        let resolved = loaded.resolve_site(None).unwrap();
        assert_eq!(resolved.domain, "mg.globalcomix.com");
        assert_eq!(resolved.api_key, "key-abc123");
    }

    #[test]
    fn test_explicit_site_overrides_default() {
        let mut config = Config::default();
        config.set_site(
            "a",
            SiteConfig {
                api_key: "ka".into(),
                domain: "a.com".into(),
                region: Region::Us,
            },
            true,
        );
        config.set_site(
            "b",
            SiteConfig {
                api_key: "kb".into(),
                domain: "b.com".into(),
                region: Region::Eu,
            },
            false,
        );

        assert_eq!(config.resolve_site(None).unwrap().domain, "a.com");
        assert_eq!(config.resolve_site(Some("b")).unwrap().domain, "b.com");
        assert_eq!(config.resolve_site(Some("b")).unwrap().region, Region::Eu);
    }

    #[test]
    fn test_unknown_site_errors() {
        let config = Config::default();
        assert!(config.resolve_site(Some("nope")).is_err());
    }

    #[test]
    fn test_legacy_flat_config_fallback() {
        // Old config files have flat api_key/domain and no sites.
        let config = Config {
            api_key: Some("key-legacy".to_string()),
            domain: Some("mangahelpers.com".to_string()),
            region: Region::Us,
            ..Default::default()
        };
        let resolved = config.resolve_site(None).unwrap();
        assert_eq!(resolved.domain, "mangahelpers.com");
        assert_eq!(resolved.api_key, "key-legacy");
    }

    #[test]
    fn test_remove_site_reassigns_default() {
        let mut config = Config::default();
        config.set_site(
            "a",
            SiteConfig {
                api_key: "ka".into(),
                domain: "a.com".into(),
                region: Region::Us,
            },
            true,
        );
        config.set_site(
            "b",
            SiteConfig {
                api_key: "kb".into(),
                domain: "b.com".into(),
                region: Region::Us,
            },
            false,
        );
        assert!(config.remove_site("a"));
        assert_eq!(config.default_site.as_deref(), Some("b"));
    }
}
