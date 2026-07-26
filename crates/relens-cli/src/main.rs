use anyhow::Result;
use clap::Parser;
use relens_cli::{cli::Cli, commands, output};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet);
    match commands::execute(cli.command) {
        Ok(result) => {
            if !cli.quiet {
                println!("{}", output::render_success(&result, cli.output)?);
            }
            Ok(())
        }
        Err(error) => {
            if let Some(rendered) = output::render_known_failure(&error, cli.output)? {
                println!("{rendered}");
            }
            Err(error)
        }
    }
}

fn init_tracing(verbose: u8, quiet: bool) {
    let default = if quiet {
        "off"
    } else if verbose > 0 {
        "debug"
    } else {
        "warn"
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
