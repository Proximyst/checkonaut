use clap::Parser;
use eyre::{Context, Result};

fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = checkonaut::Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(cli.logger.as_deref().unwrap_or("warn,checkonaut=info"))
        .try_init()
        .map_err(|e| eyre::eyre!(e))
        .wrap_err("failed to set up logging")?;
    cli.run()?;
    Ok(())
}
