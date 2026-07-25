use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    #[arg(long, value_enum, default_value_t = OutputFormat::Human, global = true)]
    pub output: OutputFormat,
    #[arg(long, global = true)]
    pub quiet: bool,
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto, global = true)]
    pub color: ColorChoice,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a starter configuration file.
    Init {
        #[arg(default_value = "relens.toml")]
        path: PathBuf,
    },
    /// Validate and inspect a configuration file.
    Run {
        #[arg(default_value = "relens.toml")]
        path: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}
