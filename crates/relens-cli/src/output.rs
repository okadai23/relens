use anyhow::Result;
use relens_domain::CommandResult;

use crate::cli::OutputFormat;

pub fn render(result: &CommandResult, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Human => Ok(format!("{} {}", result.action, result.path)),
        OutputFormat::Json => Ok(serde_json::to_string(result)?),
    }
}
