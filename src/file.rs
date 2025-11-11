use rayon::prelude::*;
use snafu::{ResultExt, Snafu};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileSearcher {
    include_dotfiles: bool,
    include_dotdirs: bool,
    follow_links: bool,

    include_check_files: bool,
    include_test_files: bool,
    include_data_files: bool,
}

impl FileSearcher {
    pub fn include_dotfiles(mut self, include: bool) -> Self {
        self.include_dotfiles = include;
        self
    }

    pub fn include_dotdirs(mut self, include: bool) -> Self {
        self.include_dotdirs = include;
        self
    }

    pub fn include_check_files(mut self, include: bool) -> Self {
        self.include_check_files = include;
        self
    }

    pub fn include_test_files(mut self, include: bool) -> Self {
        self.include_test_files = include;
        self
    }

    pub fn include_data_files(mut self, include: bool) -> Self {
        self.include_data_files = include;
        self
    }

    pub fn search<I, J>(self, from_paths: I) -> Result<FileSearchResult, FileSearchError>
    where
        I: IntoParallelIterator<Iter = J, Item = PathBuf>,
        J: ParallelIterator<Item = PathBuf>,
    {
        from_paths
            .into_par_iter()
            .flat_map(|p| self.find_files(p))
            .try_fold(FileSearchResult::default, |mut acc, result| match result {
                Ok(path) => {
                    let ty = FileTy::derive_from_path(&path);
                    match ty {
                        Some(FileTy::Test) => acc.test_files.push(path),
                        Some(FileTy::Check) => acc.check_files.push(path),
                        Some(FileTy::Data) => acc.data_files.push(path),
                        None => unreachable!("we should never get a type we don't know about?"),
                    }
                    Ok(acc)
                }
                Err(e) => Err(e),
            })
            .try_reduce_with(|mut a, mut b| {
                a.check_files.append(&mut b.check_files);
                a.test_files.append(&mut b.test_files);
                a.data_files.append(&mut b.data_files);
                Ok(a)
            })
            .unwrap_or_else(|| Ok(FileSearchResult::default()))
    }

    fn find_files(
        &self,
        path: PathBuf,
    ) -> impl ParallelIterator<Item = Result<PathBuf, FileSearchError>> {
        walkdir::WalkDir::new(&path)
            .follow_links(self.follow_links)
            .into_iter()
            .par_bridge()
            .filter_map(move |entry| match entry {
                // We don't care about the directories themselves; walkdir will enter them for us.
                Ok(entry) if entry.file_type().is_file() => {
                    // Period is an ASCII character, so we don't need to care about whether we follow
                    // UTF-8 in the path :)
                    let name_bytes = entry.file_name().as_encoded_bytes();
                    let ty = FileTy::derive_from_byte_name(name_bytes);
                    let included = match ty {
                        Some(FileTy::Test) => self.include_test_files,
                        Some(FileTy::Check) => self.include_check_files,
                        Some(FileTy::Data) => self.include_data_files,
                        None => false,
                    };
                    let include_dot = if entry.file_type().is_dir() {
                        self.include_dotdirs
                    } else {
                        self.include_dotfiles
                    };
                    let included = included && (include_dot || !name_bytes.starts_with(b"."));

                    if included {
                        Some(Ok(entry.into_path()))
                    } else {
                        None
                    }
                }

                Ok(_) => {
                    // Ignore this item; we'll either visit the values inside that we care about,
                    // or it isn't something that we've configured ourselves to care about.
                    None
                }

                Err(err) => Some(Err(err).context(FailedDirectoryWalkSnafu {
                    path: path.to_path_buf(),
                })),
            })
    }
}

#[derive(Debug, Clone, Default)]
pub struct FileSearchResult {
    pub check_files: Vec<PathBuf>,
    pub test_files: Vec<PathBuf>,
    pub data_files: Vec<PathBuf>,
}

#[derive(Debug, Snafu)]
pub enum FileSearchError {
    #[snafu(display("Failed to walk directory '{}'", path.display()))]
    FailedDirectoryWalk {
        path: PathBuf,
        source: walkdir::Error,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileTy {
    Test,
    Check,
    Data,
}

impl FileTy {
    fn derive_from_path(path: &Path) -> Option<Self> {
        let name_bytes = path.file_name()?.as_encoded_bytes();
        if name_bytes.ends_with(b"_test.lua") {
            Some(FileTy::Test)
        } else if name_bytes.ends_with(b".lua") {
            Some(FileTy::Check)
        } else if name_bytes.ends_with(b".json")
            || name_bytes.ends_with(b".yaml")
            || name_bytes.ends_with(b".yml")
            || name_bytes.ends_with(b".toml")
        {
            Some(FileTy::Data)
        } else {
            None
        }
    }

    fn derive_from_byte_name(name_bytes: &[u8]) -> Option<Self> {
        if name_bytes.ends_with(b"_test.lua") {
            Some(FileTy::Test)
        } else if name_bytes.ends_with(b".lua") {
            Some(FileTy::Check)
        } else if name_bytes.ends_with(b".json")
            || name_bytes.ends_with(b".yaml")
            || name_bytes.ends_with(b".yml")
            || name_bytes.ends_with(b".toml")
        {
            Some(FileTy::Data)
        } else {
            None
        }
    }
}
