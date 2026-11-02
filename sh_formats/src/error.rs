// --------------------------------------------------------------------------------------------------------------------
// The one error type every reader in this crate returns.

use std::path::PathBuf;

/// Why a read, a lookup or a decode did not happen.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("reading {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    /// The archive's entry table does not add up; the message says which check failed.
    #[error("{path}: {reason}")]
    Archive { path: PathBuf, reason: String },
    #[error("entry {index} is past the {count} in the archive")]
    NoSuchEntry { index: usize, count: usize },
    /// An entry that does not decode as the thing asked for, and why.
    #[error("entry {index} is not {wanted}: {reason}")]
    WrongType { index: usize, wanted: &'static str, reason: String },
    /// Bytes that are not the format a reader was asked for.
    #[error("{0}")]
    Malformed(String),
    #[error("no entry is named {0}")]
    UnknownName(String),
    #[error("{name} is a run of {len}, there is no entry {nth} in it")]
    NoSuchFrame { name: String, len: usize, nth: usize },
    #[error("names file line {line}: {reason}")]
    Names { line: usize, reason: String },
    #[error("palette rule: {0}")]
    Palette(String),
    #[error("export: {0}")]
    Export(String),
    /// The ungrab refused: an archive it does not recognise, or a source it cannot rebuild exactly.
    #[error("ungrab: {0}")]
    Ungrab(String),
}

pub type Result<T> = std::result::Result<T, FormatError>;

impl FormatError {
    pub fn malformed(reason: impl Into<String>) -> FormatError {
        FormatError::Malformed(reason.into())
    }
}
