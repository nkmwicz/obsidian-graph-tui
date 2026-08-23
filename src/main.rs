mod algo;
mod cli;
mod config;
mod graph;
mod layout;
mod render;
mod vault;

use std::path::Path;

use clap::Parser;

use cli::{Cli, Command};
use vault::resolve::{NoteIndex, Outcome};

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

    match cli.command {
        None => {
            let positions = layout::layout(&graph);
            println!("Laid out {} node positions", positions.len());

            if let Err(e) = render::run(&graph, &positions) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        Some(Command::Pagerank { top }) => {
            let ranked = algo::pagerank(&graph, top);
            println!("Top {} notes by PageRank:", ranked.len());
            for (path, rank) in ranked {
                println!("  {rank:.4}  {path}");
            }
        }
        Some(Command::Communities) => {
            for (id, members) in algo::communities(&graph) {
                println!("Community {id}: {}", members.join(", "));
            }
        }
        Some(Command::Path { from, to }) => {
            let note_paths: Vec<&Path> = graph.node_weights().map(|n| n.path.as_path()).collect();
            let index = NoteIndex::build(&note_paths);

            let resolve_one = |name: &str| match index.resolve(name) {
                Outcome::Resolved(i) => Some(i),
                _ => None,
            };

            let (Some(from_i), Some(to_i)) = (resolve_one(&from), resolve_one(&to)) else {
                eprintln!("error: couldn't resolve note name(s) \"{from}\" / \"{to}\"");
                std::process::exit(1);
            };

            let from_path = note_paths[from_i].to_string_lossy();
            let to_path = note_paths[to_i].to_string_lossy();

            match algo::shortest_path(&graph, &from_path, &to_path) {
                Some(hops) => println!("{}", hops.join(" -> ")),
                None => println!("no path found"),
            }
        }
    }
}
