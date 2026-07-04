use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::token::{CommentKind, Keyword, LitKind, Symbol, Token};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanOptions {
    pub strict_whitespace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpannedToken {
    pub pos: usize,
    pub token: Token,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScannerMark {
    pos: usize,
}

#[derive(Debug, Clone)]
pub struct Scanner {
    source: String,
    chars: Vec<char>,
    byte_offsets: Vec<usize>,
    pos: usize,
    path: PathBuf,
    line_info: Vec<usize>,
    options: ScanOptions,
}

impl Scanner {
    pub fn from<S: AsRef<str>>(source: S) -> Self {
        Self::with_options(source, ScanOptions::default())
    }

    pub fn with_options<S: AsRef<str>>(source: S, options: ScanOptions) -> Self {
        let source = source.as_ref().to_owned();
        Self::new(source, PathBuf::from("<source>"), options)
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let path = path.as_ref();
        let source = fs::read_to_string(path)?;
        Ok(Self::new(
            source,
            path.to_path_buf(),
            ScanOptions::default(),
        ))
    }

    fn new(source: String, path: PathBuf, options: ScanOptions) -> Self {
        let chars: Vec<char> = source.chars().collect();
        let mut byte_offsets: Vec<usize> = source.char_indices().map(|(idx, _)| idx).collect();
        byte_offsets.push(source.len());

        let mut line_info = vec![0];
        for (idx, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_info.push(idx + 1);
            }
        }

        Self {
            source,
            chars,
            byte_offsets,
            pos: 0,
            path,
            line_info,
            options,
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn lines(&self) -> &[usize] {
        &self.line_info
    }

    pub fn position(&self) -> usize {
        self.byte_pos()
    }

    pub fn preback(&self) -> ScannerMark {
        ScannerMark { pos: self.pos }
    }

    pub fn goback(&mut self, mark: ScannerMark) {
        self.pos = mark.pos;
    }

    pub fn line_info(&self, pos: usize) -> (usize, usize) {
        let line_idx = self
            .line_info
            .partition_point(|line_start| *line_start <= pos)
            .saturating_sub(1);
        let line_start = self.line_info.get(line_idx).copied().unwrap_or(0);
        (line_idx + 1, pos.saturating_sub(line_start) + 1)
    }

    pub fn next_token(&mut self) -> Result<Option<SpannedToken>, Error> {
        self.skip_whitespace()?;
        if self.is_eof() {
            return Ok(None);
        }

        let pos = self.byte_pos();
        let token = self.scan_token()?;
        Ok(Some(SpannedToken { pos, token }))
    }

    pub fn scan_all(mut self) -> Result<Vec<SpannedToken>, Error> {
        let mut tokens = Vec::new();
        while let Some(token) = self.next_token()? {
            tokens.push(token);
        }
        Ok(tokens)
    }

    fn scan_token(&mut self) -> Result<Token, Error> {
        if self.starts_with("//") {
            return Ok(self.scan_comment());
        }
        if self.starts_with("\\\\") {
            return Ok(self.scan_multiline_string());
        }

        match self.peek() {
            Some('@') => self.scan_at_prefixed(),
            Some('"') => self.scan_string_like(LitKind::String, false),
            Some('\'') => self.scan_char_literal(),
            Some(ch) if ch.is_ascii_digit() => self.scan_number(),
            Some(ch) if is_ident_start(ch) => Ok(self.scan_identifier_or_keyword()),
            Some(_) => self.scan_symbol(),
            None => unreachable!("scan_token is only called when not EOF"),
        }
    }

    fn skip_whitespace(&mut self) -> Result<(), Error> {
        loop {
            match self.peek() {
                Some(' ' | '\n') => {
                    self.advance();
                }
                Some('\t' | '\r') if !self.options.strict_whitespace => {
                    self.advance();
                }
                Some('\t' | '\r') => {
                    return Err(self.error_here("invalid whitespace outside PEG skip rule"));
                }
                _ => return Ok(()),
            }
        }
    }

    fn scan_comment(&mut self) -> Token {
        let start = self.pos;
        let kind = if self.starts_with("////") {
            CommentKind::Line
        } else if self.starts_with("//!") {
            CommentKind::ContainerDoc
        } else if self.starts_with("///") {
            CommentKind::Doc
        } else {
            CommentKind::Line
        };

        while !self.is_eof() && self.peek() != Some('\n') {
            self.advance();
        }

        Token::Comment(kind, self.slice_chars(start, self.pos).to_owned())
    }

    fn scan_identifier_or_keyword(&mut self) -> Token {
        let start = self.pos;
        self.advance();
        while self.peek().is_some_and(is_ident_continue) {
            self.advance();
        }

        let text = self.slice_chars(start, self.pos);
        match text.parse::<Keyword>() {
            Ok(keyword) => Token::Keyword(keyword),
            Err(_) => Token::Literal(LitKind::Ident, text.to_owned()),
        }
    }

    fn scan_at_prefixed(&mut self) -> Result<Token, Error> {
        if self.starts_with("@\"") {
            return self.scan_string_like(LitKind::Ident, true);
        }

        let start = self.pos;
        self.advance();
        if !self.peek().is_some_and(is_ident_start) {
            return Err(self.error_at(
                start,
                "expected builtin identifier or quoted identifier after '@'",
            ));
        }
        self.advance();
        while self.peek().is_some_and(is_ident_continue) {
            self.advance();
        }

        Ok(Token::Literal(
            LitKind::BuiltinIdent,
            self.slice_chars(start, self.pos).to_owned(),
        ))
    }

    fn scan_number(&mut self) -> Result<Token, Error> {
        let start = self.pos;
        let kind = if self.starts_with("0b") {
            self.advance_n(2);
            self.scan_digit_run(2, "binary integer")?;
            self.ensure_number_boundary()?;
            LitKind::Integer
        } else if self.starts_with("0o") {
            self.advance_n(2);
            self.scan_digit_run(8, "octal integer")?;
            self.ensure_number_boundary()?;
            LitKind::Integer
        } else if self.starts_with("0x") {
            self.advance_n(2);
            self.scan_digit_run(16, "hex integer")?;
            if self.peek() == Some('.') && self.peek_n(1) != Some('.') {
                self.advance();
                self.scan_digit_run(16, "hex float fraction")?;
                if matches!(self.peek(), Some('p' | 'P')) {
                    self.advance();
                    self.scan_optional_sign();
                    self.scan_digit_run(10, "hex float exponent")?;
                }
                self.ensure_number_boundary()?;
                LitKind::Float
            } else if matches!(self.peek(), Some('p' | 'P')) {
                self.advance();
                self.scan_optional_sign();
                self.scan_digit_run(10, "hex float exponent")?;
                self.ensure_number_boundary()?;
                LitKind::Float
            } else {
                self.ensure_number_boundary()?;
                LitKind::Integer
            }
        } else {
            self.scan_digit_run(10, "decimal integer")?;
            if self.peek() == Some('.') && self.peek_n(1) != Some('.') {
                self.advance();
                self.scan_digit_run(10, "decimal float fraction")?;
                if matches!(self.peek(), Some('e' | 'E')) {
                    self.advance();
                    self.scan_optional_sign();
                    self.scan_digit_run(10, "decimal float exponent")?;
                }
                self.ensure_number_boundary()?;
                LitKind::Float
            } else if matches!(self.peek(), Some('e' | 'E')) {
                self.advance();
                self.scan_optional_sign();
                self.scan_digit_run(10, "decimal float exponent")?;
                self.ensure_number_boundary()?;
                LitKind::Float
            } else {
                self.ensure_number_boundary()?;
                LitKind::Integer
            }
        };

        Ok(Token::Literal(
            kind,
            self.slice_chars(start, self.pos).to_owned(),
        ))
    }

    fn scan_digit_run(&mut self, radix: u32, label: &str) -> Result<(), Error> {
        if !self.peek().is_some_and(|ch| is_digit_for_radix(ch, radix)) {
            return Err(self.error_here(&format!("expected digit in {label}")));
        }

        self.advance();
        loop {
            match (self.peek(), self.peek_n(1)) {
                (Some('_'), Some(next)) if is_digit_for_radix(next, radix) => {
                    self.advance_n(2);
                }
                (Some('_'), _) => {
                    return Err(self.error_here("underscore must be followed by a digit"));
                }
                (Some(ch), _) if is_digit_for_radix(ch, radix) => {
                    self.advance();
                }
                _ => return Ok(()),
            }
        }
    }

    fn scan_optional_sign(&mut self) {
        if matches!(self.peek(), Some('+' | '-')) {
            self.advance();
        }
    }

    fn ensure_number_boundary(&self) -> Result<(), Error> {
        if self
            .peek()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err(self.error_here("invalid character after numeric literal"));
        }
        Ok(())
    }

    fn scan_char_literal(&mut self) -> Result<Token, Error> {
        let start = self.pos;
        self.advance();
        self.scan_char_char("character literal")?;
        if self.peek() != Some('\'') {
            return Err(self.error_here("expected closing quote for character literal"));
        }
        self.advance();

        Ok(Token::Literal(
            LitKind::Char,
            self.slice_chars(start, self.pos).to_owned(),
        ))
    }

    fn scan_string_like(&mut self, kind: LitKind, quoted_identifier: bool) -> Result<Token, Error> {
        let start = self.pos;
        if quoted_identifier {
            self.advance();
        }
        if self.peek() != Some('"') {
            return Err(self.error_here("expected string opening quote"));
        }
        self.advance();

        loop {
            match self.peek() {
                Some('"') => {
                    self.advance();
                    break;
                }
                Some('\n') | None => return Err(self.error_here("unterminated string literal")),
                Some('\\') => self.scan_escape()?,
                Some(ch) if ch.is_control() => {
                    return Err(self.error_here("control character in string literal"));
                }
                Some(_) => {
                    self.advance();
                }
            }
        }

        Ok(Token::Literal(
            kind,
            self.slice_chars(start, self.pos).to_owned(),
        ))
    }

    fn scan_char_char(&mut self, label: &str) -> Result<(), Error> {
        match self.peek() {
            Some('\\') => self.scan_escape(),
            Some('\n' | '\'') | None => Err(self.error_here(&format!("invalid {label}"))),
            Some(ch) if ch.is_control() => {
                Err(self.error_here(&format!("control character in {label}")))
            }
            Some(_) => {
                self.advance();
                Ok(())
            }
        }
    }

    fn scan_escape(&mut self) -> Result<(), Error> {
        self.advance();
        match self.peek() {
            Some('x') => {
                self.advance();
                self.scan_exact_hex_digits(2)
            }
            Some('u') => {
                self.advance();
                if self.peek() != Some('{') {
                    return Err(self.error_here("expected '{' after unicode escape"));
                }
                self.advance();
                let mut digits = 0usize;
                while self.peek().is_some_and(|ch| ch.is_ascii_hexdigit()) {
                    digits += 1;
                    self.advance();
                }
                if digits == 0 {
                    return Err(self.error_here("unicode escape requires hex digits"));
                }
                if self.peek() != Some('}') {
                    return Err(self.error_here("expected '}' after unicode escape"));
                }
                self.advance();
                Ok(())
            }
            Some('n' | 'r' | '\\' | 't' | '\'' | '"') => {
                self.advance();
                Ok(())
            }
            _ => Err(self.error_here("invalid escape sequence")),
        }
    }

    fn scan_exact_hex_digits(&mut self, count: usize) -> Result<(), Error> {
        for _ in 0..count {
            if !self.peek().is_some_and(|ch| ch.is_ascii_hexdigit()) {
                return Err(self.error_here("expected hex digit"));
            }
            self.advance();
        }
        Ok(())
    }

    fn scan_multiline_string(&mut self) -> Token {
        let start = self.pos;
        loop {
            while !self.is_eof() && self.peek() != Some('\n') {
                self.advance();
            }

            let skip_start = self.pos;
            loop {
                while matches!(self.peek(), Some(' ' | '\n' | '\t' | '\r')) {
                    self.advance();
                }
                if self.starts_with("////")
                    || (self.starts_with("//")
                        && !self.starts_with("//!")
                        && !self.starts_with("///"))
                {
                    while !self.is_eof() && self.peek() != Some('\n') {
                        self.advance();
                    }
                    continue;
                }
                break;
            }

            if self.starts_with("\\\\") {
                continue;
            }

            self.pos = skip_start;
            break;
        }

        Token::Literal(
            LitKind::MultilineString,
            self.slice_chars(start, self.pos).to_owned(),
        )
    }

    fn scan_symbol(&mut self) -> Result<Token, Error> {
        const SYMBOLS: &[&str] = &[
            "<<|=", "*%=", "*|=", "-%=", "-|=", "+%=", "+|=", "...", "<<=", "<<|", ">>=", "&=",
            "**", "*=", "*%", "*|", "^=", "..", ".*", ".?", "==", "=>", "!=", "<<", "<=", "-=",
            "-%", "-|", "->", "%=", "||", "|=", "++", "+=", "+%", "+|", ">>", ">=", "&", "*", "^",
            ":", ",", ".", "=", "!", "<", "{", "[", "(", "-", "%", "|", "+", "?", ">", "}", "]",
            ")", ";", "/=", "/", "~",
        ];

        for lexeme in SYMBOLS {
            if self.starts_with(lexeme) {
                self.advance_n(lexeme.chars().count());
                if let Some(symbol) = Symbol::from_lexeme(lexeme) {
                    return Ok(Token::Symbol(symbol));
                }
            }
        }

        Err(self.error_here("invalid character"))
    }

    fn starts_with(&self, needle: &str) -> bool {
        self.source[self.byte_pos()..].starts_with(needle)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_n(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn advance_n(&mut self, count: usize) {
        self.pos += count;
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn byte_pos(&self) -> usize {
        self.byte_offsets[self.pos]
    }

    fn slice_chars(&self, start: usize, end: usize) -> &str {
        &self.source[self.byte_offsets[start]..self.byte_offsets[end]]
    }

    fn error_here(&self, reason: &str) -> Error {
        self.error_at(self.byte_pos(), reason)
    }

    fn error_at(&self, pos: usize, reason: &str) -> Error {
        Error::Else {
            path: self.path.clone(),
            location: self.line_info(pos),
            reason: reason.to_owned(),
        }
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_digit_for_radix(ch: char, radix: u32) -> bool {
    ch.is_digit(radix)
}

#[cfg(test)]
mod tests {
    use crate::token::TokenKind;

    use super::*;

    fn scan_tokens(source: &str) -> Vec<Token> {
        Scanner::from(source)
            .scan_all()
            .unwrap()
            .into_iter()
            .map(|spanned| spanned.token)
            .collect()
    }

    #[test]
    fn scan_whitespace_positions_and_invalid_characters() {
        let mut scanner = Scanner::from(" \n  const");
        let token = scanner.next_token().unwrap().unwrap();
        assert_eq!(token.pos, 4);
        assert_eq!(scanner.line_info(token.pos), (2, 3));

        let mut strict = Scanner::with_options(
            "\tconst",
            ScanOptions {
                strict_whitespace: true,
            },
        );
        assert!(strict.next_token().is_err());
    }

    #[test]
    fn scan_comment_kinds() {
        let tokens = scan_tokens("// x\n/// doc\n//! module\n//// not-doc\n");
        assert_eq!(
            tokens,
            vec![
                Token::Comment(CommentKind::Line, "// x".to_owned()),
                Token::Comment(CommentKind::Doc, "/// doc".to_owned()),
                Token::Comment(CommentKind::ContainerDoc, "//! module".to_owned()),
                Token::Comment(CommentKind::Line, "//// not-doc".to_owned()),
            ]
        );
    }

    #[test]
    fn scan_identifiers_keywords_builtins_and_quoted_identifiers() {
        let tokens = scan_tokens("const async @import @\"pub\"");
        assert_eq!(
            tokens,
            vec![
                Token::Keyword(Keyword::Const),
                Token::Literal(LitKind::Ident, "async".to_owned()),
                Token::Literal(LitKind::BuiltinIdent, "@import".to_owned()),
                Token::Literal(LitKind::Ident, "@\"pub\"".to_owned()),
            ]
        );

        assert!(Scanner::from("@").next_token().is_err());
    }

    #[test]
    fn scan_numbers_and_reject_invalid_forms() {
        let tokens = scan_tokens("0 123 1_000 0b1010 0o755 0xCAFE 1.5 1.5e-3 0x1.8p+4 0x1p4 1..2");
        let kinds: Vec<TokenKind> = tokens.iter().map(Token::kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Literal(LitKind::Integer),
                TokenKind::Literal(LitKind::Integer),
                TokenKind::Literal(LitKind::Integer),
                TokenKind::Literal(LitKind::Integer),
                TokenKind::Literal(LitKind::Integer),
                TokenKind::Literal(LitKind::Integer),
                TokenKind::Literal(LitKind::Float),
                TokenKind::Literal(LitKind::Float),
                TokenKind::Literal(LitKind::Float),
                TokenKind::Literal(LitKind::Float),
                TokenKind::Literal(LitKind::Integer),
                TokenKind::Symbol(Symbol::Dot2),
                TokenKind::Literal(LitKind::Integer),
            ]
        );

        for invalid in ["0x", "1_", "0b2", "1.e3"] {
            assert!(Scanner::from(invalid).next_token().is_err(), "{invalid}");
        }
    }

    #[test]
    fn scan_char_string_and_multiline_string() {
        let tokens = scan_tokens("'a' '\\n' \"hello\\n\" \\\\first\n\n  \\\\second\nnext");
        assert_eq!(tokens[0].kind(), TokenKind::Literal(LitKind::Char));
        assert_eq!(tokens[1].kind(), TokenKind::Literal(LitKind::Char));
        assert_eq!(tokens[2].kind(), TokenKind::Literal(LitKind::String));
        assert_eq!(
            tokens[3].kind(),
            TokenKind::Literal(LitKind::MultilineString)
        );
        assert_eq!(tokens[4], Token::Literal(LitKind::Ident, "next".to_owned()));

        let tokens = scan_tokens("\\\\first\n// comment between lines\n\\\\second\nnext");
        assert_eq!(
            tokens[0].kind(),
            TokenKind::Literal(LitKind::MultilineString)
        );
        assert_eq!(tokens[1], Token::Literal(LitKind::Ident, "next".to_owned()));

        assert!(Scanner::from("\"a\n b\"").next_token().is_err());
        assert!(Scanner::from("\"\\q\"").next_token().is_err());
        assert!(Scanner::from("'ab'").next_token().is_err());
    }

    #[test]
    fn scan_all_symbols_with_longest_match() {
        for symbol in Symbol::ALL {
            let token = Scanner::from(symbol.as_str())
                .next_token()
                .unwrap()
                .unwrap()
                .token;
            assert_eq!(token, Token::Symbol(symbol), "{}", symbol.as_str());
        }

        assert_eq!(
            scan_tokens("a.*.? x<<|=y"),
            vec![
                Token::Literal(LitKind::Ident, "a".to_owned()),
                Token::Symbol(Symbol::DotAsterisk),
                Token::Symbol(Symbol::DotQuestionMark),
                Token::Literal(LitKind::Ident, "x".to_owned()),
                Token::Symbol(Symbol::LArrow2PipeEqual),
                Token::Literal(LitKind::Ident, "y".to_owned()),
            ]
        );
    }
}
