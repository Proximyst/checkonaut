use clap::Args;
use eyre::{Context, Result, ensure};
use std::path::{Path, PathBuf};
use tracing::trace;

#[derive(Debug, Args)]
pub struct Check {
    /// The data files or directories to check.
    ///
    /// If given a directory, all recognisable files within the directory will be checked against
    /// all checks. Recognisable files are those with the extensions `.json`, `.yml`, `.yaml`, or
    /// `.toml`.
    #[arg(default_value = ".")]
    input: Vec<PathBuf>,

    /// All check files or directories to run with.
    ///
    /// If given a directory, all Lua check files within the directory will be run.
    /// Note that `_test.lua` files are ignored in directory scans. Likewise, Lua files that
    /// do not expose a `Check` function are also ignored.
    #[arg(short, long, default_value = ".")]
    checks: Vec<PathBuf>,

    /// Act recursively when given directories as input or checks.
    #[arg(short, long)]
    recursive: bool,

    /// Do we want to process .dotfiles and .dotfolders?
    #[arg(long)]
    dotfiles: bool,
}

impl Check {
    pub fn run(self) -> Result<()> {
        // TODO: Support streaming paths instead, to reduce bulk memory usage when given
        // millions/billions of files.
        let mut data_files = Vec::new();
        for file in &self.input {
            data_files.extend(
                find_data_files(file, self.recursive, self.dotfiles).wrap_err_with(|| {
                    format!(
                        "listing data files in {} (recursive={})",
                        file.to_string_lossy(),
                        self.recursive,
                    )
                })?,
            );
        }
        println!("found {} data files to check", data_files.len());

        // Figure out how many bytes total
        use rayon::prelude::*;
        let sz = data_files
            .par_iter()
            .map(|path| {
                let metadata = std::fs::metadata(path).wrap_err_with(|| {
                    format!("could not get metadata for file {}", path.to_string_lossy())
                })?;
                Ok::<u64, eyre::Report>(metadata.len())
            })
            .try_reduce(|| 0u64, |a, b| Ok(a + b))
            .wrap_err("failed to sum data file sizes")?;
        println!("total size of data files: {} bytes", sz);

        Ok(())
    }
}

fn find_data_files(path: &Path, recursive: bool, dotfiles: bool) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.into()]);
    } else {
        ensure!(
            path.is_dir(),
            "path ({}) is neither a file nor a directory",
            path.to_string_lossy(),
        );
    }

    use rayon::prelude::*;

    std::fs::read_dir(path)
        .wrap_err_with(|| format!("could not read directory ({})", path.to_string_lossy()))?
        .par_bridge()
        .try_fold(Vec::new, |mut files, entry| {
            let entry = entry.wrap_err_with(|| {
                format!(
                    "could not read directory entry in {}",
                    path.to_string_lossy()
                )
            })?;
            let entry_path = entry.path();
            if !dotfiles
                && entry_path
                    .file_name()
                    .map_or(false, |n| n.to_string_lossy().starts_with('.'))
            {
                trace!(
                    ?path,
                    ?entry_path,
                    "ignoring dotfile or dotfolder as per configuration",
                );
                return Ok(files);
            }

            if entry_path.is_file() {
                if let Some(ext) = entry_path.extension() {
                    if ext == "json" || ext == "yml" || ext == "yaml" || ext == "toml" {
                        files.push(entry_path);
                    }
                }
                Ok::<Vec<PathBuf>, eyre::Report>(files)
            } else if recursive && entry_path.is_dir() {
                let mut new_files = files;
                new_files.extend(find_data_files(&entry_path, recursive, dotfiles)?);
                Ok(new_files)
            } else {
                trace!(
                    ?path,
                    ?entry_path,
                    "ignoring data directory because we are not recursively iterating"
                );
                Ok(files)
            }
        })
        .try_reduce(Vec::new, |mut a, mut b| {
            a.append(&mut b);
            Ok(a)
        })
}
