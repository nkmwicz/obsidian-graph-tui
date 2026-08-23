mod cli;
mod config;
mod graph;
mod layout;
mod render;
mod vault;

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

    let vault_path = vault.canonicalize().unwrap_or(vault);
    println!("Vault: {}", vault_path.display());

    let parsed = match vault::parse(&vault_path) {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    println!("Notes: {}", parsed.notes.len());
    println!("Edges: {}", parsed.edges.len());
    println!("Unresolved links: {}", parsed.unresolved_links);
    println!("Orphan notes: {}", parsed.orphan_count());

    let graph = graph::build(parsed);
    println!("Graph nodes: {}", graph.node_count());
    println!("Graph edges: {}", graph.edge_count());

    let positions = layout::layout(&graph);
    println!("Laid out {} node positions", positions.len());

    if let Err(e) = render::run(&graph, &positions) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
