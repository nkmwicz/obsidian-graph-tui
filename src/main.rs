mod cli;
mod config;

use clap::Parser;

use cli::Cli;

fn main() {
    let cli = Cli::parse();

    let config = match config::config_path() {
        Some(path) => match config::load(&path) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        None => config::Config::default(),
    };

    let Some(vault) = config::resolve_vault(cli.vault, config.vault) else {
        let config_path = config::config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<config dir unavailable>".to_string());
        eprintln!(
            "error: no vault path provided.\n\
             Pass one as an argument (`obg <path>`) or set `vault = \"...\"` in {config_path}"
        );
        std::process::exit(1);
    };

    if !vault.is_dir() {
        eprintln!(
            "error: vault path {} does not exist or is not a directory",
            vault.display()
        );
        std::process::exit(1);
    }

    let vault = vault.canonicalize().unwrap_or(vault);
    println!("Vault: {}", vault.display());
}
