use anyhow::{Context, Result};
use relens_domain::CommandResult;

use crate::cli::Command;

pub fn execute(command: Command) -> Result<CommandResult> {
    match command {
        Command::Init { path } => {
            relens_store::initialize(&path).context("failed to initialize relens")
        }
        Command::Run { path } => relens_store::inspect(&path).context("failed to run relens"),
    }
}
