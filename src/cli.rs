//! 命令行接口定义。

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "mdp",
    version,
    about = "macOS multi-display layouts: save solutions, switch primary, apply in one key",
    long_about = "macOS multi-display layout manager.\n\n\
Run with no arguments to open an interactive picker (arrows to move, Enter applies).\n\
Solutions live in ~/.config/mdp/solutions.toml and record only positions and the primary display."
)]
pub struct Cli {
    /// Solution file path (default ~/.config/mdp/solutions.toml, override with $MDP_CONFIG)
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Emit JSON for scripting
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Save the current display layout as a solution
    Store {
        /// Solution name
        name: String,
        /// Optional description (multiple words are joined with spaces)
        #[arg(trailing_var_arg = true, value_name = "description")]
        desc: Vec<String>,
        /// Overwrite an existing solution with the same name
        #[arg(short, long)]
        force: bool,
    },

    /// Apply a solution
    #[command(visible_alias = "use")]
    Apply {
        name: String,
        /// Show what would change without applying it
        #[arg(long)]
        dry_run: bool,
        /// Only for the current login session, do not persist
        #[arg(long)]
        session: bool,
    },

    /// List all solutions (* marks the one currently in use)
    #[command(visible_alias = "ls")]
    List,

    /// Show the current display layout
    Current,

    /// Delete a solution
    #[command(visible_aliases = ["rm", "remove"])]
    Delete { name: String },

    /// Rename a solution
    Rename { from: String, to: String },

    /// Show one solution in detail
    Show { name: String },

    /// Open the solution file in $EDITOR
    Edit {
        /// Optional: validate that this solution exists first
        name: Option<String>,
    },

    /// Print the solution file path
    Path,
}
