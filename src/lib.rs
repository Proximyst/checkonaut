use clap::{Parser, Subcommand};
use eyre::{Context, Result};

mod check;
mod test;

/// A tool for running checks against arbitrary JSON-like data.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// The logger configuration.
    #[arg(long, env = "RUST_LOG")]
    pub logger: Option<String>,

    /// How many threads should Rayon use in processing?
    /// By default, this is the same amount as CPUs available.
    #[arg(long)]
    rayon_threads: Option<usize>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check that the given data conforms to the specified checks.
    Check(self::check::Check),

    /// Check that the given checks behave as expected against test cases.
    Test(self::test::Test),
}

impl Cli {
    pub fn run(self) -> Result<()> {
        if let Some(n) = self.rayon_threads {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build_global()
                .wrap_err("failed to set up Rayon thread pool")?;
        }

        match self.command {
            Command::Check(cmd) => cmd.run()?,
            Command::Test(cmd) => cmd.run()?,
        }

        Ok(())
    }
}
