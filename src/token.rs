use std::fmt;

use strum::{EnumString, IntoStaticStr};

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, IntoStaticStr)]
pub enum Keyword {
    #[strum(serialize = "addrspace")]
    Addrspace,
    #[strum(serialize = "align")]
    Align,
    #[strum(serialize = "allowzero")]
    Allowzero,
    #[strum(serialize = "and")]
    And,
    #[strum(serialize = "anyframe")]
    Anyframe,
    #[strum(serialize = "anytype")]
    Anytype,
    #[strum(serialize = "asm")]
    Asm,
    #[strum(serialize = "break")]
    Break,
    #[strum(serialize = "callconv")]
    Callconv,
    #[strum(serialize = "catch")]
    Catch,
    #[strum(serialize = "comptime")]
    Comptime,
    #[strum(serialize = "const")]
    Const,
    #[strum(serialize = "continue")]
    Continue,
    #[strum(serialize = "defer")]
    Defer,
    #[strum(serialize = "else")]
    Else,
    #[strum(serialize = "enum")]
    Enum,
    #[strum(serialize = "errdefer")]
    Errdefer,
    #[strum(serialize = "error")]
    Error,
    #[strum(serialize = "export")]
    Export,
    #[strum(serialize = "extern")]
    Extern,
    #[strum(serialize = "fn")]
    Fn,
    #[strum(serialize = "for")]
    For,
    #[strum(serialize = "if")]
    If,
    #[strum(serialize = "inline")]
    Inline,
    #[strum(serialize = "noalias")]
    Noalias,
    #[strum(serialize = "nosuspend")]
    Nosuspend,
    #[strum(serialize = "noinline")]
    Noinline,
    #[strum(serialize = "opaque")]
    Opaque,
    #[strum(serialize = "or")]
    Or,
    #[strum(serialize = "orelse")]
    Orelse,
    #[strum(serialize = "packed")]
    Packed,
    #[strum(serialize = "pub")]
    Pub,
    #[strum(serialize = "resume")]
    Resume,
    #[strum(serialize = "return")]
    Return,
    #[strum(serialize = "linksection")]
    Linksection,
    #[strum(serialize = "struct")]
    Struct,
    #[strum(serialize = "suspend")]
    Suspend,
    #[strum(serialize = "switch")]
    Switch,
    #[strum(serialize = "test")]
    Test,
    #[strum(serialize = "threadlocal")]
    Threadlocal,
    #[strum(serialize = "try")]
    Try,
    #[strum(serialize = "union")]
    Union,
    #[strum(serialize = "unreachable")]
    Unreachable,
    #[strum(serialize = "var")]
    Var,
    #[strum(serialize = "volatile")]
    Volatile,
    #[strum(serialize = "while")]
    While,
}

impl Keyword {
    pub const ALL: [Self; 46] = [
        Self::Addrspace,
        Self::Align,
        Self::Allowzero,
        Self::And,
        Self::Anyframe,
        Self::Anytype,
        Self::Asm,
        Self::Break,
        Self::Callconv,
        Self::Catch,
        Self::Comptime,
        Self::Const,
        Self::Continue,
        Self::Defer,
        Self::Else,
        Self::Enum,
        Self::Errdefer,
        Self::Error,
        Self::Export,
        Self::Extern,
        Self::Fn,
        Self::For,
        Self::If,
        Self::Inline,
        Self::Noalias,
        Self::Nosuspend,
        Self::Noinline,
        Self::Opaque,
        Self::Or,
        Self::Orelse,
        Self::Packed,
        Self::Pub,
        Self::Resume,
        Self::Return,
        Self::Linksection,
        Self::Struct,
        Self::Suspend,
        Self::Switch,
        Self::Test,
        Self::Threadlocal,
        Self::Try,
        Self::Union,
        Self::Unreachable,
        Self::Var,
        Self::Volatile,
        Self::While,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }

