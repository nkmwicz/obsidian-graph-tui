use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "obg",
    about = "Terminal-native 3D graph viewer for an Obsidian vault"
)]
pub struct Cli {
    /// Path to the Obsidian vault. Falls back to `vault` in the config
    /// file if omitted.
    pub vault: Option<PathBuf>,
}
