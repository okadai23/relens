use anyhow::Result;
use clap::Parser;
use relens::{cli::Cli, commands, output};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet);
    let result = commands::execute(cli.command)?;
    if !cli.quiet {
        println!("{}", output::render(&result, cli.output)?);
    }
    Ok(())
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
