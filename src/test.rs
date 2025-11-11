use crate::{
    file::{FileSearchResult, FileSearcher},
    lua::SourceCode,
};
use clap::Args;
use eyre::{Context, Result, ensure};
use mlua::Lua;
use rayon::prelude::*;
use std::path::PathBuf;
use tracing::error;

#[derive(Debug, Args)]
pub struct Test {
    /// The check test files or directories to test.
    /// We only process files ending in `_test.lua`.
    ///
    /// Files starting with a period (`.`) are ignored by default.
    #[arg(default_value = ".")]
    input: Vec<PathBuf>,

    /// Enable processing of files starting with a period.
    #[arg(long)]
    dotfiles: bool,
}

impl Test {
    pub fn run(self) -> Result<()> {
        let FileSearchResult {
            check_files: _,
            test_files,
            data_files: _,
        } = FileSearcher::default()
            .include_dotfiles(self.dotfiles)
            .include_dotdirs(self.dotfiles)
            .include_test_files(true)
            .search(self.input.into_par_iter())
            .wrap_err("failed to search input paths for relevant files")?;

        #[derive(Debug, Clone)]
        struct TestResult {
            file: PathBuf,
            errors: Vec<String>,
        }
        let mut results = test_files
            .into_par_iter()
            .map(|file| {
                let f2 = file.clone();
                Ok(TestResult {
                    errors: test_file(file).wrap_err_with(|| {
                        format!("while testing file {:?}", f2.to_string_lossy())
                    })?,
                    file: f2,
                })
            })
            .filter(|r| match r {
                Err(_) => true,
                Ok(res) => !res.errors.is_empty(),
            })
            .collect::<Result<Vec<_>>>()?;
        results.sort_unstable_by_key(|r| r.file.clone());
        for res in &results {
            for error in &res.errors {
                error!(file = ?res.file, %error, "test failure");
            }
        }
        ensure!(results.is_empty(), "one or more tests failed");
        Ok(())
    }
}

fn test_file(path: PathBuf) -> Result<Vec<String>> {
    let source = SourceCode::read(&path).wrap_err("failed to read test source file")?;
    let lua = Lua::new();
    source
        .load_into(&lua)
        .wrap_err("failed to load source code into Lua")?;

    Ok(source
        .call_test_functions(&lua)
        .wrap_err("failed to run test functions")?)
}
