use clap::{Parser, Subcommand};
use eyre::{Context, Result};

mod check;
mod file;
mod lua;
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

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;
    use eyre::{Context, ContextCompat, Result};
    use std::fs;
    use test_temp_dir::test_temp_dir;
    use tracing_test::traced_test;

    #[test]
    fn test_check_always_valid() -> Result<()> {
        const SCRIPT: &str = r#"
            function Check()
                return { }
            end
        "#;
        let dir = test_temp_dir!();
        fs::write(dir.as_path_untracked().join("script.lua"), SCRIPT)?;
        fs::write(
            dir.as_path_untracked().join("data.json"),
            r#"{"foo": "bar"}"#,
        )?;

        let cmd = Cli::try_parse_from([
            "unittest",
            "check",
            "--",
            dir.as_path_untracked()
                .to_str()
                .wrap_err("non UTF-8 test dir")?,
        ])
        .wrap_err("failed to parse args")?;
        cmd.run().wrap_err("failed to run check")?;

        Ok(())
    }

    #[test]
    fn test_test_always_valid() -> Result<()> {
        const SCRIPT: &str = r#"
            function Check()
                return { }
            end
        "#;
        const TEST_SCRIPT: &str = r#"
            require("script")
            function TestCheck()
                local result = Check()
                assert(type(result) == "table")
                assert(#result == 0)
            end
        "#;
        let dir = test_temp_dir!();
        fs::write(dir.as_path_untracked().join("script.lua"), SCRIPT)?;
        fs::write(dir.as_path_untracked().join("script_test.lua"), TEST_SCRIPT)?;

        let cmd = Cli::try_parse_from([
            "unittest",
            "test",
            "--",
            dir.as_path_untracked()
                .to_str()
                .wrap_err("non UTF-8 test dir")?,
        ])
        .wrap_err("failed to parse args")?;
        cmd.run().wrap_err("failed to run check")?;

        Ok(())
    }

    #[test]
    fn test_check_can_error() -> Result<()> {
        const SCRIPT: &str = r#"
            function Check()
                error("shouldn't happen :( but some people might do this")
            end
        "#;
        let dir = test_temp_dir!();
        fs::write(dir.as_path_untracked().join("script.lua"), SCRIPT)?;
        fs::write(
            dir.as_path_untracked().join("data.json"),
            r#"{"foo": "bar"}"#,
        )?;

        let cmd = Cli::try_parse_from([
            "unittest",
            "check",
            "--",
            dir.as_path_untracked()
                .to_str()
                .wrap_err("non UTF-8 test dir")?,
        ])
        .wrap_err("failed to parse args")?;
        let res = cmd.run();
        assert!(res.is_err(), "expected error but got success");
        let formatted = format!("{res:?}");
        assert!(formatted.contains("runtime error:"));
        assert!(formatted.contains("shouldn't happen :("));

        Ok(())
    }

    #[test]
    #[traced_test]
    fn test_test_can_error() -> Result<()> {
        const SCRIPT: &str = r#"
            function Check()
                error("shouldn't happen :( but some people might do this")
            end
        "#;
        const TEST_SCRIPT: &str = r#"
            require("script")
            function TestCheck()
                Check()
                error("unreachable")
            end
        "#;
        let dir = test_temp_dir!();
        fs::write(dir.as_path_untracked().join("script.lua"), SCRIPT)?;
        fs::write(dir.as_path_untracked().join("script_test.lua"), TEST_SCRIPT)?;

        let cmd = Cli::try_parse_from([
            "unittest",
            "test",
            "--",
            dir.as_path_untracked()
                .to_str()
                .wrap_err("non UTF-8 test dir")?,
        ])
        .wrap_err("failed to parse args")?;
        let res = cmd.run();
        assert!(res.is_err(), "expected error but got success");
        assert!(logs_contain("shouldn't happen :("));
        assert!(!logs_contain("unreachable"));

        Ok(())
    }
}