    pub fn precedence(self) -> u8 {
        match self {
            Self::Or => 1,
            Self::And => 2,
            Self::Orelse | Self::Catch => 4,
            _ => 0,
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, IntoStaticStr)]
pub enum Symbol {
    #[strum(serialize = "&")]
    Ampersand,
    #[strum(serialize = "&=")]
    AmpersandEqual,
    #[strum(serialize = "*")]
    Asterisk,
    #[strum(serialize = "**")]
    Asterisk2,
    #[strum(serialize = "*=")]
    AsteriskEqual,
    #[strum(serialize = "*%")]
    AsteriskPercent,
    #[strum(serialize = "*%=")]
    AsteriskPercentEqual,
    #[strum(serialize = "*|")]
    AsteriskPipe,
    #[strum(serialize = "*|=")]
    AsteriskPipeEqual,
    #[strum(serialize = "^")]
    Caret,
    #[strum(serialize = "^=")]
    CaretEqual,
    #[strum(serialize = ":")]
    Colon,
    #[strum(serialize = ",")]
    Comma,
    #[strum(serialize = ".")]
    Dot,
    #[strum(serialize = "..")]
    Dot2,
    #[strum(serialize = "...")]
    Dot3,
    #[strum(serialize = ".*")]
    DotAsterisk,
    #[strum(serialize = ".?")]
    DotQuestionMark,
    #[strum(serialize = "=")]
    Equal,
    #[strum(serialize = "==")]
    EqualEqual,
    #[strum(serialize = "=>")]
    EqualRArrow,
    #[strum(serialize = "!")]
    ExclamationMark,
    #[strum(serialize = "!=")]
    ExclamationMarkEqual,
    #[strum(serialize = "<")]
    LArrow,
    #[strum(serialize = "<<")]
    LArrow2,
    #[strum(serialize = "<<=")]
    LArrow2Equal,
    #[strum(serialize = "<<|")]
    LArrow2Pipe,
    #[strum(serialize = "<<|=")]
    LArrow2PipeEqual,
    #[strum(serialize = "<=")]
    LArrowEqual,
    #[strum(serialize = "{")]
    LBrace,
    #[strum(serialize = "[")]
    LBracket,
    #[strum(serialize = "(")]
    LParen,
    #[strum(serialize = "-")]
    Minus,
    #[strum(serialize = "-=")]
    MinusEqual,
    #[strum(serialize = "-%")]
    MinusPercent,
    #[strum(serialize = "-%=")]
    MinusPercentEqual,
    #[strum(serialize = "-|")]
    MinusPipe,
    #[strum(serialize = "-|=")]
    MinusPipeEqual,
    #[strum(serialize = "->")]
    MinusRArrow,
    #[strum(serialize = "%")]
    Percent,
    #[strum(serialize = "%=")]
    PercentEqual,
    #[strum(serialize = "|")]
    Pipe,
    #[strum(serialize = "||")]
    Pipe2,
    #[strum(serialize = "|=")]
    PipeEqual,
    #[strum(serialize = "+")]
    Plus,
    #[strum(serialize = "++")]
    Plus2,
    #[strum(serialize = "+=")]
    PlusEqual,
    #[strum(serialize = "+%")]
    PlusPercent,
    #[strum(serialize = "+%=")]
    PlusPercentEqual,
    #[strum(serialize = "+|")]
    PlusPipe,
    #[strum(serialize = "+|=")]
    PlusPipeEqual,
    #[strum(serialize = "?")]
    QuestionMark,
    #[strum(serialize = ">")]
    RArrow,
    #[strum(serialize = ">>")]
    RArrow2,
    #[strum(serialize = ">>=")]
    RArrow2Equal,
    #[strum(serialize = ">=")]
    RArrowEqual,
    #[strum(serialize = "}")]
    RBrace,
    #[strum(serialize = "]")]
    RBracket,
    #[strum(serialize = ")")]
    RParen,
    #[strum(serialize = ";")]
    Semicolon,
    #[strum(serialize = "/")]
    Slash,
    #[strum(serialize = "/=")]
    SlashEqual,
    #[strum(serialize = "~")]
    Tilde,
}

impl Symbol {
    pub const ALL: [Self; 63] = [
        Self::Ampersand,
        Self::AmpersandEqual,
        Self::Asterisk,
        Self::Asterisk2,
        Self::AsteriskEqual,
        Self::AsteriskPercent,
        Self::AsteriskPercentEqual,
        Self::AsteriskPipe,
        Self::AsteriskPipeEqual,
        Self::Caret,
        Self::CaretEqual,
        Self::Colon,
        Self::Comma,
        Self::Dot,
        Self::Dot2,
        Self::Dot3,
        Self::DotAsterisk,
        Self::DotQuestionMark,
        Self::Equal,
        Self::EqualEqual,
        Self::EqualRArrow,
        Self::ExclamationMark,
        Self::ExclamationMarkEqual,
        Self::LArrow,
        Self::LArrow2,
        Self::LArrow2Equal,
        Self::LArrow2Pipe,
        Self::LArrow2PipeEqual,
        Self::LArrowEqual,
        Self::LBrace,
        Self::LBracket,
        Self::LParen,
        Self::Minus,
        Self::MinusEqual,
        Self::MinusPercent,
        Self::MinusPercentEqual,
        Self::MinusPipe,
        Self::MinusPipeEqual,
        Self::MinusRArrow,
        Self::Percent,
        Self::PercentEqual,
        Self::Pipe,
        Self::Pipe2,
        Self::PipeEqual,
        Self::Plus,
        Self::Plus2,
        Self::PlusEqual,
        Self::PlusPercent,
        Self::PlusPercentEqual,
        Self::PlusPipe,
        Self::PlusPipeEqual,
        Self::QuestionMark,
        Self::RArrow,
        Self::RArrow2,
        Self::RArrow2Equal,
        Self::RArrowEqual,
        Self::RBrace,
        Self::RBracket,
        Self::RParen,
        Self::Semicolon,
        Self::Slash,
        Self::SlashEqual,
        Self::Tilde,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }

