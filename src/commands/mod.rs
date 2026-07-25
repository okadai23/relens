use anyhow::{Context, Result};

use crate::{CommandResult, cli::Command};

pub fn execute(command: Command) -> Result<CommandResult> {
    match command {
        Command::Init { path } => crate::initialize(&path).context("failed to initialize relens"),
        Command::Run { path } => crate::inspect(&path).context("failed to run relens"),
    }
}
