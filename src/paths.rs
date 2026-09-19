//! Read-only root validation. This is a snapshot, not a guarantee against later
//! filesystem changes; any future writer must revalidate and defend against races.
use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct ValidatedPaths {
    pub input: PathBuf,
    pub output: PathBuf,
}

#[derive(Debug)]
pub enum PathError {
    Io { path: PathBuf, source: io::Error },
    NotDirectory(PathBuf),
    AmbiguousOutput(PathBuf),
    Overlap,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot resolve '{}': {source}", path.display()),
            Self::NotDirectory(path) => write!(f, "'{}' is not a directory", path.display()),
            Self::AmbiguousOutput(path) => write!(
                f,
                "cannot safely resolve output '{}': parent traversal after a missing directory is unsupported",
                path.display()
            ),
            Self::Overlap => write!(
                f,
                "unsafe paths: input and output must be separate directories; equal or nested roots are forbidden"
            ),
        }
    }
}

impl std::error::Error for PathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn io_error(path: &Path, source: io::Error) -> PathError {
    PathError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn require_directory(path: &Path) -> Result<(), PathError> {
    let metadata = fs::metadata(path).map_err(|error| io_error(path, error))?;
    if !metadata.is_dir() {
        return Err(PathError::NotDirectory(path.to_path_buf()));
    }
    Ok(())
}

pub fn validate_input(input: &Path) -> Result<PathBuf, PathError> {
    let input = fs::canonicalize(input).map_err(|error| io_error(input, error))?;
    require_directory(&input)?;
    Ok(input)
}

pub fn validate(input: &Path, output: &Path) -> Result<ValidatedPaths, PathError> {
    let input = validate_input(input)?;
    let output = resolve_output(output)?;
    if input.starts_with(&output) || output.starts_with(&input) {
        return Err(PathError::Overlap);
    }
    Ok(ValidatedPaths { input, output })
}

fn resolve_output(output: &Path) -> Result<PathBuf, PathError> {
    let mut ancestor = if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| io_error(output, error))?
            .join(output)
    };
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(&ancestor) {
            Ok(mut resolved) => {
                require_directory(&resolved)?;
                for name in missing.iter().rev() {
                    resolved.push(name);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // A dangling symlink exists but cannot be canonicalized. Never
                // mistake it for a directory that a future writer can create.
                match fs::symlink_metadata(&ancestor) {
                    Ok(_) => return Err(io_error(&ancestor, error)),
                    Err(probe) if probe.kind() == io::ErrorKind::NotFound => {}
                    Err(probe) => return Err(io_error(&ancestor, probe)),
                }
                let name = ancestor
                    .file_name()
                    .ok_or_else(|| PathError::AmbiguousOutput(output.to_path_buf()))?;
                missing.push(name.to_os_string());
                if !ancestor.pop() {
                    return Err(PathError::AmbiguousOutput(output.to_path_buf()));
                }
            }
            Err(error) => return Err(io_error(&ancestor, error)),
        }
    }
}
