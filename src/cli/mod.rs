//! Nyquest CLI — Interactive installer, configurator, and management commands.

pub mod config_cmd;
pub mod doctor;
pub mod install;
pub mod preflight;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "nyquest",
    version,
    about = "Nyquest — AI Prompt Compression Engine"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Path to config file
    #[arg(long, short, global = true, default_value = "nyquest.yaml")]
    pub config: String,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the proxy server (default behavior)
    Serve,

    /// Run the interactive setup wizard
    Install {
        /// Non-interactive: accept all defaults
        #[arg(long)]
        defaults: bool,

        /// Override settings: --set key=value (repeatable)
        #[arg(long = "set", value_name = "KEY=VALUE")]
        overrides: Vec<String>,
    },

    /// Reconfigure settings (re-opens the wizard)
    Configure {
        /// Configure only a specific section
        #[arg(long, short)]
        section: Option<String>,
    },

    /// Validate configuration and test connectivity
    Doctor,

    /// Show Nyquest engine status
    Status,

    /// Full system requirements check (hardware, GPU, dependencies, semantic stage)
    Preflight {
        /// Show all details including passing checks
        #[arg(long, short)]
        verbose: bool,
    },

    /// Manage configuration values
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Show the current resolved configuration
    Show,
    /// Get a single config value
    Get {
        /// Config key (e.g. port, compression_level, providers.anthropic.api_key)
        key: String,
    },
    /// Set a single config value
    Set {
        /// Config key
        key: String,
        /// New value
        value: String,
    },
}
