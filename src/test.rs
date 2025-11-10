use clap::Args;
use eyre::{Result, bail};

#[derive(Debug, Args)]
pub struct Test {}

impl Test {
    pub fn run(self) -> Result<()> {
        bail!("not yet implemented")
    }
}
