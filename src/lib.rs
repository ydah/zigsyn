//! Pure Rust scanner and parser for Zig source code.
//!
//! `zigsyn` parses Zig syntax into a structured AST while preserving token-level
//! information for lossless inspection.
//!
//! # Example
//!
//! ```
//! let file = zigsyn::parse_source("pub fn main() void {}")?;
//! assert_eq!(file.members.len(), 1);
//! # Ok::<(), zigsyn::Error>(())
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub mod ast;
pub mod error;
pub mod parser;
pub mod scanner;
pub mod token;

pub use error::Error;
pub use parser::Parser;

/// Parses a Zig source string into an AST file.
pub fn parse_source<S: AsRef<str>>(source: S) -> Result<ast::File, Error> {
    Parser::from(source.as_ref()).parse_file()
}

/// Reads and parses a Zig source file.
pub fn parse_file<P: AsRef<Path>>(path: P) -> Result<ast::File, Error> {
    Parser::from_file(path)?.parse_file()
}

/// Parses every `.zig` file directly under `dir`.
pub fn parse_dir<P: AsRef<Path>>(dir: P) -> Result<HashMap<String, ast::File>, Error> {
    let dir = dir.as_ref();
    let mut files = HashMap::new();

    for entry in fs::read_dir(dir).map_err(|source| Error::io_at(dir, source))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("zig") {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                Error::io_at(
                    &path,
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "non UTF-8 file name"),
                )
            })?;
        files.insert(name, parse_file(path)?);
    }

    Ok(files)
}
