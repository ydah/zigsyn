use std::fmt;
use std::path::{Path, PathBuf};

use crate::token::{Token, TokenKind};

#[derive(Debug)]
pub enum Error {
    IO {
        path: Option<PathBuf>,
        source: std::io::Error,
    },
    UnexpectedToken {
        path: PathBuf,
        offset: usize,
        location: (usize, usize),
        expect: Vec<TokenKind>,
        actual: Option<Token>,
    },
    Syntax {
        path: PathBuf,
        offset: usize,
        location: (usize, usize),
        reason: String,
    },
}

impl Error {
    pub(crate) fn io_at(path: &Path, source: std::io::Error) -> Self {
        Self::IO {
            path: Some(path.to_owned()),
            source,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Self::IO { path: None, source }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IO { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IO {
                path: Some(path),
                source,
            } => write!(f, "failed to read {}: {source}", path.display()),
            Self::IO { path: None, source } => write!(f, "{source}"),
            Self::UnexpectedToken {
                path,
                location,
                expect,
                actual,
                ..
            } => {
                write!(
                    f,
                    "{}:{}:{} expected ",
                    path.display(),
                    location.0,
                    location.1
                )?;
                write_expectations(f, expect)?;
                match actual {
                    Some(token) => write!(f, ", found {token}"),
                    None => write!(f, ", found EOF"),
                }
            }
            Self::Syntax {
                path,
                location,
                reason,
                ..
            } => write!(
                f,
                "{}:{}:{} {}",
                path.display(),
                location.0,
                location.1,
                reason
            ),
        }
    }
}

fn write_expectations(f: &mut fmt::Formatter<'_>, expect: &[TokenKind]) -> fmt::Result {
    match expect {
        [] => write!(f, "valid token"),
        [single] => write!(f, "{single}"),
        many => {
            for (idx, token) in many.iter().enumerate() {
                if idx > 0 {
                    write!(f, " or ")?;
                }
                write!(f, "{token}")?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::token::{Keyword, Symbol, Token};

    use super::*;

    #[test]
    fn formats_unexpected_single_token() {
        let err = Error::UnexpectedToken {
            path: PathBuf::from("x.zig"),
            offset: 3,
            location: (2, 3),
            expect: vec![TokenKind::Symbol(Symbol::Semicolon)],
            actual: Some(Token::Keyword(Keyword::Fn)),
        };

        assert_eq!(err.to_string(), "x.zig:2:3 expected ';', found 'fn'");
    }

    #[test]
    fn formats_unexpected_multiple_tokens() {
        let err = Error::UnexpectedToken {
            path: PathBuf::from("x.zig"),
            offset: 10,
            location: (4, 5),
            expect: vec![
                TokenKind::Symbol(Symbol::Comma),
                TokenKind::Symbol(Symbol::RBrace),
            ],
            actual: Some(Token::Symbol(Symbol::Semicolon)),
        };

        assert_eq!(err.to_string(), "x.zig:4:5 expected ',' or '}', found ';'");
    }

    #[test]
    fn formats_unexpected_eof_and_syntax_error() {
        let err = Error::UnexpectedToken {
            path: PathBuf::from("x.zig"),
            offset: 25,
            location: (10, 1),
            expect: vec![TokenKind::Symbol(Symbol::RParen)],
            actual: None,
        };
        assert_eq!(err.to_string(), "x.zig:10:1 expected ')', found EOF");

        let err = Error::Syntax {
            path: PathBuf::from("x.zig"),
            offset: 0,
            location: (1, 1),
            reason: "invalid input".to_owned(),
        };
        assert_eq!(err.to_string(), "x.zig:1:1 invalid input");
    }
}