    pub fn from_lexeme(lexeme: &str) -> Option<Self> {
        match lexeme {
            "&" => Some(Self::Ampersand),
            "&=" => Some(Self::AmpersandEqual),
            "*" => Some(Self::Asterisk),
            "**" => Some(Self::Asterisk2),
            "*=" => Some(Self::AsteriskEqual),
            "*%" => Some(Self::AsteriskPercent),
            "*%=" => Some(Self::AsteriskPercentEqual),
            "*|" => Some(Self::AsteriskPipe),
            "*|=" => Some(Self::AsteriskPipeEqual),
            "^" => Some(Self::Caret),
            "^=" => Some(Self::CaretEqual),
            ":" => Some(Self::Colon),
            "," => Some(Self::Comma),
            "." => Some(Self::Dot),
            ".." => Some(Self::Dot2),
            "..." => Some(Self::Dot3),
            ".*" => Some(Self::DotAsterisk),
            ".?" => Some(Self::DotQuestionMark),
            "=" => Some(Self::Equal),
            "==" => Some(Self::EqualEqual),
            "=>" => Some(Self::EqualRArrow),
            "!" => Some(Self::ExclamationMark),
            "!=" => Some(Self::ExclamationMarkEqual),
            "<" => Some(Self::LArrow),
            "<<" => Some(Self::LArrow2),
            "<<=" => Some(Self::LArrow2Equal),
            "<<|" => Some(Self::LArrow2Pipe),
            "<<|=" => Some(Self::LArrow2PipeEqual),
            "<=" => Some(Self::LArrowEqual),
            "{" => Some(Self::LBrace),
            "[" => Some(Self::LBracket),
            "(" => Some(Self::LParen),
            "-" => Some(Self::Minus),
            "-=" => Some(Self::MinusEqual),
            "-%" => Some(Self::MinusPercent),
            "-%=" => Some(Self::MinusPercentEqual),
            "-|" => Some(Self::MinusPipe),
            "-|=" => Some(Self::MinusPipeEqual),
            "->" => Some(Self::MinusRArrow),
            "%" => Some(Self::Percent),
            "%=" => Some(Self::PercentEqual),
            "|" => Some(Self::Pipe),
            "||" => Some(Self::Pipe2),
            "|=" => Some(Self::PipeEqual),
            "+" => Some(Self::Plus),
            "++" => Some(Self::Plus2),
            "+=" => Some(Self::PlusEqual),
            "+%" => Some(Self::PlusPercent),
            "+%=" => Some(Self::PlusPercentEqual),
            "+|" => Some(Self::PlusPipe),
            "+|=" => Some(Self::PlusPipeEqual),
            "?" => Some(Self::QuestionMark),
            ">" => Some(Self::RArrow),
            ">>" => Some(Self::RArrow2),
            ">>=" => Some(Self::RArrow2Equal),
            ">=" => Some(Self::RArrowEqual),
            "}" => Some(Self::RBrace),
            "]" => Some(Self::RBracket),
            ")" => Some(Self::RParen),
            ";" => Some(Self::Semicolon),
            "/" => Some(Self::Slash),
            "/=" => Some(Self::SlashEqual),
            "~" => Some(Self::Tilde),
            _ => None,
        }
    }

    pub fn precedence(self) -> u8 {
        match self {
            Self::EqualEqual
            | Self::ExclamationMarkEqual
            | Self::LArrow
            | Self::RArrow
            | Self::LArrowEqual
            | Self::RArrowEqual => 3,
            Self::Ampersand | Self::Caret | Self::Pipe => 4,
            Self::LArrow2 | Self::RArrow2 | Self::LArrow2Pipe => 5,
            Self::Plus
            | Self::Minus
            | Self::Plus2
            | Self::PlusPercent
            | Self::MinusPercent
            | Self::PlusPipe
            | Self::MinusPipe => 6,
            Self::Pipe2
            | Self::Asterisk
            | Self::Slash
            | Self::Percent
            | Self::Asterisk2
            | Self::AsteriskPercent
            | Self::AsteriskPipe => 7,
            _ => 0,
        }
    }

