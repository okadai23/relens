use anyhow::{Error, Result};
use relens_domain::CommandResult;
use serde::Serialize;

use crate::{
    cli::OutputFormat,
    commands::{ExportVerification, UpdateConflict},
};

/// A stable presentation model for both successful commands and expected failures.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CommandOutput<'a> {
    Success { action: &'a str, path: &'a str },
    Conflict { files: &'a [String] },
    VerificationFailed { locations: &'a str },
}

pub fn render_success(result: &CommandResult, format: OutputFormat) -> Result<String> {
    render(
        &CommandOutput::Success {
            action: result.action,
            path: &result.path,
        },
        format,
    )
}

/// Converts an expected command failure into contract output. Unknown errors are
/// diagnostics only and therefore return `None`.
pub fn render_known_failure(error: &Error, format: OutputFormat) -> Result<Option<String>> {
    let output = if let Some(conflict) = error.downcast_ref::<UpdateConflict>() {
        CommandOutput::Conflict {
            files: &conflict.files,
        }
    } else if let Some(failure) = error.downcast_ref::<ExportVerification>() {
        CommandOutput::VerificationFailed {
            locations: &failure.locations,
        }
    } else {
        return Ok(None);
    };
    render(&output, format).map(Some)
}

fn render(output: &CommandOutput<'_>, format: OutputFormat) -> Result<String> {
    match (format, output) {
        (OutputFormat::Human, CommandOutput::Success { action, path }) => {
            Ok(format!("{action} {path}"))
        }
        (OutputFormat::Human, CommandOutput::Conflict { files }) => {
            Ok(format!("conflicts: {}", files.join(",")))
        }
        (OutputFormat::Human, CommandOutput::VerificationFailed { locations }) => {
            Ok(format!("verification failed at {locations}"))
        }
        (OutputFormat::Json, output) => Ok(serde_json::to_string(output)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_structured_known_failures() {
        let conflict = Error::new(UpdateConflict {
            files: vec!["README.md".into()],
        });
        let rendered = render_known_failure(&conflict, OutputFormat::Json)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&rendered).unwrap(),
            serde_json::json!({"status":"conflict","files":["README.md"]})
        );

        let verification = Error::new(ExportVerification {
            locations: "README.md:0..8".into(),
        });
        let rendered = render_known_failure(&verification, OutputFormat::Human)
            .unwrap()
            .unwrap();
        assert_eq!(rendered, "verification failed at README.md:0..8");
    }
}
