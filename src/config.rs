use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::{linear, merge::MergeMethod};

#[derive(Deserialize)]
struct ConfigFile {
    accounts: Vec<Entry>,
    #[serde(flatten)]
    merge: MergeOptions,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub struct MergeOptions {
    pub enable_merge: bool,
    pub merge_method: MergeMethod,
}

pub struct Config {
    pub accounts: Vec<AccountConfig>,
    pub merge: MergeOptions,
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    key_env: Option<String>,
    key_file: Option<String>,
}

/// A configured Account (one Linear login). A missing key is kept as an error
/// message so one bad Account does not stop the others from loading.
pub struct AccountConfig {
    pub name: String,
    pub key: Result<String, String>,
}

pub fn config_path() -> PathBuf {
    if let Ok(dir) = std::env::var("HERDR_PLUGIN_CONFIG_DIR") {
        return PathBuf::from(dir).join("config.toml");
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/herdr-linear/config.toml")
}

pub fn load() -> Result<Config> {
    let path = config_path();
    let text = match read_config(&path)? {
        Some(text) => text,
        None => {
            // No config file: fall back to the single-key setup.
            let key = linear::api_key()
                .map_err(|_| format!("no config at {} and no LINEAR_API_KEY set", path.display()));
            return Ok(Config {
                accounts: vec![AccountConfig {
                    name: "linear".into(),
                    key,
                }],
                merge: MergeOptions::default(),
            });
        }
    };
    let file: ConfigFile =
        toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))?;
    if file.accounts.is_empty() {
        bail!("{} has no [[accounts]]", path.display());
    }
    Ok(Config {
        accounts: file.accounts.into_iter().map(resolve).collect(),
        merge: file.merge,
    })
}

fn read_config(path: &std::path::Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("cannot read config {}", path.display())),
    }
}

fn resolve(entry: Entry) -> AccountConfig {
    let key = match (&entry.key_env, &entry.key_file) {
        (Some(var), _) => std::env::var(var)
            .map_err(|_| format!("env var {var} is not set"))
            .and_then(non_empty),
        (None, Some(file)) => std::fs::read_to_string(expand_home(file))
            .map_err(|e| format!("cannot read {file}: {e}"))
            .and_then(non_empty),
        (None, None) => Err("set key_env or key_file".to_string()),
    };
    AccountConfig {
        name: entry.name,
        key,
    }
}

fn non_empty(key: String) -> Result<String, String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        Err("key is empty".into())
    } else {
        Ok(key)
    }
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_accounts() {
        let file: ConfigFile = toml::from_str(
            "[[accounts]]\nname = \"work\"\nkey_env = \"A\"\n[[accounts]]\nname = \"personal\"\nkey_file = \"~/k\"\n",
        )
        .unwrap();
        assert!(!file.merge.enable_merge);
        assert_eq!(file.merge.merge_method, MergeMethod::Squash);
        assert_eq!(file.accounts.len(), 2);
        assert_eq!(file.accounts[1].key_file.as_deref(), Some("~/k"));
    }

    #[test]
    fn merge_is_explicitly_opt_in_and_method_is_validated() {
        let file: ConfigFile = toml::from_str(
            "enable_merge = true\nmerge_method = \"rebase\"\n[[accounts]]\nname = \"example\"\n",
        )
        .unwrap();
        assert!(file.merge.enable_merge);
        assert_eq!(file.merge.merge_method, MergeMethod::Rebase);
        assert!(toml::from_str::<ConfigFile>(
            "merge_method = \"auto\"\n[[accounts]]\nname = \"example\"\n"
        )
        .is_err());
    }

    #[test]
    fn missing_config_differs_from_unreadable_config() {
        let dir =
            std::env::temp_dir().join(format!("herdr-linear-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_config(&dir.join("missing.toml")).unwrap().is_none());
        // A directory exists but cannot be read as a file.
        let error = read_config(&dir).unwrap_err().to_string();
        assert!(error.contains("cannot read config"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn account_without_key_source_is_an_error() {
        let account = resolve(Entry {
            name: "x".into(),
            key_env: None,
            key_file: None,
        });
        assert!(account.key.is_err());
    }
}
