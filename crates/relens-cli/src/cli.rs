use std::{path::PathBuf, str::FromStr};

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "relens", author, version, about)]
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
    /// Render a new project from a template.
    New {
        template: PathBuf,
        #[arg(long, short = 'd', default_value = ".")]
        destination: PathBuf,
        #[arg(long = "answer", short = 'a', value_name = "NAME=VALUE")]
        answers: Vec<String>,
    },
    /// Report files changed since generation.
    Drift {
        #[arg(default_value = ".")]
        project: PathBuf,
    },
    /// Lift project drift into a verified template patch.
    Lift {
        #[arg(default_value = ".")]
        project: PathBuf,
        /// Resume the latest review session after applying explicit review decisions.
        #[arg(long)]
        resume: bool,
        /// Resolve a pending edit (for example, `0=keep-literal` or `0=substitute`).
        #[arg(long = "decision", value_name = "EDIT=CHOICE")]
        decisions: Vec<ReviewResolution>,
        /// Export the latest verified session to a Git branch.
        #[arg(long)]
        export: bool,
    },
    /// Update a generated project to the template repository's HEAD.
    Update {
        #[arg(default_value = ".")]
        project: PathBuf,
    },
    /// Render a pairwise answer matrix and report every invalid combination.
    Matrix {
        #[arg(default_value = ".")]
        template: PathBuf,
        /// Print the generated answer plan without rendering it.
        #[arg(long)]
        plan: bool,
    },
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

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ReviewChoice {
    KeepLiteral,
    Substitute,
}

#[derive(Clone, Copy, Debug)]
pub struct ReviewResolution {
    pub edit: usize,
    pub choice: ReviewChoice,
}

impl FromStr for ReviewResolution {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (edit, choice) = value
            .split_once('=')
            .ok_or_else(|| "expected EDIT=CHOICE".to_string())?;
        let edit = edit
            .parse()
            .map_err(|_| format!("invalid edit index `{edit}`"))?;
        let choice = ReviewChoice::from_str(choice, true).map_err(|_| {
            format!("invalid choice `{choice}`; expected keep-literal or substitute")
        })?;
        Ok(Self { edit, choice })
    }
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
