use eyre::{Context, ContextCompat, Result, bail, eyre};
use mlua::{FromLua, Function, Lua, LuaSerdeExt};
use std::{
    fmt,
    path::{Path, PathBuf},
};
use tracing::debug;

#[derive(Debug, Clone)]
pub struct SourceCode {
    pub path: PathBuf,
    name: String,
    contents: String,
}

impl SourceCode {
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let name = format!("@{}", path.to_string_lossy());
        let contents = std::fs::read_to_string(&path)
            .wrap_err_with(|| format!("failed to read source file: {}", path.display()))?;
        Ok(Self {
            path,
            name,
            contents,
        })
    }

    pub fn load_into(&self, to: &Lua) -> Result<()> {
        update_package_path(to, &self.path)?;
        self.checkonaut_module(to)
            .wrap_err("failed to load 'checkonaut' module")?;
        to.load(&self.contents)
            .set_name(&self.name)
            .exec()
            .map_err(|e| {
                eyre!(
                    "failed to load Lua source from '{}': {e}",
                    self.path.display(),
                )
            })?;
        Ok(())
    }

    pub fn has_check_function(&self) -> Result<bool> {
        let lua = new_lua_for(&self.path)?;
        self.checkonaut_module(&lua)
            .wrap_err("failed to load 'checkonaut' module")?;
        self.load_into(&lua)?;

        match lua.globals().get::<mlua::Function>("Check") {
            Ok(_) => Ok(true),
            Err(mlua::Error::FromLuaConversionError { .. }) => Ok(false),
            Err(e) => Err(eyre!("failed to check for 'Check' function: {e}")),
        }
    }

    /// Calls the `Check` function defined in the source code.
    ///
    /// You should call `load_into` before calling this function, otherwise there is no `Check`.
    /// You should only call this function if [`Self::has_check_function`] returns `true`.
    pub fn call_check_function(
        &self,
        lua: &Lua,
        document: &mlua::Value,
        context: &mlua::Value,
    ) -> Result<Vec<CheckError>> {
        let check_fn: Function = lua
            .globals()
            .get("Check")
            .map_err(|e| eyre!("failed to find 'Check' function in Lua state: {e}"))?;
        let result: CheckResult = check_fn
            .call((document, context))
            .map_err(|e| eyre!("could not call 'Check' function: {e}"))?;
        Ok(result.flatten())
    }

    /// Calls all `Test` functions defined in the source code.
    ///
    /// You should call `load_into` before calling this function, otherwise there are no `Test`
    /// functions.
    pub fn call_test_functions(&self, lua: &Lua) -> Result<Vec<String>> {
        let fln = self
            .path
            .file_name()
            .map(|s| s.display())
            .wrap_err("failed to find file name for test source code")?;
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
                    results.push(format!("{fln}/{}: {json}", k.to_string_lossy()));
                }
                Err(mlua::Error::RuntimeError(e)) => {
                    results.push(format!("{fln}/{}: {e}", k.to_string_lossy()));
                }
                Err(e) => {
                    bail!(
                        "failed to call test function '{}': {}",
                        k.to_string_lossy(),
                        e
                    );
                }
            }
        }
        Ok(results)
    }

    fn checkonaut_module(&self, lua: &Lua) -> Result<()> {
        let source_path = self.path.clone();

        let read_json = lua
            .create_function(move |l, path: mlua::String| {
                let path_str = path.to_str()?;
                let parent = source_path.parent().ok_or_else(|| {
                    mlua::Error::runtime(format!(
                        "cannot determine parent directory of '{}'",
                        source_path.display(),
                    ))
                })?;
                let full_path = parent.join(path_str.to_string());
                let contents = std::fs::read_to_string(&full_path).map_err(|e| {
                    mlua::Error::runtime(format!("failed to read '{}': {}", full_path.display(), e))
                })?;
                let json: serde_json::Value = serde_json::from_str(&contents).map_err(|e| {
                    mlua::Error::runtime(format!(
                        "failed to parse JSON in '{}': {}",
                        full_path.display(),
                        e
                    ))
                })?;
                let value = l.to_value(&json)?;
                Ok(value)
            })
            .map_err(|e| eyre!("failed to create read_json function: {e}"))?;

        let matches = lua
            .create_function(|_, (str, pattern): (mlua::String, mlua::String)| {
                let regexp = regex::Regex::new(&pattern.to_str()?).map_err(|e| {
                    mlua::Error::runtime(format!(
                        "invalid regex pattern '{}': {}",
                        pattern.display(),
                        e
                    ))
                })?;
                Ok(regexp.is_match(&str.to_str()?))
            })
            .map_err(|e| eyre!("failed to create matches function: {e}"))?;

        let module = lua
            .create_table_from([("ReadJSON", read_json), ("Matches", matches)])
            .map_err(|e| eyre!("failed to create table for module: {e}"))?;
        lua.register_module("@checkonaut", module)
            .map_err(|e| eyre!("failed to register checkonaut module: {e}"))?;
        debug!("loaded 'checkonaut' module for '{}'", self.path.display());
        Ok(())
    }
}

fn new_lua_for(path: &Path) -> Result<Lua> {
    let lua = Lua::new();
    update_package_path(&lua, path)?;
    Ok(lua)
}

fn update_package_path(lua: &Lua, for_file: &Path) -> Result<()> {
    let parent_str = for_file
        .parent()
        .and_then(|p| p.to_str())
        .wrap_err_with(|| format!("path is not UTF-8: {}", for_file.display()))?;

    // TODO: Can we set this with a scope so that we don't pollute the global state?
    //  We could use the _ENV variable...
    lua.globals()
        .set("__CHECKONAUT_FILE_PATH", parent_str)
        .map_err(|e| eyre!("failed to set global in Lua: {e}"))?;
    lua.load(r#"
        package.path = package.path .. ";" .. __CHECKONAUT_FILE_PATH .. "/?.lua;" .. __CHECKONAUT_FILE_PATH .. "/?/init.lua"
"#).set_name("=checkonaut_update_package_path").exec().map_err(|e| {
            eyre!( "failed to update package.path in Lua for file '{}': {e}", for_file.display())
        })?;

    Ok(())
}

/// The severity of a check finding, as returned by `Check` functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckSeverity {
    Error,
    Warning,
}

/// Intermediate result type returned by `Check` functions.
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

/// A single check error.
#[derive(Debug, Clone)]
pub struct CheckError {
    pub severity: CheckSeverity,
    pub error: String,
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:?}] {}", self.severity, self.error)
    }
}
