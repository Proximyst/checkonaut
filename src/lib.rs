use clap::{Parser, Subcommand};
use eyre::Result;

mod check;
mod test;

/// A tool for running checks against arbitrary JSON-like data.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// The logger configuration.
    #[arg(long, env = "RUST_LOG")]
    pub logger: Option<String>,

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
        match self.command {
            Command::Check(cmd) => cmd.run()?,
            Command::Test(cmd) => cmd.run()?,
        }

        Ok(())
    }
}