    pub fn is_assign_op(self) -> bool {
        matches!(
            self,
            Self::Equal
                | Self::AsteriskEqual
                | Self::SlashEqual
                | Self::PercentEqual
                | Self::PlusEqual
                | Self::MinusEqual
                | Self::LArrow2Equal
                | Self::RArrow2Equal
                | Self::AmpersandEqual
                | Self::CaretEqual
                | Self::PipeEqual
                | Self::AsteriskPercentEqual
                | Self::PlusPercentEqual
                | Self::MinusPercentEqual
                | Self::AsteriskPipeEqual
                | Self::PlusPipeEqual
                | Self::MinusPipeEqual
                | Self::LArrow2PipeEqual
        )
    }

    pub fn is_comparison(self) -> bool {
        self.precedence() == 3
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LitKind {
    Ident,
    BuiltinIdent,
    Integer,
    Float,
    Char,
    String,
    MultilineString,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommentKind {
    Line,
    Doc,
    ContainerDoc,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Token {
    Comment(CommentKind, String),
    Keyword(Keyword),
    Symbol(Symbol),
    Literal(LitKind, String),
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Comment,
    Keyword(Keyword),
    Symbol(Symbol),
    Literal(LitKind),
}

impl Token {
    pub fn kind(&self) -> TokenKind {
        match self {
            Self::Comment(_, _) => TokenKind::Comment,
            Self::Keyword(keyword) => TokenKind::Keyword(*keyword),
            Self::Symbol(symbol) => TokenKind::Symbol(*symbol),
            Self::Literal(kind, _) => TokenKind::Literal(*kind),
        }
    }

    pub fn is(&self, kind: &TokenKind) -> bool {
        &self.kind() == kind
    }

    pub fn str_len(&self) -> usize {
        match self {
            Self::Comment(_, text) | Self::Literal(_, text) => text.len(),
            Self::Keyword(keyword) => keyword.as_str().len(),
            Self::Symbol(symbol) => symbol.as_str().len(),
        }
    }

    pub fn precedence(&self) -> u8 {
        match self {
            Self::Keyword(keyword) => keyword.precedence(),
            Self::Symbol(symbol) => symbol.precedence(),
            _ => 0,
        }
    }

    pub fn lexeme(&self) -> &str {
        match self {
            Self::Comment(_, text) | Self::Literal(_, text) => text,
            Self::Keyword(keyword) => keyword.as_str(),
            Self::Symbol(symbol) => symbol.as_str(),
        }
    }
}

impl fmt::Display for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}'", self.as_str())
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}'", self.as_str())
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}'", self.lexeme())
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Comment => write!(f, "comment"),
            Self::Keyword(keyword) => write!(f, "{keyword}"),
            Self::Symbol(symbol) => write!(f, "{symbol}"),
            Self::Literal(kind) => write!(f, "{kind:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn keyword_roundtrip_has_current_set() {
        assert_eq!(Keyword::ALL.len(), 46);
        for keyword in Keyword::ALL {
            assert_eq!(Keyword::from_str(keyword.as_str()).unwrap(), keyword);
        }

        assert!(Keyword::from_str("async").is_err());
        assert!(Keyword::from_str("await").is_err());
        assert!(Keyword::from_str("usingnamespace").is_err());
    }

    #[test]
    fn symbol_roundtrip_and_precedence() {
        for symbol in Symbol::ALL {
            assert_eq!(Symbol::from_lexeme(symbol.as_str()), Some(symbol));
        }

        assert_eq!(Keyword::Or.precedence(), 1);
        assert_eq!(Keyword::And.precedence(), 2);
        assert_eq!(Symbol::EqualEqual.precedence(), 3);
        assert_eq!(Keyword::Catch.precedence(), 4);
        assert_eq!(Symbol::LArrow2Pipe.precedence(), 5);
        assert_eq!(Symbol::PlusPercent.precedence(), 6);
        assert_eq!(Symbol::AsteriskPipe.precedence(), 7);
        assert_eq!(Symbol::Semicolon.precedence(), 0);
    }

    #[test]
    fn token_kind_helpers_match_exactly() {
        let token = Token::Literal(LitKind::Ident, "name".to_owned());
        assert_eq!(token.kind(), TokenKind::Literal(LitKind::Ident));
        assert!(token.is(&TokenKind::Literal(LitKind::Ident)));
        assert!(!token.is(&TokenKind::Literal(LitKind::String)));
        assert_eq!(token.str_len(), 4);

        let token = Token::Symbol(Symbol::Plus);
        assert_eq!(token.precedence(), 6);
    }
}
