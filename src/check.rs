use crate::{
    file::{FileSearchResult, FileSearcher},
    lua::{CheckError, CheckSeverity, SourceCode},
};
use clap::Args;
use eyre::{Context, Result, bail, ensure, eyre};
use mlua::{Lua, LuaSerdeExt};
use rayon::prelude::*;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};

#[derive(Debug, Args)]
pub struct Check {
    /// The data files or directories to check with.
    ///
    /// Data files are files with the extensions `.json`, `.yml`, `.yaml`, or `.toml`.
    /// Check files are files with the extension `.lua`. `_test.lua` files are ignored.
    ///
    /// Files starting with a period (`.`) are ignored by default.
    #[arg(default_value = ".")]
    input: Vec<PathBuf>,

    /// Enable processing of files starting with a period.
    #[arg(long)]
    dotfiles: bool,
}

impl Check {
    pub fn run(self) -> Result<()> {
        let FileSearchResult {
            check_files,
            test_files: _,
            data_files,
        } = FileSearcher::default()
            .include_dotfiles(self.dotfiles)
            .include_dotdirs(self.dotfiles)
            .include_check_files(true)
            .include_data_files(true)
            .search(self.input.into_par_iter())
            .wrap_err("failed to search input paths for relevant files")?;

        let check_files = check_files
            .into_par_iter()
            .map(|p| {
                SourceCode::read(&p)
                    .wrap_err_with(|| format!("reading check file: {}", p.display()))
            })
            .filter_map(|src| {
                match src.and_then(|src| src.has_check_function().map(|b| b.then_some(src))) {
                    Ok(Some(src)) => Some(Ok(src)),
                    Ok(None) => None,
                    Err(e) => Some(Err(e)),
                }
            })
            .collect::<Result<Vec<_>>>()?;

        ensure!(check_files.len() > 0, "no check files found to run");
        ensure!(data_files.len() > 0, "no data files found to check");
        // We now have all the Lua files (i.e. checks) and all the data files we want to run on.

        #[derive(Debug, Clone)]
        struct EvalResult<'a> {
            data_file: PathBuf,
            /// The errors in a tuple of `(check_file, errors)`.
            /// If no errors are found for a check, it won't be included.
            errors: Vec<(&'a SourceCode, Vec<CheckError>)>,
        }
        let mut results: Vec<EvalResult> = data_files
            .into_par_iter()
            .map(|file| {
                let f2 = file.clone();
                Ok(EvalResult {
                    errors: check_file(file, &check_files)
                        .wrap_err_with(|| format!("checking data file: {}", f2.display()))?,
                    data_file: f2,
                })
            })
            .collect::<Result<Vec<EvalResult>>>()?;
        results.sort_unstable_by_key(|e| e.data_file.clone());
        let mut found_error = false;
        for res in results {
            let path = res.data_file.display();
            for (check, errs) in res.errors {
                let (errors, warnings) = errs
                    .iter()
                    .partition::<Vec<_>, _>(|e| e.severity == CheckSeverity::Error);
                found_error |= !errors.is_empty();
                let check = check.path.display();
                if !errors.is_empty() {
                    error!(%path, count = errors.len(), ?errors, %check, "errors found by check");
                }
                if !warnings.is_empty() {
                    warn!(%path, count = warnings.len(), ?warnings, %check, "warnings found by check");
                }
            }
        }
        ensure!(!found_error, "one or more errors were found during checks");
        info!("no errors found");
        Ok(())
    }
}

fn check_file(
    file: impl AsRef<Path>,
    checks: &[SourceCode],
) -> Result<Vec<(&SourceCode, Vec<CheckError>)>> {
    let file = file.as_ref();
    let lua = Lua::new();
    let documents = parse_data(&lua, file).wrap_err("failed to parse data file")?;

    fn perform_check(
        lua: Lua,
        doc_file: &Path,
        documents: &[mlua::Value],
        check: &SourceCode,
    ) -> Result<Vec<CheckError>> {
        check.load_into(&lua).wrap_err_with(|| {
            format!(
                "failed to load check source code from file: {}",
                check.path.display()
            )
        })?;

        let context = lua
            .create_table_from([
                ("check_file", check.path.to_string_lossy()),
                ("document_file", doc_file.to_string_lossy()),
            ])
            .map_err(|e| eyre!("failed to create context table: {e}"))?;
        let context = mlua::Value::Table(context);

        let mut errors = Vec::new();
        for doc in documents {
            let res = check.call_check_function(&lua, doc, &context)?;
            errors.extend(res);
        }

        Ok(errors)
    }

    let mut results = Vec::new();
    // TODO: Test with parallelism of checks as well?
    for check in checks {
        let res = perform_check(lua.clone(), file, &documents, check)
            .wrap_err_with(|| format!("failed to run check: {}", check.path.display()))?;
        if !res.is_empty() {
            results.push((check, res));
        }
    }

    Ok(results)
}

fn parse_data(lua: &Lua, path: &Path) -> Result<Vec<mlua::Value>> {
    let contents = std::fs::read(path).wrap_err("failed to read data file")?;
    let ext = path.extension().and_then(|e| e.to_str());
    if ext.map_or(false, |s| s.eq_ignore_ascii_case("json")) {
        // We have a simple JSON document: there is only 1 document per file.
        let value: serde_json::Value =
            serde_json::from_slice(&contents).wrap_err("failed to parse JSON")?;
        let value = lua
            .to_value(&value)
            .map_err(|e| eyre!("failed to serialize JSON to Lua value: {e}"))
            .wrap_err("failed to convert JSON to Lua value")?;
        Ok(vec![value])
    } else if ext.map_or(false, |s| s.eq_ignore_ascii_case("toml")) {
        // We have a simple TOML document: there is only 1 document per file.
        let value: serde_json::Value =
            toml::from_slice(&contents).wrap_err("failed to parse TOML")?;
        let value = lua
            .to_value(&value)
            .map_err(|e| eyre!("failed to serialize TOML to Lua value: {e}"))
            .wrap_err("failed to convert TOML to Lua value")?;
        Ok(vec![value])
    } else if ext.map_or(false, |s| {
        s.eq_ignore_ascii_case("yml") || s.eq_ignore_ascii_case("yaml")
    }) {
        // We may have multiple YAML documents in a single file.
        let mut deserializer = serde_norway::Deserializer::from_slice(&contents);
        let mut values = Vec::with_capacity(1);
        while let Some(de) = deserializer.next() {
            let value: serde_json::Value =
                serde_json::Value::deserialize(de).wrap_err("failed to parse YAML document")?;
            let value = lua
                .to_value(&value)
                .map_err(|e| eyre!("failed to serialize YAML to Lua value: {e}"))
                .wrap_err("failed to convert YAML to Lua value")?;
            values.push(value);
        }
        Ok(values)
    } else {
        bail!("unrecognised file extension")
    }
}
