use std::path::PathBuf;

use directories::ProjectDirs;
use serde::Deserialize;

#[derive(Deserialize, Default)]
pub struct Config {
    /// Absolute path to the Obsidian vault. Not `~`-expanded — the shell
    /// expands `~` for CLI args, but a config file value is read as-is.
    pub vault: Option<PathBuf>,
}

/// Where `config.toml` lives: `~/.config/obg/config.toml` on Linux (via
/// `directories`' XDG conventions), the platform equivalent elsewhere.
pub fn config_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "obg").map(|dirs| dirs.config_dir().join("config.toml"))
}

/// Reads the config file if it exists. A missing file is not an error —
/// it just means no config-supplied defaults.
pub fn load(path: &PathBuf) -> Result<Config, String> {
    if !path.exists() {
        return Ok(Config::default());
    }

    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("couldn't read config file {}: {e}", path.display()))?;

    toml::from_str(&contents)
        .map_err(|e| format!("couldn't parse config file {}: {e}", path.display()))
}

/// CLI arg wins over config file.
pub fn resolve_vault(cli: Option<PathBuf>, config: Option<PathBuf>) -> Option<PathBuf> {
    cli.or(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_wins_when_both_present() {
        let cli = Some(PathBuf::from("/from/cli"));
        let config = Some(PathBuf::from("/from/config"));
        assert_eq!(resolve_vault(cli, config), Some(PathBuf::from("/from/cli")));
    }

    #[test]
    fn falls_back_to_config_when_cli_absent() {
        let config = Some(PathBuf::from("/from/config"));
        assert_eq!(resolve_vault(None, config.clone()), config);
    }

    #[test]
    fn cli_only() {
        let cli = Some(PathBuf::from("/from/cli"));
        assert_eq!(resolve_vault(cli.clone(), None), cli);
    }

    #[test]
    fn neither_present() {
        assert_eq!(resolve_vault(None, None), None);
    }
}
