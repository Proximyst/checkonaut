use clap::Args;
use eyre::{Context, Result, bail, ensure, eyre};
use mlua::{Lua, LuaSerdeExt};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
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
        let input_files = self
            .input
            .into_par_iter()
            .flat_map(|p| find_files(p, self.dotfiles))
            .filter(|res| match res {
                Ok(path) => path.file_name().map_or(false, |name| {
                    name.to_string_lossy()
                        .to_ascii_lowercase()
                        .ends_with("_test.lua")
                }),
                Err(_) => true, // keep errors for later handling
            })
            .collect::<Result<Vec<_>>>()?;

        #[derive(Debug, Clone)]
        struct TestResult {
            file: PathBuf,
            errors: Vec<String>,
        }
        let mut results = input_files
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

// TODO: We could make this into a util function somewhere.
fn find_files(
    path: impl AsRef<Path>,
    dotfiles: bool,
) -> impl ParallelIterator<Item = Result<PathBuf>> {
    walkdir::WalkDir::new(path)
        .min_depth(1) // don't return the root itself
        .into_iter()
        .par_bridge()
        .filter_map(move |entry| match entry {
            Ok(entry) => {
                if !dotfiles && entry.file_name().to_string_lossy().starts_with('.') {
                    None
                } else if entry.file_type().is_file() {
                    Some(Ok(entry.path().to_path_buf()))
                } else {
                    // we get notified of directories too, so let's just skip 'em
                    None
                }
            }

            Err(err) => Some(Err(err.into())),
        })
}

fn test_file(path: PathBuf) -> Result<Vec<String>> {
    let path_str = path
        .to_str()
        .ok_or_else(|| eyre!("invalid UTF-8 in path: {}", path.to_string_lossy()))?;
    let parent_str = match path.parent() {
        None => "/",
        Some(p) => p
            .to_str()
            .ok_or_else(|| eyre!("invalid UTF-8 in path: {}", path.to_string_lossy()))?,
    };

    let contents = std::fs::read_to_string(path_str)?;
    let lua = Lua::new();
    lua.globals()
        .set("_TEST_FILE", path_str)
        .map_err(|e| eyre!("failed to set _TEST_FILE global: {e}"))?;
    lua.globals()
        .set("_TEST_DIR", parent_str)
        .map_err(|e| eyre!("failed to set _TEST_DIR global: {e}"))?;
    lua.load(r#"package.path = package.path .. ";" .. _TEST_DIR .. "/?.lua;" .. _TEST_DIR .. "/?/init.lua" "#)
        .exec()
        .map_err(|e| eyre!("failed to set package.path: {e}"))?;

    lua.load(&contents)
        .set_name(format!("@{path_str}"))
        .exec()
        .map_err(|e| eyre!("failed to load test Lua script: {e}"))?;

    let mut results = Vec::new();
    for pair in lua.globals().pairs::<mlua::Value, mlua::Value>() {
        let (k, v) = pair.map_err(|e| eyre!("failed to iterate over Lua globals: {e}"))?;
        let Some(v) = v.as_function() else { continue };
        let k = k
            .as_string()
            .ok_or_else(|| eyre!("expected string key for a function value"))?;
        if !k.to_string_lossy().starts_with("Test") {
            continue;
        }

        match v.call::<mlua::Value>(()) {
            Ok(mlua::Value::Nil) => {}
            Ok(val) => {
                let json: serde_json::Value = lua.from_value(val).map_err(|e| {
                    eyre!(
                        "failed to convert return value of test function '{}' to JSON: {}",
                        k.to_string_lossy(),
                        e
                    )
                })?;
                let json = serde_json::to_string(&json)
                    .wrap_err("failed to convert serde_json::Value to string")?;
                results.push(format!("{}: {json}", k.display()));
            }
            Err(mlua::Error::RuntimeError(e)) if e.contains("assertion failed!") => {
                results.push(format!("{}: {e}", k.display()));
            }
            Err(e) => {
                return Ok(vec![format!(
                    "Test function '{}' failed: {}",
                    k.to_string_lossy(),
                    e
                )]);
            }
        }
    }
    Ok(results)
}
