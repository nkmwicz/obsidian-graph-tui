use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "obg",
    about = "Terminal-native 3D graph viewer for an Obsidian vault"
)]
pub struct Cli {
    /// Path to the Obsidian vault. Falls back to `vault` in the config
    /// file if omitted.
    pub vault: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Query subcommands (Kùzu-backed, TODO.md Phase 6). Omitting a
/// subcommand entirely keeps the original behavior: render the graph.
#[derive(Subcommand)]
pub enum Command {
    /// Rank notes by PageRank (structural centrality).
    Pagerank {
        /// How many top-ranked notes to print.
        #[arg(long, default_value_t = 15)]
        top: usize,
    },
    /// Detect communities (Louvain) from link structure.
    Communities,
    /// Trace the shortest path between two notes (by title/basename,
    /// same resolution rules as wikilinks).
    Path {
        from: String,
        to: String,
    },
}
