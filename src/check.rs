use clap::Args;
use eyre::{Context, Result, bail, ensure, eyre};
use mlua::{FromLua, Function, Lua, LuaSerdeExt};
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
        let input_files = self
            .input
            .into_par_iter()
            .flat_map(|p| find_files(p, self.dotfiles))
            // TODO: Make an option to not collect all into memory at once...
            .collect::<Result<Vec<_>>>()?;
        let (lua_files, data_files) = {
            let mut lua_files = Vec::new();
            let mut data_files = Vec::new();
            for file in input_files {
                if file
                    .extension()
                    .and_then(|e| e.to_str())
                    .map_or(false, |s| s.eq_ignore_ascii_case("lua"))
                    && !file
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map_or(false, |n| n.ends_with("_test.lua"))
                {
                    if has_check_function(&file).wrap_err_with(|| {
                        format!(
                            "checking Lua file for Check function: {}",
                            file.to_string_lossy()
                        )
                    })? {
                        lua_files.push(file);
                    }
                } else {
                    data_files.push(file);
                }
            }
            (lua_files, data_files)
        };

        ensure!(lua_files.len() > 0, "no check files found to run");
        ensure!(data_files.len() > 0, "no data files found to check");
        // We now have all the Lua files (i.e. checks) and all the data files we want to run on.

        #[derive(Debug, Clone)]
        struct EvalResult {
            data_file: PathBuf,
            /// The errors in a tuple of `(check_file, errors)`.
            /// If no errors are found for a check, it won't be included.
            errors: Vec<(PathBuf, Vec<CheckError>)>,
        }
        let mut results: Vec<EvalResult> = data_files
            .into_par_iter()
            .map(|file| {
                let f2 = file.clone();
                Ok(EvalResult {
                    errors: check_file(file, &lua_files).wrap_err_with(|| {
                        format!("checking data file: {}", f2.to_string_lossy())
                    })?,
                    data_file: f2,
                })
            })
            .collect::<Result<Vec<EvalResult>>>()?;
        results.sort_unstable_by_key(|e| e.data_file.clone());
        let mut found_error = false;
        for res in results {
            let path = res.data_file.to_string_lossy();
            for (check, errs) in res.errors {
                let (errors, warnings) = errs
                    .iter()
                    .partition::<Vec<_>, _>(|e| e.severity == CheckSeverity::Error);
                found_error |= !errors.is_empty();
                let check = check.to_string_lossy();
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

fn check_file(
    file: impl AsRef<Path>,
    checks: &[PathBuf],
) -> Result<Vec<(PathBuf, Vec<CheckError>)>> {
    let file = file.as_ref();
    let path_str = file
        .to_str()
        .ok_or_else(|| eyre!("invalid UTF-8 in path: {}", file.to_string_lossy()))?;
    let parent_str = match file.parent() {
        None => "/",
        Some(p) => p
            .to_str()
            .ok_or_else(|| eyre!("invalid UTF-8 in path: {}", file.to_string_lossy()))?,
    };

    // TODO: DRY the lua init
    let lua = Lua::new();
    lua.globals()
        .set("_CHECK_FILE", path_str)
        .map_err(|e| eyre!("failed to set _CHECK_FILE global: {e}"))?;
    lua.globals()
        .set("_CHECK_DIR", parent_str)
        .map_err(|e| eyre!("failed to set _CHECK_DIR global: {e}"))?;
    lua.load(r#"package.path = package.path .. ";" .. _CHECK_DIR .. "/?.lua;" .. _CHECK_DIR .. "/?/init.lua" "#)
        .exec()
        .map_err(|e| eyre!("failed to set package.path: {e}"))?;

    let documents = {
        let data = std::fs::read(&file).wrap_err("failed to read data file")?;
        let ext = file.extension().and_then(|e| e.to_str());
        if ext.map_or(false, |s| s.eq_ignore_ascii_case("json")) {
            // We have a simple JSON document: there is only 1 document per file.
            let value: serde_json::Value =
                serde_json::from_slice(&data).wrap_err("failed to parse JSON")?;
            let value = lua
                .to_value(&value)
                .map_err(|e| eyre!("failed to serialize JSON to Lua value: {e}"))
                .wrap_err("failed to convert JSON to Lua value")?;
            vec![value]
        } else if ext.map_or(false, |s| s.eq_ignore_ascii_case("toml")) {
            // We have a simple TOML document: there is only 1 document per file.
            let value: serde_json::Value =
                toml::from_slice(&data).wrap_err("failed to parse TOML")?;
            let value = lua
                .to_value(&value)
                .map_err(|e| eyre!("failed to serialize TOML to Lua value: {e}"))
                .wrap_err("failed to convert TOML to Lua value")?;
            vec![value]
        } else if ext.map_or(false, |s| {
            s.eq_ignore_ascii_case("yml") || s.eq_ignore_ascii_case("yaml")
        }) {
            // We may have multiple YAML documents in a single file.
            let mut deserializer = serde_norway::Deserializer::from_slice(&data);
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
            values
        } else {
            bail!(
                "unrecognised file extension for file {}",
                file.to_string_lossy(),
            );
        }
    };

    fn perform_check(lua: Lua, documents: &[mlua::Value], check: &Path) -> Result<Vec<CheckError>> {
        let check_contents =
            std::fs::read_to_string(check).wrap_err("failed to read check file")?;
        lua.load(&check_contents)
            .set_name(format!("@{}", &check.to_string_lossy()))
            .exec()
            .map_err(|e| eyre!("failed to load check Lua script: {e}"))?;

        let check_function: Function = lua
            .globals()
            .get("Check")
            .map_err(|e| eyre!("failed to get 'Check' function from Lua script: {e}"))?;
        let mut errors = Vec::new();
        for doc in documents {
            let res: CheckResult = check_function
                .call(doc)
                .map_err(|e| eyre!("failed to execute 'Check' function: {e}"))?;
            errors.extend(res.flatten());
        }

        Ok(errors)
    }

    let mut results = Vec::new();
    // TODO: Test with parallelism of checks as well?
    for check in checks {
        let res = perform_check(lua.clone(), &documents, check)
            .wrap_err_with(|| format!("failed to run check: {}", check.to_string_lossy()))?;
        if !res.is_empty() {
            results.push((check.clone(), res));
        }
    }

    Ok(results)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
enum CheckResult {
    /// Nil represents a result to be ignored.
    Nil,
    /// A single error result.
    Error {
        severity: Option<CheckSeverity>,
        error: String,
    },
    /// A wrapper around multiple error results (or potentially nils).
    Many {
        severity: Option<CheckSeverity>,
        results: Vec<CheckResult>,
    },
}

impl CheckResult {
    fn flatten(self) -> Vec<CheckError> {
        let mut acc = Vec::new();
        self.flatten_internal(&mut acc, CheckSeverity::Error);
        acc
    }

    fn flatten_internal(self, acc: &mut Vec<CheckError>, inherited_severity: CheckSeverity) {
        match self {
            Self::Nil => {}
            Self::Error { severity, error } => acc.push(CheckError {
                severity: severity.unwrap_or(inherited_severity),
                error,
            }),
            Self::Many { severity, results } => {
                let severity = severity.unwrap_or(inherited_severity);
                for result in results {
                    result.flatten_internal(acc, severity);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // we use Debug to print this
struct CheckError {
    severity: CheckSeverity,
    error: String,
}

impl FromLua for CheckResult {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        match value {
            mlua::Value::Nil => Ok(CheckResult::Nil),

            mlua::Value::String(s) => {
                let error = s.to_str()?.to_string();
                Ok(CheckResult::Error {
                    severity: None,
                    error,
                })
            }

            mlua::Value::Table(table) => {
                // A table can exist for multiple reasons:
                //   * We can have a sequence of errors (i.e., a vec).
                //   * We can have a dictionary with a "message" and optionally "severity" (i.e., a
                //     single error). The message can be either a string, or a vec of strings (or
                //     even nil).

                if !table.contains_key("message")? {
                    // If we have no "message" key, we'll assume it's a sequence of errors.
                    let mut results = Vec::new();
                    for pair in table.sequence_values::<mlua::Value>() {
                        let pair = pair?;
                        match pair {
                            mlua::Value::Nil => results.push(CheckResult::Nil),
                            mlua::Value::String(s) => results.push(CheckResult::Error {
                                severity: None,
                                error: s.to_str()?.to_string(),
                            }),
                            otherwise => results.push(CheckResult::from_lua(otherwise, lua)?),
                        }
                    }
                    Ok(Self::Many {
                        severity: None,
                        results,
                    })
                } else {
                    let severity: Option<String> = table.get("severity")?;
                    let severity = match severity.as_deref() {
                        None => None,
                        Some("error") => Some(CheckSeverity::Error),
                        Some("warning") => Some(CheckSeverity::Warning),
                        Some(other) => {
                            return Err(mlua::Error::FromLuaConversionError {
                                from: "string",
                                to: "CheckSeverity".into(),
                                message: Some(format!("invalid severity level: {}", other)),
                            });
                        }
                    };
                    let error: String = table.get("message")?;
                    Ok(CheckResult::Error { severity, error })
                }
            }
            _ => Err(mlua::Error::FromLuaConversionError {
                from: value.type_name(),
                to: "CheckError".into(),
                message: Some("expected string or table".into()),
            }),
        }
    }
}

fn has_check_function(file: impl AsRef<Path>) -> Result<bool> {
    let file = file.as_ref();
    let lua = Lua::new();
    let check_contents = std::fs::read_to_string(file).wrap_err("failed to read check file")?;
    lua.load(&check_contents)
        .set_name(format!("@{}", &file.to_string_lossy()))
        .exec()
        .map_err(|e| eyre!("failed to load check Lua script: {e}"))?;

    match lua.globals().get::<mlua::Function>("Check") {
        Ok(_) => Ok(true),
        Err(mlua::Error::FromLuaConversionError { .. }) => Ok(false),
        Err(e) => Err(eyre!("failed to get 'Check' function from Lua script: {e}")),
    }
}
