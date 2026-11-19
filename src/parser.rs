use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ast;
use crate::error::Error;
use crate::scanner::{Scanner, SpannedToken};
use crate::token::{CommentKind, Keyword, LitKind, Symbol, Token, TokenKind};

const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParserMark {
    cursor: usize,
    depth: usize,
    suppress_curly_suffix: usize,
    comment_count: usize,
}

#[derive(Debug)]
pub struct Parser {
    tokens: Vec<SpannedToken>,
    cursor: usize,
    path: PathBuf,
    line_info: Vec<usize>,
    source_len: usize,
    comments: Vec<Arc<ast::Comment>>,
    depth: usize,
    suppress_curly_suffix: usize,
    scan_error: Option<Error>,
}

#[derive(Debug)]
struct LocalVarProto {
    keyword: Keyword,
    name: ast::Ident,
    ty: Option<ast::Expression>,
    align: Option<ast::Expression>,
    addrspace: Option<ast::Expression>,
    linksection: Option<ast::Expression>,
}

impl LocalVarProto {
    fn into_later_binding(self) -> ast::VarProtoOrExpr {
        ast::VarProtoOrExpr::VarProto {
            pos: self.name.pos,
            keyword: self.keyword,
            name: self.name,
            ty: self.ty,
            align: self.align.map(Box::new),
            addrspace: self.addrspace.map(Box::new),
            linksection: self.linksection.map(Box::new),
        }
    }
}

enum PayloadSyntax {
    Plain,
    Pointer,
    Switch,
    For,
}

impl Parser {
    pub fn from<S: AsRef<str>>(source: S) -> Self {
        let scanner = Scanner::from(source);
        Self::from_scanner(scanner)
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let scanner = Scanner::from_file(path)?;
        Ok(Self::from_scanner(scanner))
    }

    /// Creates a parser from the scanner's remaining tokens.
    pub fn from_scanner(scanner: Scanner) -> Self {
        let path = scanner.path().to_path_buf();
        let line_info = scanner.lines().to_vec();
        let source_len = scanner.source().len();
        let scan_result = scanner.scan_all();
        let (tokens, scan_error) = match scan_result {
            Ok(tokens) => (tokens, None),
            Err(err) => (Vec::new(), Some(err)),
        };

        Self {
            tokens,
            cursor: 0,
            path,
            line_info,
            source_len,
            comments: Vec::new(),
            depth: 0,
            suppress_curly_suffix: 0,
            scan_error,
        }
    }

    pub fn parse_file(mut self) -> Result<ast::File, Error> {
        self.check_scan_error()?;
        self.validate_token_stream()?;

        let container_docs = self.collect_doc_comments(CommentKind::ContainerDoc);
        let members = self.parse_container_members_until(None)?;

        Ok(ast::File {
            path: self.path,
            line_info: self.line_info,
            container_docs,
            members,
            comments: self.comments,
        })
    }

    pub fn expression(&mut self) -> Result<ast::Expression, Error> {
        self.check_scan_error()?;
        self.validate_balanced_delimiters()?;
        let expr = self.parse_expr_bp(0)?;
        if !self.is_eof() {
            return Err(self.unexpected(vec![], self.current().map(|token| token.token.clone())));
        }
        Ok(expr)
    }

    pub fn parse_statement(&mut self) -> Result<ast::Statement, Error> {
        self.check_scan_error()?;
        self.skip_comments();
        let mark = self.preback();
        match self.try_parse_statement() {
            Ok(statement) => Ok(statement),
            Err(err) => {
                self.goback(mark);
                Err(err)
            }
        }
    }

    fn try_parse_statement(&mut self) -> Result<ast::Statement, Error> {
        self.with_depth(Self::parse_statement_inner)
    }

    fn parse_statement_inner(&mut self) -> Result<ast::Statement, Error> {
        if self.current_is_symbol(Symbol::LBrace) {
            return Ok(ast::Statement::Block(Box::new(self.parse_block()?)));
        }
        if self.current_is_keyword(Keyword::Const) || self.current_is_keyword(Keyword::Var) {
            return Ok(ast::Statement::Var(Box::new(
                self.parse_var_decl_expr_statement()?,
            )));
        }
        if self.current_is_keyword(Keyword::Comptime) {
            return self.parse_prefixed_statement(Keyword::Comptime);
        }
        if self.current_is_keyword(Keyword::Nosuspend) {
            return self.parse_prefixed_statement(Keyword::Nosuspend);
        }
        if self.current_is_keyword(Keyword::Suspend) {
            return self.parse_prefixed_statement(Keyword::Suspend);
        }
        if self.current_is_keyword(Keyword::Defer) || self.current_is_keyword(Keyword::Errdefer) {
            return self.parse_defer_statement();
        }
        if self.current_is_keyword(Keyword::If) {
            return self.parse_if_statement();
        }
        if self.current_is_keyword(Keyword::Inline)
            || self.current_is_keyword(Keyword::While)
            || self.current_is_keyword(Keyword::For)
        {
            return self.parse_loop_statement();
        }
        if self.current_is_keyword(Keyword::Switch) {
            let mark = self.preback();
            let pos = self.current_pos();
            let expr = self.parse_switch_expr()?;
            let tokens = self.tokens_since(mark);
            return Ok(ast::Statement::Switch(Box::new(ast::SwitchStmt {
                pos,
                label: None,
                expr,
                tokens,
            })));
        }
        if self.current_is_block_label() {
            return self.parse_labeled_statement();
        }

        self.parse_expr_or_assign_statement(true)
    }

    fn parse_prefixed_statement(&mut self, keyword: Keyword) -> Result<ast::Statement, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        self.expect_keyword(keyword)?;
        let body = Box::new(self.parse_prefixed_body(keyword == Keyword::Comptime)?);
        let tokens = self.tokens_since(mark);
        Ok(match keyword {
            Keyword::Comptime => {
                ast::Statement::Comptime(Box::new(ast::ComptimeStmt { pos, body, tokens }))
            }
            Keyword::Nosuspend => {
                ast::Statement::Nosuspend(Box::new(ast::NosuspendStmt { pos, body, tokens }))
            }
            Keyword::Suspend => {
                ast::Statement::Suspend(Box::new(ast::SuspendStmt { pos, body, tokens }))
            }
            _ => return Err(self.error_at(pos, "unsupported statement prefix")),
        })
    }

    fn parse_defer_statement(&mut self) -> Result<ast::Statement, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        let err = if self.consume_keyword(Keyword::Errdefer) {
            true
        } else {
            self.expect_keyword(Keyword::Defer)?;
            false
        };
        let payload = if err {
            self.parse_optional_payload(PayloadSyntax::Plain)?
        } else {
            None
        };
        let body = Box::new(self.parse_prefixed_body(false)?);
        let tokens = self.tokens_since(mark);
        Ok(ast::Statement::Defer(Box::new(ast::DeferStmt {
            pos,
            err,
            payload,
            body,
            tokens,
        })))
    }

    fn parse_prefixed_body(&mut self, allow_declarations: bool) -> Result<ast::Statement, Error> {
        if self.current_is_block_expr()
            || (allow_declarations
                && (self.current_is_keyword(Keyword::Const)
                    || self.current_is_keyword(Keyword::Var)))
        {
            return self.try_parse_statement();
        }
        self.parse_expr_or_assign_statement(allow_declarations)
    }

    fn parse_if_statement(&mut self) -> Result<ast::Statement, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        let (condition, payload) = self.parse_if_prefix()?;
        let (then_branch, then_semicolon) = self.parse_control_body_statement()?;
        self.skip_comments();
        let (else_payload, else_branch) = if self.consume_keyword(Keyword::Else) {
            let payload = self.parse_optional_payload(PayloadSyntax::Plain)?;
            (payload, Some(Box::new(self.try_parse_statement()?)))
        } else {
            if !then_semicolon && !matches!(&then_branch, ast::Statement::Block(_)) {
                return Err(self.unexpected(
                    vec![
                        TokenKind::Symbol(Symbol::Semicolon),
                        TokenKind::Keyword(Keyword::Else),
                    ],
                    self.current().map(|token| token.token.clone()),
                ));
            }
            (None, None)
        };
        let tokens = self.tokens_since(mark);
        Ok(ast::Statement::If(Box::new(ast::IfStmt {
            pos,
            condition,
            payload,
            then_branch: Box::new(then_branch),
            else_payload,
            else_branch,
            tokens,
        })))
    }

    fn parse_control_body_statement(&mut self) -> Result<(ast::Statement, bool), Error> {
        self.skip_comments();
        if self.current_is_block_expr() {
            return Ok((self.try_parse_statement()?, false));
        }

        let mark = self.preback();
        let pos = self.current_pos();
        let expr = self.parse_assign_expr()?;
        let semicolon = self.consume_symbol(Symbol::Semicolon);
        if semicolon && self.current_is_keyword(Keyword::Else) {
            return Err(self.error_at(self.current_pos(), "unexpected 'else' after a semicolon"));
        }
        let tokens = self.tokens_since(mark);
        Ok((
            ast::Statement::Expr(Box::new(ast::ExprStmt {
                pos,
                expr,
                semicolon,
                tokens,
            })),
            semicolon,
        ))
    }

    fn parse_loop_statement(&mut self) -> Result<ast::Statement, Error> {
        let mark = self.preback();
        self.parse_loop_statement_from(None, mark)
    }

    fn parse_loop_statement_from(
        &mut self,
        label: Option<ast::Ident>,
        mark: ParserMark,
    ) -> Result<ast::Statement, Error> {
        let pos = label
            .as_ref()
            .map_or_else(|| self.current_pos(), |label| label.pos);
        let inline_ = self.consume_keyword(Keyword::Inline);
        let kind = if self.current_is_keyword(Keyword::While) {
            let (condition, payload, continue_expr) = self.parse_while_prefix()?;
            ast::LoopKind::While {
                condition: Box::new(condition),
                payload,
                continue_expr: continue_expr.map(Box::new),
            }
        } else {
            let (items, payload) = self.parse_for_prefix()?;
            ast::LoopKind::For { items, payload }
        };
        let (body, body_semicolon) = self.parse_control_body_statement()?;
        self.skip_comments();
        let (else_payload, else_branch) = if self.consume_keyword(Keyword::Else) {
            let payload = match kind {
                ast::LoopKind::While { .. } => self.parse_optional_payload(PayloadSyntax::Plain)?,
                ast::LoopKind::For { .. } => None,
            };
            (payload, Some(Box::new(self.try_parse_statement()?)))
        } else {
            if !body_semicolon && !matches!(&body, ast::Statement::Block(_)) {
                return Err(self.unexpected(
                    vec![
                        TokenKind::Symbol(Symbol::Semicolon),
                        TokenKind::Keyword(Keyword::Else),
                    ],
                    self.current().map(|token| token.token.clone()),
                ));
            }
            (None, None)
        };
        let tokens = self.tokens_since(mark);
        Ok(ast::Statement::Loop(Box::new(ast::LoopStmt {
            pos,
            label,
            inline_,
            kind,
            body: Box::new(body),
            else_payload,
            else_branch,
            tokens,
        })))
    }

    fn parse_labeled_statement(&mut self) -> Result<ast::Statement, Error> {
        let mark = self.preback();
        let Some(label) = self.parse_block_label()? else {
            return Err(self.error_at(self.current_pos(), "expected block label"));
        };
        let pos = label.pos;
        if self.current_is_symbol(Symbol::LBrace) {
            let mut block = self.parse_block()?;
            block.label = Some(label);
            return Ok(ast::Statement::Block(Box::new(block)));
        }
        if self.current_is_keyword(Keyword::Inline)
            || self.current_is_keyword(Keyword::While)
            || self.current_is_keyword(Keyword::For)
        {
            return self.parse_loop_statement_from(Some(label), mark);
        }
        if self.current_is_keyword(Keyword::Switch) {
            let expr = self.parse_switch_expr_with_label(Some(label.clone()))?;
            let tokens = self.tokens_since(mark);
            return Ok(ast::Statement::Switch(Box::new(ast::SwitchStmt {
                pos,
                label: Some(label),
                expr,
                tokens,
            })));
        }
        Err(self.unexpected(
            vec![
                TokenKind::Symbol(Symbol::LBrace),
                TokenKind::Keyword(Keyword::Inline),
                TokenKind::Keyword(Keyword::While),
                TokenKind::Keyword(Keyword::For),
                TokenKind::Keyword(Keyword::Switch),
            ],
            self.current().map(|token| token.token.clone()),
        ))
    }

    fn parse_expr_or_assign_statement(
        &mut self,
        allow_declarations: bool,
    ) -> Result<ast::Statement, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        let first_lhs = self.parse_expr_bp(0)?;
        if self.consume_symbol(Symbol::Comma) {
            let mut assign_lhs = vec![first_lhs.clone()];
            let mut var_lhs = vec![ast::VarProtoOrExpr::Expr(first_lhs)];
            let mut has_var_proto = false;
            let mut first_keyword = Keyword::Const;
            loop {
                if self.current_is_keyword(Keyword::Const) || self.current_is_keyword(Keyword::Var)
                {
                    if !allow_declarations {
                        return Err(self.error_at(self.current_pos(), "expected assignment target"));
                    }
                    let proto = self.parse_local_var_proto()?;
                    if !has_var_proto {
                        first_keyword = proto.keyword;
                    }
                    has_var_proto = true;
                    var_lhs.push(proto.into_later_binding());
                } else {
                    let expr = self.parse_expr_bp(0)?;
                    assign_lhs.push(expr.clone());
                    var_lhs.push(ast::VarProtoOrExpr::Expr(expr));
                }
                if !self.consume_symbol(Symbol::Comma) {
                    break;
                }
            }
            self.expect_symbol(Symbol::Equal)?;
            let rhs = self.parse_expr_bp(0)?;
            self.expect_symbol(Symbol::Semicolon)?;
            let tokens = self.tokens_since(mark);
            if has_var_proto {
                return Ok(ast::Statement::Var(Box::new(ast::VarDecl {
                    pos,
                    docs: Vec::new(),
                    visibility: None,
                    extern_export: None,
                    threadlocal: false,
                    keyword: first_keyword,
                    lhs: var_lhs,
                    align: None,
                    addrspace: None,
                    linksection: None,
                    init: Some(rhs),
                    tokens,
                })));
            }
            return Ok(ast::Statement::Assign(Box::new(ast::AssignStmt {
                pos,
                lhs: assign_lhs,
                op: Token::Symbol(Symbol::Equal),
                rhs,
                tokens,
            })));
        }
        let lhs = first_lhs;
        if self.current().is_some_and(|token| match token.token {
            Token::Symbol(symbol) => symbol.is_assign_op(),
            _ => false,
        }) {
            let op = self.bump_token().expect("assign op exists");
            let rhs = self.parse_expr_bp(0)?;
            self.expect_symbol(Symbol::Semicolon)?;
            let tokens = self.tokens_since(mark);
            return Ok(ast::Statement::Assign(Box::new(ast::AssignStmt {
                pos,
                lhs: vec![lhs],
                op,
                rhs,
                tokens,
            })));
        }

        self.expect_symbol(Symbol::Semicolon)?;
        let tokens = self.tokens_since(mark);
        Ok(ast::Statement::Expr(Box::new(ast::ExprStmt {
            pos,
            expr: lhs,
            semicolon: true,
            tokens,
        })))
    }

    fn parse_var_decl_expr_statement(&mut self) -> Result<ast::VarDecl, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        let first = self.parse_local_var_proto()?;
        let mut lhs = vec![ast::VarProtoOrExpr::VarProto {
            pos: first.name.pos,
            keyword: first.keyword,
            name: first.name,
            ty: first.ty,
            align: None,
            addrspace: None,
            linksection: None,
        }];
        let mut destructuring = false;
        while self.consume_symbol(Symbol::Comma) {
            destructuring = true;
            if self.current_is_keyword(Keyword::Const) || self.current_is_keyword(Keyword::Var) {
                let proto = self.parse_local_var_proto()?;
                lhs.push(proto.into_later_binding());
            } else {
                lhs.push(ast::VarProtoOrExpr::Expr(self.parse_expr_bp(0)?));
            }
        }

        let init = if self.consume_symbol(Symbol::Equal) {
            Some(self.parse_expr_bp(0)?)
        } else {
            if destructuring {
                return Err(self.error_at(
                    pos,
                    "destructuring variable declarations require an initializer",
                ));
            }
            None
        };
        self.expect_symbol(Symbol::Semicolon)?;
        let tokens = self.tokens_since(mark);
        Ok(ast::VarDecl {
            pos,
            docs: Vec::new(),
            visibility: None,
            extern_export: None,
            threadlocal: false,
            keyword: first.keyword,
            lhs,
            align: first.align,
            addrspace: first.addrspace,
            linksection: first.linksection,
            init,
            tokens,
        })
    }

    fn parse_local_var_proto(&mut self) -> Result<LocalVarProto, Error> {
        let keyword = if self.consume_keyword(Keyword::Const) {
            Keyword::Const
        } else {
            self.expect_keyword(Keyword::Var)?;
            Keyword::Var
        };
        let name = self.expect_ident(self.current_pos())?;
        let ty = if self.consume_symbol(Symbol::Colon) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        let align = self.parse_expr_attr(Keyword::Align)?;
        let addrspace = self.parse_expr_attr(Keyword::Addrspace)?;
        let linksection = self.parse_expr_attr(Keyword::Linksection)?;
        Ok(LocalVarProto {
            keyword,
            name,
            ty,
            align,
            addrspace,
            linksection,
        })
    }

    fn parse_container_member(
        &mut self,
        docs: Vec<Arc<ast::Comment>>,
    ) -> Result<ast::ContainerMember, Error> {
        if self.current_is_symbol(Symbol::Comma) || self.current_is_symbol(Symbol::Semicolon) {
            return Err(self.error_at(self.current_pos(), "unexpected container member separator"));
        }
        self.try_parse_container_member(docs)
    }

    fn try_parse_container_member(
        &mut self,
        docs: Vec<Arc<ast::Comment>>,
    ) -> Result<ast::ContainerMember, Error> {
        if self.current_is_keyword(Keyword::Test) {
            self.reject_unattached_docs(&docs)?;
            return self.parse_test_decl(docs);
        }

        if self.current_is_keyword(Keyword::Comptime)
            && self
                .peek_non_comment(1)
                .is_some_and(|token| matches!(token.token, Token::Symbol(Symbol::LBrace)))
        {
            self.reject_unattached_docs(&docs)?;
            return self.parse_comptime_decl(docs);
        }

        let mark = self.preback();
        let pos = self.current_pos();
        let visibility = self.consume_keyword(Keyword::Pub).then_some(Keyword::Pub);
        let modifier = self.parse_decl_modifier()?;
        if self.current_is_keyword(Keyword::Fn) {
            return self.parse_fn_decl(mark, pos, docs, visibility, modifier);
        }

        if matches!(
            self.current().map(|token| &token.token),
            Some(Token::Keyword(
                Keyword::Threadlocal | Keyword::Const | Keyword::Var
            ))
        ) || matches!(
            modifier,
            Some(ast::FnModifier::Export | ast::FnModifier::Extern(_))
        ) {
            let var_modifier = match modifier {
                Some(ast::FnModifier::Export) => Some(ast::VarModifier::Export),
                Some(ast::FnModifier::Extern(abi)) => Some(ast::VarModifier::Extern(abi)),
                Some(ast::FnModifier::Inline | ast::FnModifier::NoInline) => {
                    return Err(self.error_at(self.current_pos(), "inline modifier requires fn"));
                }
                None => None,
            };
            return self.parse_global_var_decl(mark, pos, docs, visibility, var_modifier);
        }

        if modifier.is_some() || visibility.is_some() {
            return Err(self.error_at(
                self.current_pos(),
                "declaration modifier requires declaration",
            ));
        }

        self.parse_container_field(docs)
    }

    fn parse_test_decl(
        &mut self,
        docs: Vec<Arc<ast::Comment>>,
    ) -> Result<ast::ContainerMember, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        self.expect_keyword(Keyword::Test)?;
        let name = match self.current().cloned().map(|token| token.token) {
            Some(Token::Literal(LitKind::String, text)) => {
                let pos = self.current_pos();
                self.advance();
                Some(ast::Expression::BasicLit(ast::BasicLit {
                    pos,
                    kind: LitKind::String,
                    text,
                }))
            }
            Some(Token::Literal(LitKind::Ident, _)) => Some(ast::Expression::Ident(
                self.expect_ident(self.current_pos())?,
            )),
            _ => None,
        };
        let body = self.parse_block()?;
        let tokens = self.tokens_since(mark);
        Ok(ast::ContainerMember::Test(Box::new(ast::TestDecl {
            pos,
            docs,
            name,
            body,
            tokens,
        })))
    }

    fn parse_comptime_decl(
        &mut self,
        docs: Vec<Arc<ast::Comment>>,
    ) -> Result<ast::ContainerMember, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        self.expect_keyword(Keyword::Comptime)?;
        let block = self.parse_block()?;
        let tokens = self.tokens_since(mark);
        Ok(ast::ContainerMember::Comptime(Box::new(
            ast::ComptimeDecl {
                pos,
                docs,
                block,
                tokens,
            },
        )))
    }

    fn parse_decl_modifier(&mut self) -> Result<Option<ast::FnModifier>, Error> {
        if self.consume_keyword(Keyword::Export) {
            return Ok(Some(ast::FnModifier::Export));
        }
        if self.consume_keyword(Keyword::Extern) {
            let abi = match self.current().map(|token| &token.token) {
                Some(Token::Literal(LitKind::String, text)) => {
                    let text = text.clone();
                    self.advance();
                    Some(text)
                }
                _ => None,
            };
            return Ok(Some(ast::FnModifier::Extern(abi)));
        }
        if self.consume_keyword(Keyword::Inline) {
            return Ok(Some(ast::FnModifier::Inline));
        }
        if self.consume_keyword(Keyword::Noinline) {
            return Ok(Some(ast::FnModifier::NoInline));
        }
        Ok(None)
    }

    fn parse_fn_decl(
        &mut self,
        mark: ParserMark,
        pos: usize,
        docs: Vec<Arc<ast::Comment>>,
        visibility: Option<Keyword>,
        modifier: Option<ast::FnModifier>,
    ) -> Result<ast::ContainerMember, Error> {
        let mut decl = self.parse_fn_proto(docs, visibility, modifier)?;
        decl.pos = pos;
        decl.body = if self.consume_symbol(Symbol::Semicolon) {
            None
        } else {
            Some(self.parse_block()?)
        };
        decl.tokens = self.tokens_since(mark);
        Ok(ast::ContainerMember::Fn(Box::new(decl)))
    }

    /// FnProto <- KEYWORD_fn IDENTIFIER? LPAREN ParamDeclList RPAREN ByteAlign? AddrSpace? LinkSection? CallConv? EXCLAMATIONMARK? TypeExpr
    fn parse_fn_proto(
        &mut self,
        docs: Vec<Arc<ast::Comment>>,
        visibility: Option<Keyword>,
        modifier: Option<ast::FnModifier>,
    ) -> Result<ast::FnDecl, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        self.expect_keyword(Keyword::Fn)?;
        let name = if self.current_is_kind(TokenKind::Literal(LitKind::Ident)) {
            Some(self.expect_ident(self.current_pos())?)
        } else {
            None
        };
        self.expect_symbol(Symbol::LParen)?;
        let params = self.parse_param_decl_list()?;
        self.expect_symbol(Symbol::RParen)?;
        let align = self.parse_expr_attr(Keyword::Align)?;
        let addrspace = self.parse_expr_attr(Keyword::Addrspace)?;
        let linksection = self.parse_expr_attr(Keyword::Linksection)?;
        let callconv = self.parse_expr_attr(Keyword::Callconv)?;
        let bang = self.consume_symbol(Symbol::ExclamationMark);
        let ret = Some(Box::new(self.parse_type_expr()?));
        let tokens = self.tokens_since(mark);

        Ok(ast::FnDecl {
            pos,
            docs,
            visibility,
            modifier,
            name,
            params,
            align,
            addrspace,
            linksection,
            callconv,
            bang,
            ret,
            body: None,
            tokens,
        })
    }

    fn parse_param_decl_list(&mut self) -> Result<Vec<ast::ParamDecl>, Error> {
        let mut params = Vec::new();
        while !self.current_is_symbol(Symbol::RParen) {
            let docs = self.collect_doc_comments(CommentKind::Doc);
            if self.current_is_symbol(Symbol::RParen) {
                self.reject_unattached_docs(&docs)?;
                break;
            }
            let pos = self.current_pos();
            if self.consume_symbol(Symbol::Dot3) {
                self.reject_unattached_docs(&docs)?;
                params.push(ast::ParamDecl {
                    pos,
                    docs,
                    noalias: false,
                    comptime: false,
                    name: None,
                    ty: ast::ParamType::VarArgs(pos),
                });
            } else {
                let noalias = self.consume_keyword(Keyword::Noalias);
                let comptime = self.consume_keyword(Keyword::Comptime);
                let name = if self.current_is_kind(TokenKind::Literal(LitKind::Ident))
                    && self
                        .peek_non_comment(1)
                        .is_some_and(|token| matches!(token.token, Token::Symbol(Symbol::Colon)))
                {
                    let name = self.expect_ident(self.current_pos())?;
                    self.expect_symbol(Symbol::Colon)?;
                    Some(name)
                } else {
                    None
                };
                let ty = if self.consume_keyword(Keyword::Anytype) {
                    ast::ParamType::AnyType(pos)
                } else {
                    ast::ParamType::Type(Box::new(self.parse_type_expr()?))
                };
                params.push(ast::ParamDecl {
                    pos,
                    docs,
                    noalias,
                    comptime,
                    name,
                    ty,
                });
            }

            if !self.consume_symbol(Symbol::Comma) {
                break;
            }
        }
        Ok(params)
    }

    /// GlobalVarDecl <- VarDeclProto (EQUAL Expr)? SEMICOLON
    fn parse_global_var_decl(
        &mut self,
        mark: ParserMark,
        pos: usize,
        docs: Vec<Arc<ast::Comment>>,
        visibility: Option<Keyword>,
        extern_export: Option<ast::VarModifier>,
    ) -> Result<ast::ContainerMember, Error> {
        let proto_pos = self.current_pos();
        let threadlocal = self.consume_keyword(Keyword::Threadlocal);
        let keyword = if self.consume_keyword(Keyword::Const) {
            Keyword::Const
        } else {
            self.expect_keyword(Keyword::Var)?;
            Keyword::Var
        };
        let name = self.expect_ident(self.current_pos())?;
        let ty = if self.consume_symbol(Symbol::Colon) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        let align = self.parse_expr_attr(Keyword::Align)?;
        let addrspace = self.parse_expr_attr(Keyword::Addrspace)?;
        let linksection = self.parse_expr_attr(Keyword::Linksection)?;
        let init = if self.consume_symbol(Symbol::Equal) {
            Some(self.parse_expr_bp(0)?)
        } else {
            None
        };
        self.expect_symbol(Symbol::Semicolon)?;
        let tokens = self.tokens_since(mark);
        let lhs = vec![ast::VarProtoOrExpr::VarProto {
            pos: proto_pos,
            keyword,
            name,
            ty,
            align: None,
            addrspace: None,
            linksection: None,
        }];
        Ok(ast::ContainerMember::Var(Box::new(ast::VarDecl {
            pos,
            docs,
            visibility,
            extern_export,
            threadlocal,
            keyword,
            lhs,
            align,
            addrspace,
            linksection,
            init,
            tokens,
        })))
    }

    fn parse_container_field(
        &mut self,
        docs: Vec<Arc<ast::Comment>>,
    ) -> Result<ast::ContainerMember, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        let comptime = self.consume_keyword(Keyword::Comptime);
        if self.current_is_keyword(Keyword::Fn) {
            return Err(self.error_at(self.current_pos(), "container field cannot start with fn"));
        }
        let name = if self.current_is_kind(TokenKind::Literal(LitKind::Ident))
            && self
                .peek_non_comment(1)
                .is_some_and(|token| matches!(token.token, Token::Symbol(Symbol::Colon)))
        {
            let name = self.expect_ident(self.current_pos())?;
            self.expect_symbol(Symbol::Colon)?;
            Some(name)
        } else {
            None
        };
        let ty = Some(self.parse_type_expr()?);
        let align = self.parse_expr_attr(Keyword::Align)?;
        let default = if self.consume_symbol(Symbol::Equal) {
            Some(self.parse_value_expr()?)
        } else {
            None
        };
        if self.current().is_some() && !self.current_is_symbol(Symbol::RBrace) {
            self.expect_symbol(Symbol::Comma)?;
        }
        let tokens = self.tokens_since(mark);
        Ok(ast::ContainerMember::Field(Box::new(ast::ContainerField {
            pos,
            docs,
            comptime,
            name,
            ty,
            align,
            default,
            tokens,
        })))
    }

    fn collect_doc_comments(&mut self, kind: CommentKind) -> Vec<Arc<ast::Comment>> {
        let mut docs = Vec::new();
        self.skip_comments();
        while matches!(self.current().map(|token| &token.token), Some(Token::Comment(actual, _)) if *actual == kind)
        {
            let comment = self
                .take_comment()
                .expect("documentation comment at cursor");
            docs.push(Arc::clone(&comment));
            self.comments.push(comment);
            self.skip_comments();
        }
        docs
    }

    fn reject_unattached_docs(&self, docs: &[Arc<ast::Comment>]) -> Result<(), Error> {
        if let Some(doc) = docs.first() {
            return Err(self.error_at(doc.pos, "unattached documentation comment"));
        }
        Ok(())
    }

    fn take_comment(&mut self) -> Option<Arc<ast::Comment>> {
        let current = self.tokens.get(self.cursor)?;
        let Token::Comment(kind, text) = &current.token else {
            return None;
        };
        self.cursor += 1;
        Some(Arc::new(ast::Comment {
            pos: current.pos,
            kind: *kind,
            text: text.clone(),
        }))
    }

    fn skip_comments(&mut self) {
        while self
            .tokens
            .get(self.cursor)
            .is_some_and(|token| matches!(token.token, Token::Comment(CommentKind::Line, _)))
        {
            let comment = self.take_comment().expect("line comment at cursor");
            self.comments.push(comment);
        }
    }

    /// Block <- LBRACE Statement* RBRACE
    fn parse_block(&mut self) -> Result<ast::Block, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        self.expect_symbol(Symbol::LBrace)?;
        let mut statements = Vec::new();
        while !self.current_is_symbol(Symbol::RBrace) {
            self.skip_comments();
            if self.current_is_symbol(Symbol::RBrace) {
                break;
            }
            if self.is_eof() {
                return Err(self.unexpected(vec![TokenKind::Symbol(Symbol::RBrace)], None));
            }
            statements.push(self.parse_statement()?);
        }
        self.expect_symbol(Symbol::RBrace)?;
        let tokens = self.tokens_since(mark);
        Ok(ast::Block {
            pos,
            label: None,
            statements,
            tokens,
        })
    }

    fn parse_type_expr(&mut self) -> Result<ast::Expression, Error> {
        self.suppress_curly_suffix += 1;
        let result = self.parse_expr_bp(0);
        self.suppress_curly_suffix = self.suppress_curly_suffix.saturating_sub(1);
        result
    }

    fn parse_value_expr(&mut self) -> Result<ast::Expression, Error> {
        let saved = self.suppress_curly_suffix;
        self.suppress_curly_suffix = 0;
        let result = self.parse_expr_bp(0);
        self.suppress_curly_suffix = saved;
        result
    }

    fn parse_expr_attr(&mut self, keyword: Keyword) -> Result<Option<ast::Expression>, Error> {
        if !self.consume_keyword(keyword) {
            return Ok(None);
        }
        self.expect_symbol(Symbol::LParen)?;
        let expr = self.parse_value_expr()?;
        self.expect_symbol(Symbol::RParen)?;
        Ok(Some(expr))
    }

    /// AssignExpr <- Expr (AssignOp Expr / (COMMA Expr)+ EQUAL Expr)?
    fn parse_assign_expr(&mut self) -> Result<ast::Expression, Error> {
        self.parse_assign_expr_inner(true)
    }

    /// SingleAssignExpr <- Expr (AssignOp Expr)?
    fn parse_single_assign_expr(&mut self) -> Result<ast::Expression, Error> {
        self.parse_assign_expr_inner(false)
    }

    fn parse_assign_expr_inner(&mut self, allow_multi: bool) -> Result<ast::Expression, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        let first_lhs = self.parse_expr_bp(0)?;
        if allow_multi && self.consume_symbol(Symbol::Comma) {
            let mut lhs = vec![first_lhs];
            loop {
                lhs.push(self.parse_expr_bp(0)?);
                if !self.consume_symbol(Symbol::Comma) {
                    break;
                }
            }
            self.expect_symbol(Symbol::Equal)?;
            let rhs = self.parse_expr_bp(0)?;
            let tokens = self.tokens_since(mark);
            return Ok(ast::Expression::Assign(ast::AssignExpr {
                pos,
                lhs,
                op: Token::Symbol(Symbol::Equal),
                rhs: Box::new(rhs),
                tokens,
            }));
        }

        if self.current().is_some_and(|token| match token.token {
            Token::Symbol(symbol) => symbol.is_assign_op(),
            _ => false,
        }) {
            let op = self.bump_token().expect("assign op exists");
            let rhs = self.parse_expr_bp(0)?;
            let tokens = self.tokens_since(mark);
            return Ok(ast::Expression::Assign(ast::AssignExpr {
                pos,
                lhs: vec![first_lhs],
                op,
                rhs: Box::new(rhs),
                tokens,
            }));
        }

        Ok(first_lhs)
    }

    /// Expr <- BoolOrExpr
    fn parse_expr_bp(&mut self, min_bp: u8) -> Result<ast::Expression, Error> {
        self.skip_comments();
        self.with_depth(|parser| parser.parse_expr_bp_inner(min_bp))
    }

    fn parse_expr_bp_inner(&mut self, min_bp: u8) -> Result<ast::Expression, Error> {
        let mut lhs = self.parse_prefix_expr(true)?;

        loop {
            self.skip_comments();
            let Some(op) = self.current_operator() else {
                break;
            };
            let precedence = op.precedence();
            if precedence == 0 || precedence < min_bp {
                break;
            }

            let op_pos = self.current_pos();
            self.advance();

            if op == Token::Keyword(Keyword::Catch) {
                let payload = self.parse_optional_payload(PayloadSyntax::Plain)?;
                let rhs = self.parse_expr_bp(precedence + 1)?;
                lhs = ast::Expression::Catch(ast::CatchExpr {
                    pos: op_pos,
                    lhs: Box::new(lhs),
                    payload,
                    rhs: Box::new(rhs),
                });
                continue;
            }

            let rhs = self.parse_expr_bp(precedence + 1)?;
            if token_is_comparison(&op)
                && self
                    .current_operator()
                    .is_some_and(|next| token_is_comparison(&next))
            {
                return Err(self.unexpected(
                    vec![TokenKind::Symbol(Symbol::Semicolon)],
                    self.current().map(|token| token.token.clone()),
                ));
            }

            lhs = ast::Expression::Binary(ast::BinaryOp {
                pos: op_pos,
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            });
        }

        Ok(lhs)
    }

    /// PrefixExpr <- PrefixOp* PrimaryExpr
    fn parse_prefix_expr(&mut self, allow_initializer: bool) -> Result<ast::Expression, Error> {
        let expr = if self.current_is_keyword(Keyword::Anyframe)
            && self
                .peek_non_comment(1)
                .is_some_and(|token| matches!(token.token, Token::Symbol(Symbol::MinusRArrow)))
        {
            let pos = self.current_pos();
            self.advance();
            self.expect_symbol(Symbol::MinusRArrow)?;
            let result = self.parse_type_prefix_operand()?;
            ast::Expression::AnyframeType(ast::AnyframeType {
                pos,
                result: Some(Box::new(result)),
            })
        } else if self.current_is_symbol(Symbol::QuestionMark) {
            let pos = self.current_pos();
            self.advance();
            let child = self.parse_type_prefix_operand()?;
            ast::Expression::Optional(ast::OptionalType {
                pos,
                child: Box::new(child),
            })
        } else if self.starts_type_prefix() {
            self.parse_prefix_type_op()?
        } else if self.current_is_prefix_op() {
            let pos = self.current_pos();
            let op = self.bump_token().expect("current token exists");
            let expr = self.with_depth(|parser| parser.parse_prefix_expr(allow_initializer))?;
            return Ok(ast::Expression::Unary(ast::UnaryOp {
                pos,
                op,
                expr: Box::new(expr),
            }));
        } else {
            self.parse_primary_expr()?
        };

        self.parse_suffix_tail(expr, allow_initializer)
    }

    fn parse_type_prefix_operand(&mut self) -> Result<ast::Expression, Error> {
        self.with_depth(|parser| parser.parse_prefix_expr(false))
    }

    /// PrefixTypeOp <- QUESTIONMARK / KEYWORD_anyframe MINUSRARROW / SliceTypeStart ... / PtrTypeStart ... / ArrayTypeStart
    fn parse_prefix_type_op(&mut self) -> Result<ast::Expression, Error> {
        if self.current_is_symbol(Symbol::Asterisk) || self.current_is_symbol(Symbol::Asterisk2) {
            return self.parse_pointer_type_prefix();
        }
        if self.current_is_symbol(Symbol::LBracket) {
            if self.peek_non_comment(1).is_some_and(|token| {
                matches!(
                    token.token,
                    Token::Symbol(Symbol::RBracket | Symbol::Colon | Symbol::Asterisk)
                )
            }) {
                if self
                    .peek_non_comment(1)
                    .is_some_and(|token| matches!(token.token, Token::Symbol(Symbol::Asterisk)))
                {
                    return self.parse_pointer_type_prefix();
                }
                return self.parse_slice_type_prefix();
            }
            return self.parse_array_type_prefix();
        }
        Err(self.unexpected(vec![], self.current().map(|token| token.token.clone())))
    }

    fn parse_pointer_type_prefix(&mut self) -> Result<ast::Expression, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        let double_asterisk = self.consume_symbol(Symbol::Asterisk2);
        let kind = if double_asterisk || self.consume_symbol(Symbol::Asterisk) {
            ast::PointerKind::One
        } else {
            self.expect_symbol(Symbol::LBracket)?;
            self.expect_symbol(Symbol::Asterisk)?;
            let kind = if self.consume_ident_text("c") {
                ast::PointerKind::C
            } else if self.consume_symbol(Symbol::Colon) {
                ast::PointerKind::Sentinel(Box::new(self.parse_value_expr()?))
            } else {
                ast::PointerKind::Many
            };
            self.expect_symbol(Symbol::RBracket)?;
            kind
        };
        let modifiers = self.parse_pointer_modifiers(true)?;
        let child = self.parse_type_prefix_operand()?;
        let mut pointer = ast::PointerType {
            pos,
            kind,
            modifiers,
            child: Box::new(child),
            tokens: self.tokens_since(mark),
        };
        if double_asterisk {
            let tokens = pointer.tokens.clone();
            pointer.pos += 1;
            pointer.tokens[0] = Token::Symbol(Symbol::Asterisk);
            pointer = ast::PointerType {
                pos,
                kind: ast::PointerKind::One,
                modifiers: Vec::new(),
                child: Box::new(ast::Expression::Pointer(pointer)),
                tokens,
            };
        }
        Ok(ast::Expression::Pointer(pointer))
    }

    fn parse_slice_type_prefix(&mut self) -> Result<ast::Expression, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        self.expect_symbol(Symbol::LBracket)?;
        let sentinel = if self.consume_symbol(Symbol::Colon) {
            Some(Box::new(self.parse_value_expr()?))
        } else {
            None
        };
        self.expect_symbol(Symbol::RBracket)?;
        let modifiers = self.parse_pointer_modifiers(false)?;
        let child = self.parse_type_prefix_operand()?;
        let tokens = self.tokens_since(mark);
        Ok(ast::Expression::SliceType(ast::SliceType {
            pos,
            sentinel,
            modifiers,
            child: Box::new(child),
            tokens,
        }))
    }

    fn parse_array_type_prefix(&mut self) -> Result<ast::Expression, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        self.expect_symbol(Symbol::LBracket)?;
        let len = self.parse_value_expr()?;
        let sentinel = if self.consume_symbol(Symbol::Colon) {
            Some(Box::new(self.parse_value_expr()?))
        } else {
            None
        };
        self.expect_symbol(Symbol::RBracket)?;
        let child = self.parse_type_prefix_operand()?;
        let tokens = self.tokens_since(mark);
        Ok(ast::Expression::ArrayType(ast::ArrayType {
            pos,
            len: Box::new(len),
            sentinel,
            child: Box::new(child),
            tokens,
        }))
    }

    fn parse_pointer_modifiers(
        &mut self,
        allow_bit_range: bool,
    ) -> Result<Vec<ast::PointerModifier>, Error> {
        let mut modifiers = Vec::new();
        loop {
            if self.consume_keyword(Keyword::Const) {
                modifiers.push(ast::PointerModifier::Const);
            } else if self.consume_keyword(Keyword::Volatile) {
                modifiers.push(ast::PointerModifier::Volatile);
            } else if self.consume_keyword(Keyword::Allowzero) {
                modifiers.push(ast::PointerModifier::AllowZero);
            } else if self.consume_keyword(Keyword::Addrspace) {
                self.expect_symbol(Symbol::LParen)?;
                let expr = self.parse_value_expr()?;
                self.expect_symbol(Symbol::RParen)?;
                modifiers.push(ast::PointerModifier::AddrSpace(Box::new(expr)));
            } else if self.consume_keyword(Keyword::Align) {
                self.expect_symbol(Symbol::LParen)?;
                let mut exprs = vec![self.parse_value_expr()?];
                if allow_bit_range && self.consume_symbol(Symbol::Colon) {
                    exprs.push(self.parse_value_expr()?);
                    self.expect_symbol(Symbol::Colon)?;
                    exprs.push(self.parse_value_expr()?);
                }
                self.expect_symbol(Symbol::RParen)?;
                modifiers.push(ast::PointerModifier::Align(exprs));
            } else {
                break;
            }
        }
        Ok(modifiers)
    }

    fn starts_type_prefix(&self) -> bool {
        self.current_is_symbol(Symbol::Asterisk)
            || self.current_is_symbol(Symbol::Asterisk2)
            || self.current_is_symbol(Symbol::LBracket)
    }

    /// SuffixExpr <- PrimaryTypeExpr (SuffixOp / FnCallArguments)*
    fn parse_suffix_tail(
        &mut self,
        mut expr: ast::Expression,
        allow_initializer: bool,
    ) -> Result<ast::Expression, Error> {
        loop {
            if self.current_is_symbol(Symbol::LParen) {
                let pos = expr.pos();
                let args = self.parse_delimited_exprs(Symbol::LParen, Symbol::RParen)?;
                expr = ast::Expression::Call(ast::Call {
                    pos,
                    callee: Box::new(expr),
                    args,
                });
                continue;
            }

            if self.current_is_symbol(Symbol::DotAsterisk) {
                let pos = self.current_pos();
                self.advance();
                expr = ast::Expression::Deref(ast::Deref {
                    pos,
                    expr: Box::new(expr),
                });
                continue;
            }

            if self.current_is_symbol(Symbol::DotQuestionMark) {
                let pos = self.current_pos();
                self.advance();
                expr = ast::Expression::Unwrap(ast::Unwrap {
                    pos,
                    expr: Box::new(expr),
                });
                continue;
            }

            if self.current_is_symbol(Symbol::Dot)
                && self
                    .peek_non_comment(1)
                    .is_some_and(|token| matches!(token.token, Token::Literal(LitKind::Ident, _)))
            {
                let pos = self.current_pos();
                self.advance();
                let field_pos = self.current_pos();
                let field = self.expect_ident(field_pos)?;
                expr = ast::Expression::FieldAccess(ast::FieldAccess {
                    pos,
                    expr: Box::new(expr),
                    field,
                });
                continue;
            }

            if self.current_is_symbol(Symbol::LBracket) {
                expr = self.parse_index_or_slice(expr)?;
                continue;
            }

            if allow_initializer
                && self.suppress_curly_suffix == 0
                && self.current_is_symbol(Symbol::LBrace)
            {
                let mark = self.preback();
                let pos = expr.pos();
                let entries = self.parse_init_list()?;
                let tokens = self.tokens_since(mark);
                expr = ast::Expression::InitList(ast::CurlyInit {
                    pos,
                    ty: Box::new(expr),
                    entries,
                    tokens,
                });
                continue;
            }

            break;
        }

        if self.current_is_symbol(Symbol::ExclamationMark) {
            let pos = self.current_pos();
            self.advance();
            let payload = self.with_depth(|parser| parser.parse_prefix_expr(allow_initializer))?;
            expr = ast::Expression::ErrorUnion(ast::ErrorUnionType {
                pos,
                error: Box::new(expr),
                payload: Box::new(payload),
            });
        }

        Ok(expr)
    }

    /// SuffixOp <- LBRACKET Expr (DOT2 (Expr? (COLON Expr)?)?)? RBRACKET
    fn parse_index_or_slice(&mut self, expr: ast::Expression) -> Result<ast::Expression, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        self.expect_symbol(Symbol::LBracket)?;
        let start = self.parse_expr_bp(0)?;
        if self.consume_symbol(Symbol::Dot2) {
            let end = if self.current_is_symbol(Symbol::Colon)
                || self.current_is_symbol(Symbol::RBracket)
            {
                None
            } else {
                Some(Box::new(self.parse_expr_bp(0)?))
            };
            let sentinel = if self.consume_symbol(Symbol::Colon) {
                Some(Box::new(self.parse_expr_bp(0)?))
            } else {
                None
            };
            self.expect_symbol(Symbol::RBracket)?;
            let tokens = self.tokens_since(mark);
            Ok(ast::Expression::Slice(ast::Slice {
                pos,
                expr: Box::new(expr),
                start: Some(Box::new(start)),
                end,
                sentinel,
                tokens,
            }))
        } else {
            self.expect_symbol(Symbol::RBracket)?;
            Ok(ast::Expression::Index(ast::Index {
                pos,
                expr: Box::new(expr),
                index: Box::new(start),
            }))
        }
    }

    /// InitList <- LBRACE FieldInit... / LBRACE Expr... / LBRACE RBRACE
    fn parse_init_list(&mut self) -> Result<Vec<ast::InitEntry>, Error> {
        self.expect_symbol(Symbol::LBrace)?;
        let mut entries = Vec::new();
        while !self.current_is_symbol(Symbol::RBrace) {
            self.skip_comments();
            if self.current_is_symbol(Symbol::RBrace) {
                break;
            }
            let is_field = self.current_is_symbol(Symbol::Dot)
                && self
                    .peek_non_comment(1)
                    .is_some_and(|token| matches!(token.token, Token::Literal(LitKind::Ident, _)))
                && self
                    .peek_non_comment(2)
                    .is_some_and(|token| matches!(token.token, Token::Symbol(Symbol::Equal)));
            if entries
                .first()
                .is_some_and(|first| matches!(first, ast::InitEntry::Field { .. }) != is_field)
            {
                return Err(self.error_at(
                    self.current_pos(),
                    "cannot mix field and positional initializers",
                ));
            }
            if is_field {
                self.expect_symbol(Symbol::Dot)?;
                let name = self.expect_ident(self.current_pos())?;
                self.expect_symbol(Symbol::Equal)?;
                let value = self.parse_value_expr()?;
                entries.push(ast::InitEntry::Field { name, value });
            } else {
                entries.push(ast::InitEntry::Expr(self.parse_value_expr()?));
            }
            if !self.consume_symbol(Symbol::Comma) {
                break;
            }
        }
        self.expect_symbol(Symbol::RBrace)?;
        Ok(entries)
    }

    /// PrimaryTypeExpr <- BUILTINIDENTIFIER FnCallArguments / CHAR_LITERAL / ... / IDENTIFIER
    fn parse_primary_expr(&mut self) -> Result<ast::Expression, Error> {
        let Some(current) = self.current().cloned() else {
            return Err(self.unexpected(vec![], None));
        };

        match current.token {
            Token::Literal(LitKind::Ident, name) => {
                if self.current_is_block_label() {
                    let Some(label) = self.parse_block_label()? else {
                        return Err(self.error_at(current.pos, "expected block label"));
                    };
                    if self.current_is_symbol(Symbol::LBrace) {
                        let mut block = self.parse_block()?;
                        block.label = Some(label);
                        return Ok(ast::Expression::Block(block));
                    }
                    if self.current_is_keyword(Keyword::Inline) {
                        self.advance();
                        if self.current_is_keyword(Keyword::While) {
                            return self.parse_while_expr(Some(label), true);
                        }
                        if self.current_is_keyword(Keyword::For) {
                            return self.parse_for_expr(Some(label), true);
                        }
                        return Err(self.unexpected(
                            vec![
                                TokenKind::Keyword(Keyword::While),
                                TokenKind::Keyword(Keyword::For),
                            ],
                            self.current().map(|token| token.token.clone()),
                        ));
                    }
                    if self.current_is_keyword(Keyword::While) {
                        return self.parse_while_expr(Some(label), false);
                    }
                    if self.current_is_keyword(Keyword::For) {
                        return self.parse_for_expr(Some(label), false);
                    }
                    if self.current_is_keyword(Keyword::Switch) {
                        return Ok(ast::Expression::Switch(
                            self.parse_switch_expr_with_label(Some(label))?,
                        ));
                    }
                    return Err(self.unexpected(
                        vec![
                            TokenKind::Symbol(Symbol::LBrace),
                            TokenKind::Keyword(Keyword::Inline),
                            TokenKind::Keyword(Keyword::While),
                            TokenKind::Keyword(Keyword::For),
                            TokenKind::Keyword(Keyword::Switch),
                        ],
                        self.current().map(|token| token.token.clone()),
                    ));
                }
                self.advance();
                Ok(ast::Expression::Ident(ast::Ident {
                    pos: current.pos,
                    name,
                }))
            }
            Token::Literal(LitKind::BuiltinIdent, name) => {
                self.advance();
                let args = self.parse_delimited_exprs(Symbol::LParen, Symbol::RParen)?;
                Ok(ast::Expression::BuiltinCall(ast::BuiltinCall {
                    pos: current.pos,
                    name,
                    args,
                }))
            }
            Token::Literal(kind, text)
                if matches!(
                    kind,
                    LitKind::Integer | LitKind::Float | LitKind::Char | LitKind::String
                ) =>
            {
                self.advance();
                Ok(ast::Expression::BasicLit(ast::BasicLit {
                    pos: current.pos,
                    kind,
                    text,
                }))
            }
            Token::Literal(LitKind::MultilineString, mut text) => {
                self.advance();
                while let Some(Token::Literal(LitKind::MultilineString, next)) =
                    self.current().map(|token| &token.token)
                {
                    text.push('\n');
                    text.push_str(next);
                    self.advance();
                }
                Ok(ast::Expression::MultilineStr(ast::MultilineStr {
                    pos: current.pos,
                    text,
                }))
            }
            Token::Keyword(Keyword::Unreachable) => {
                self.advance();
                Ok(ast::Expression::Unreachable(current.pos))
            }
            Token::Keyword(Keyword::Anyframe) => {
                self.advance();
                Ok(ast::Expression::AnyframeType(ast::AnyframeType {
                    pos: current.pos,
                    result: None,
                }))
            }
            Token::Keyword(Keyword::Error) => self.parse_error_expr(),
            Token::Keyword(Keyword::Fn) => {
                let decl = self.parse_fn_proto(Vec::new(), None, None)?;
                Ok(ast::Expression::FnProto(Box::new(decl)))
            }
            Token::Keyword(
                Keyword::Extern
                | Keyword::Packed
                | Keyword::Struct
                | Keyword::Enum
                | Keyword::Union
                | Keyword::Opaque,
            ) => self.parse_container_decl_expr(),
            Token::Symbol(Symbol::LParen) => {
                self.advance();
                let expr = self.parse_expr_bp(0)?;
                self.expect_symbol(Symbol::RParen)?;
                Ok(ast::Expression::Grouped(Box::new(expr)))
            }
            Token::Symbol(Symbol::Dot) => {
                self.advance();
                if self.current_is_symbol(Symbol::LBrace) {
                    let entries = self.parse_init_list()?;
                    return Ok(ast::Expression::AnonInit(ast::AnonInit {
                        pos: current.pos,
                        entries,
                    }));
                }
                let ident = self.expect_ident(self.current_pos())?;
                Ok(ast::Expression::EnumLiteral(ast::EnumLiteral {
                    pos: current.pos,
                    name: ident,
                }))
            }
            Token::Keyword(Keyword::Comptime) => {
                self.advance();
                let expr = self.parse_expr_bp(0)?;
                Ok(ast::Expression::Comptime(Box::new(expr)))
            }
            Token::Keyword(Keyword::Nosuspend) => {
                self.advance();
                let expr = self.parse_expr_bp(0)?;
                Ok(ast::Expression::Nosuspend(Box::new(expr)))
            }
            Token::Keyword(Keyword::Resume) => {
                self.advance();
                let expr = self.parse_expr_bp(0)?;
                Ok(ast::Expression::Resume(Box::new(expr)))
            }
            Token::Keyword(Keyword::If) => self.parse_if_expr(),
            Token::Keyword(Keyword::While) => self.parse_while_expr(None, false),
            Token::Keyword(Keyword::For) => self.parse_for_expr(None, false),
            Token::Keyword(Keyword::Switch) => {
                let switch = self.parse_switch_expr()?;
                Ok(ast::Expression::Switch(switch))
            }
            Token::Keyword(Keyword::Inline) => {
                self.advance();
                if self.current_is_keyword(Keyword::While) {
                    self.parse_while_expr(None, true)
                } else if self.current_is_keyword(Keyword::For) {
                    self.parse_for_expr(None, true)
                } else {
                    Err(self.unexpected(
                        vec![
                            TokenKind::Keyword(Keyword::While),
                            TokenKind::Keyword(Keyword::For),
                        ],
                        self.current().map(|token| token.token.clone()),
                    ))
                }
            }
            Token::Keyword(Keyword::Asm) => self.parse_asm_expr(),
            Token::Keyword(Keyword::Return) => {
                self.advance();
                let value = if self.current_stops_optional_expr() {
                    None
                } else {
                    Some(Box::new(self.parse_expr_bp(0)?))
                };
                Ok(ast::Expression::Return(ast::ReturnExpr {
                    pos: current.pos,
                    value,
                }))
            }
            Token::Keyword(Keyword::Break) => {
                self.advance();
                let label = self.parse_optional_break_label()?;
                let value = if self.current_stops_optional_expr() {
                    None
                } else {
                    Some(Box::new(self.parse_expr_bp(0)?))
                };
                Ok(ast::Expression::Break(ast::BreakExpr {
                    pos: current.pos,
                    label,
                    value,
                }))
            }
            Token::Keyword(Keyword::Continue) => {
                self.advance();
                let label = self.parse_optional_break_label()?;
                let value = if self.current_stops_optional_expr() {
                    None
                } else {
                    Some(Box::new(self.parse_expr_bp(0)?))
                };
                Ok(ast::Expression::Continue(ast::ContinueExpr {
                    pos: current.pos,
                    label,
                    value,
                }))
            }
            Token::Symbol(Symbol::LBrace) => {
                let block = self.parse_block()?;
                Ok(ast::Expression::Block(block))
            }
            actual => Err(self.unexpected(vec![], Some(actual))),
        }
    }

    /// ErrorSetDecl <- KEYWORD_error LBRACE IdentifierList RBRACE
    fn parse_error_expr(&mut self) -> Result<ast::Expression, Error> {
        let pos = self.current_pos();
        self.expect_keyword(Keyword::Error)?;
        if self.consume_symbol(Symbol::Dot) {
            let name = self.expect_ident(self.current_pos())?;
            return Ok(ast::Expression::ErrorValue(ast::ErrorValue { pos, name }));
        }
        self.expect_symbol(Symbol::LBrace)?;
        let mut names = Vec::new();
        while !self.current_is_symbol(Symbol::RBrace) {
            let docs = self.collect_doc_comments(CommentKind::Doc);
            if self.current_is_symbol(Symbol::RBrace) {
                self.reject_unattached_docs(&docs)?;
                break;
            }
            names.push(self.expect_ident(self.current_pos())?);
            if !self.consume_symbol(Symbol::Comma) {
                break;
            }
        }
        self.expect_symbol(Symbol::RBrace)?;
        Ok(ast::Expression::ErrorSetDecl(ast::ErrorSetDecl {
            pos,
            names,
        }))
    }

    /// ContainerDecl <- (KEYWORD_extern / KEYWORD_packed)? ContainerDeclAuto
    fn parse_container_decl_expr(&mut self) -> Result<ast::Expression, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        let layout = if self.consume_keyword(Keyword::Extern) {
            Some(Keyword::Extern)
        } else if self.consume_keyword(Keyword::Packed) {
            Some(Keyword::Packed)
        } else {
            None
        };
        let kind = match self.current().map(|token| &token.token) {
            Some(Token::Keyword(
                keyword @ (Keyword::Struct | Keyword::Enum | Keyword::Union | Keyword::Opaque),
            )) => {
                let keyword = *keyword;
                self.advance();
                keyword
            }
            _ => {
                return Err(self.unexpected(
                    vec![
                        TokenKind::Keyword(Keyword::Struct),
                        TokenKind::Keyword(Keyword::Enum),
                        TokenKind::Keyword(Keyword::Union),
                        TokenKind::Keyword(Keyword::Opaque),
                    ],
                    self.current().map(|token| token.token.clone()),
                ))
            }
        };
        let arg = if kind != Keyword::Opaque && self.consume_symbol(Symbol::LParen) {
            let arg = if kind == Keyword::Union && self.consume_keyword(Keyword::Enum) {
                let enum_mark = self.preback();
                let enum_pos = self.tokens[enum_mark.cursor - 1].pos;
                let tag_type = if self.consume_symbol(Symbol::LParen) {
                    let nested = self.parse_expr_bp(0)?;
                    self.expect_symbol(Symbol::RParen)?;
                    Some(Box::new(nested))
                } else {
                    None
                };
                ast::ContainerArg::Enum {
                    pos: enum_pos,
                    tag_type,
                    tokens: self.tokens_since(enum_mark),
                }
            } else {
                ast::ContainerArg::Expr(Box::new(self.parse_expr_bp(0)?))
            };
            self.expect_symbol(Symbol::RParen)?;
            Some(arg)
        } else {
            None
        };
        self.expect_symbol(Symbol::LBrace)?;
        let container_docs = self.collect_doc_comments(CommentKind::ContainerDoc);
        let members = self.parse_container_members_until(Some(Symbol::RBrace))?;
        self.expect_symbol(Symbol::RBrace)?;
        let tokens = self.tokens_since(mark);
        Ok(ast::Expression::Container(ast::ContainerDecl {
            pos,
            layout,
            kind,
            arg,
            members,
            tokens,
            container_docs,
        }))
    }

    fn parse_container_members_until(
        &mut self,
        end: Option<Symbol>,
    ) -> Result<Vec<ast::ContainerMember>, Error> {
        let mut members = Vec::new();
        let mut fields_closed = false;
        loop {
            let docs = self.collect_doc_comments(CommentKind::Doc);
            if end.is_some_and(|symbol| self.current_is_symbol(symbol)) {
                self.reject_unattached_docs(&docs)?;
                break;
            }
            if self.is_eof() {
                self.reject_unattached_docs(&docs)?;
                if let Some(symbol) = end {
                    return Err(self.unexpected(vec![TokenKind::Symbol(symbol)], None));
                }
                break;
            }
            let member = self.parse_container_member(docs)?;
            if let ast::ContainerMember::Field(field) = &member {
                if fields_closed {
                    return Err(self.error_at(
                        field.pos,
                        "declarations are not allowed between container fields",
                    ));
                }
            } else if matches!(members.last(), Some(ast::ContainerMember::Field(_))) {
                fields_closed = true;
            }
            members.push(member);
        }
        Ok(members)
    }

    /// IfPrefix <- KEYWORD_if LPAREN Expr RPAREN PtrPayload?
    fn parse_if_prefix(&mut self) -> Result<(ast::Expression, Option<ast::Payload>), Error> {
        self.expect_keyword(Keyword::If)?;
        self.expect_symbol(Symbol::LParen)?;
        let condition = self.parse_expr_bp(0)?;
        self.expect_symbol(Symbol::RParen)?;
        let payload = self.parse_optional_payload(PayloadSyntax::Pointer)?;
        Ok((condition, payload))
    }

    /// IfExpr <- IfPrefix Expr (KEYWORD_else Payload? Expr)?
    fn parse_if_expr(&mut self) -> Result<ast::Expression, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        let (condition, payload) = self.parse_if_prefix()?;
        let then_branch = self.parse_expr_bp(0)?;
        self.skip_comments();
        let (else_payload, else_branch) = if self.consume_keyword(Keyword::Else) {
            let payload = self.parse_optional_payload(PayloadSyntax::Plain)?;
            (payload, Some(Box::new(self.parse_expr_bp(0)?)))
        } else {
            (None, None)
        };
        let tokens = self.tokens_since(mark);
        Ok(ast::Expression::If(ast::IfExpr {
            pos,
            condition: Some(Box::new(condition)),
            payload,
            then_branch: Some(Box::new(then_branch)),
            else_payload,
            else_branch,
            tokens,
        }))
    }

    /// WhilePrefix <- KEYWORD_while LPAREN Expr RPAREN PtrPayload? WhileContinueExpr?
    fn parse_while_prefix(
        &mut self,
    ) -> Result<
        (
            ast::Expression,
            Option<ast::Payload>,
            Option<ast::Expression>,
        ),
        Error,
    > {
        self.expect_keyword(Keyword::While)?;
        self.expect_symbol(Symbol::LParen)?;
        let condition = self.parse_expr_bp(0)?;
        self.expect_symbol(Symbol::RParen)?;
        let payload = self.parse_optional_payload(PayloadSyntax::Pointer)?;
        let continue_expr = if self.consume_symbol(Symbol::Colon) {
            self.expect_symbol(Symbol::LParen)?;
            let expr = self.parse_assign_expr()?;
            self.expect_symbol(Symbol::RParen)?;
            Some(expr)
        } else {
            None
        };
        Ok((condition, payload, continue_expr))
    }

    /// WhileExpr <- WhilePrefix Expr (KEYWORD_else Payload? Expr)?
    fn parse_while_expr(
        &mut self,
        label: Option<ast::Ident>,
        inline_: bool,
    ) -> Result<ast::Expression, Error> {
        let mark = self.preback();
        let pos = label
            .as_ref()
            .map_or_else(|| self.current_pos(), |label| label.pos);
        let (condition, payload, continue_expr) = self.parse_while_prefix()?;
        let body = self.parse_expr_bp(0)?;
        self.skip_comments();
        let (else_payload, else_branch) = if self.consume_keyword(Keyword::Else) {
            let payload = self.parse_optional_payload(PayloadSyntax::Plain)?;
            (payload, Some(Box::new(self.parse_expr_bp(0)?)))
        } else {
            (None, None)
        };
        let tokens = self.tokens_since(mark);
        Ok(ast::Expression::While(ast::WhileExpr {
            pos,
            label,
            inline_,
            condition: Some(Box::new(condition)),
            payload,
            continue_expr: continue_expr.map(Box::new),
            body: Some(Box::new(body)),
            else_payload,
            else_branch,
            tokens,
        }))
    }

    /// ForPrefix <- KEYWORD_for LPAREN ForArgumentsList RPAREN PtrListPayload
    fn parse_for_prefix(&mut self) -> Result<(Vec<ast::ForItem>, ast::Payload), Error> {
        self.expect_keyword(Keyword::For)?;
        self.expect_symbol(Symbol::LParen)?;
        let mut items = vec![self.parse_for_item()?];
        while self.consume_symbol(Symbol::Comma) && !self.current_is_symbol(Symbol::RParen) {
            items.push(self.parse_for_item()?);
        }
        self.expect_symbol(Symbol::RParen)?;
        let payload = self
            .parse_optional_payload(PayloadSyntax::For)?
            .ok_or_else(|| {
                self.error_at(
                    self.current_pos(),
                    "for expression requires pointer list payload",
                )
            })?;
        Ok((items, payload))
    }

    /// ForItem <- Expr (DOT2 Expr?)?
    fn parse_for_item(&mut self) -> Result<ast::ForItem, Error> {
        let pos = self.current_pos();
        let expr = self.parse_expr_bp(0)?;
        let range = if self.consume_symbol(Symbol::Dot2) {
            if self.current_is_symbol(Symbol::Comma) || self.current_is_symbol(Symbol::RParen) {
                Some(None)
            } else {
                Some(Some(self.parse_expr_bp(0)?))
            }
        } else {
            None
        };
        Ok(ast::ForItem { pos, expr, range })
    }

    /// ForExpr <- ForPrefix Expr (KEYWORD_else Expr)?
    fn parse_for_expr(
        &mut self,
        label: Option<ast::Ident>,
        inline_: bool,
    ) -> Result<ast::Expression, Error> {
        let mark = self.preback();
        let pos = label
            .as_ref()
            .map_or_else(|| self.current_pos(), |label| label.pos);
        let (items, payload) = self.parse_for_prefix()?;
        let body = self.parse_expr_bp(0)?;
        self.skip_comments();
        let else_branch = if self.consume_keyword(Keyword::Else) {
            Some(Box::new(self.parse_expr_bp(0)?))
        } else {
            None
        };
        let tokens = self.tokens_since(mark);
        Ok(ast::Expression::For(ast::ForExpr {
            pos,
            label,
            inline_,
            items,
            payload: Some(payload),
            body: Some(Box::new(body)),
            else_branch,
            tokens,
        }))
    }

    /// SwitchExpr <- KEYWORD_switch LPAREN Expr RPAREN LBRACE SwitchProngList RBRACE
    fn parse_switch_expr(&mut self) -> Result<ast::SwitchExpr, Error> {
        self.parse_switch_expr_with_label(None)
    }

    fn parse_switch_expr_with_label(
        &mut self,
        label: Option<ast::Ident>,
    ) -> Result<ast::SwitchExpr, Error> {
        let mark = self.preback();
        let pos = label
            .as_ref()
            .map_or_else(|| self.current_pos(), |label| label.pos);
        self.expect_keyword(Keyword::Switch)?;
        self.expect_symbol(Symbol::LParen)?;
        let target = self.parse_expr_bp(0)?;
        self.expect_symbol(Symbol::RParen)?;
        self.expect_symbol(Symbol::LBrace)?;
        let mut prongs = Vec::new();
        while !self.current_is_symbol(Symbol::RBrace) {
            self.skip_comments();
            if self.current_is_symbol(Symbol::RBrace) {
                break;
            }
            prongs.push(self.parse_switch_prong()?);
            self.skip_comments();
            if !self.consume_symbol(Symbol::Comma) {
                break;
            }
        }
        self.expect_symbol(Symbol::RBrace)?;
        let tokens = self.tokens_since(mark);
        Ok(ast::SwitchExpr {
            pos,
            label,
            target: Some(Box::new(target)),
            prongs,
            tokens,
        })
    }

    /// SwitchProng <- KEYWORD_inline? SwitchCase EQUALRARROW PtrIndexPayload? SingleAssignExpr
    fn parse_switch_prong(&mut self) -> Result<ast::SwitchProng, Error> {
        let pos = self.current_pos();
        let inline_ = self.consume_keyword(Keyword::Inline);
        let cases = if self.consume_keyword(Keyword::Else) {
            vec![ast::SwitchItem::Else(pos)]
        } else {
            let mut cases = Vec::new();
            loop {
                let start = self.parse_expr_bp(0)?;
                if self.consume_symbol(Symbol::Dot3) {
                    let end = self.parse_expr_bp(0)?;
                    cases.push(ast::SwitchItem::Range(Box::new(start), Box::new(end)));
                } else {
                    cases.push(ast::SwitchItem::Expr(Box::new(start)));
                }
                if !self.consume_symbol(Symbol::Comma) {
                    break;
                }
                self.skip_comments();
                if self.current_is_symbol(Symbol::EqualRArrow) {
                    break;
                }
            }
            cases
        };
        self.expect_symbol(Symbol::EqualRArrow)?;
        let payload = self.parse_optional_payload(PayloadSyntax::Switch)?;
        let body = self.parse_single_assign_expr()?;
        Ok(ast::SwitchProng {
            pos,
            inline_,
            cases,
            payload,
            body,
        })
    }

    fn parse_optional_break_label(&mut self) -> Result<Option<ast::Ident>, Error> {
        if !self.consume_symbol(Symbol::Colon) {
            return Ok(None);
        }
        self.expect_ident(self.current_pos()).map(Some)
    }

    fn parse_block_label(&mut self) -> Result<Option<ast::Ident>, Error> {
        if !self.current_is_block_label() {
            return Ok(None);
        }
        let label = self.expect_ident(self.current_pos())?;
        self.expect_symbol(Symbol::Colon)?;
        Ok(Some(label))
    }

    /// AsmExpr <- KEYWORD_asm KEYWORD_volatile? LPAREN Expr AsmOutput? RPAREN
    fn parse_asm_expr(&mut self) -> Result<ast::Expression, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        self.expect_keyword(Keyword::Asm)?;
        let volatile = self.consume_keyword(Keyword::Volatile);
        self.expect_symbol(Symbol::LParen)?;
        let expr = self.parse_expr_bp(0)?;
        let mut outputs = Vec::new();
        let mut inputs = Vec::new();
        let mut clobbers = None;
        if self.consume_symbol(Symbol::Colon) {
            outputs = self.parse_asm_output_list()?;
            if self.consume_symbol(Symbol::Colon) {
                inputs = self.parse_asm_input_list()?;
                if self.consume_symbol(Symbol::Colon) {
                    clobbers = Some(Box::new(self.parse_expr_bp(0)?));
                }
            }
        }
        self.expect_symbol(Symbol::RParen)?;
        let tokens = self.tokens_since(mark);
        Ok(ast::Expression::Asm(ast::AsmExpr {
            pos,
            volatile,
            expr: Some(Box::new(expr)),
            outputs,
            inputs,
            clobbers,
            tokens,
        }))
    }

    fn parse_asm_output_list(&mut self) -> Result<Vec<ast::AsmOutput>, Error> {
        let mut outputs = Vec::new();
        while self.current_is_symbol(Symbol::LBracket) {
            let pos = self.current_pos();
            self.expect_symbol(Symbol::LBracket)?;
            let name = self.expect_ident(self.current_pos())?;
            self.expect_symbol(Symbol::RBracket)?;
            let constraint = self.parse_string_literal_expr()?;
            self.expect_symbol(Symbol::LParen)?;
            let target = if self.consume_symbol(Symbol::MinusRArrow) {
                ast::AsmOutputTarget::Type(self.parse_type_expr()?)
            } else {
                ast::AsmOutputTarget::Ident(self.expect_ident(self.current_pos())?)
            };
            self.expect_symbol(Symbol::RParen)?;
            outputs.push(ast::AsmOutput {
                pos,
                name,
                constraint,
                target,
            });
            if !self.consume_symbol(Symbol::Comma) {
                break;
            }
        }
        Ok(outputs)
    }

    fn parse_asm_input_list(&mut self) -> Result<Vec<ast::AsmInput>, Error> {
        let mut inputs = Vec::new();
        while self.current_is_symbol(Symbol::LBracket) {
            let pos = self.current_pos();
            self.expect_symbol(Symbol::LBracket)?;
            let name = self.expect_ident(self.current_pos())?;
            self.expect_symbol(Symbol::RBracket)?;
            let constraint = self.parse_string_literal_expr()?;
            self.expect_symbol(Symbol::LParen)?;
            let expr = self.parse_expr_bp(0)?;
            self.expect_symbol(Symbol::RParen)?;
            inputs.push(ast::AsmInput {
                pos,
                name,
                constraint,
                expr,
            });
            if !self.consume_symbol(Symbol::Comma) {
                break;
            }
        }
        Ok(inputs)
    }

    fn parse_string_literal_expr(&mut self) -> Result<ast::Expression, Error> {
        let Some(token) = self.current().cloned() else {
            return Err(self.unexpected(vec![TokenKind::Literal(LitKind::String)], None));
        };
        match token.token {
            Token::Literal(LitKind::String, text) => {
                self.advance();
                Ok(ast::Expression::BasicLit(ast::BasicLit {
                    pos: token.pos,
                    kind: LitKind::String,
                    text,
                }))
            }
            actual => Err(self.unexpected(vec![TokenKind::Literal(LitKind::String)], Some(actual))),
        }
    }

    fn parse_delimited_exprs(
        &mut self,
        open: Symbol,
        close: Symbol,
    ) -> Result<Vec<ast::Expression>, Error> {
        self.expect_symbol(open)?;
        let mut args = Vec::new();
        while !self.current_is_symbol(close) {
            self.skip_comments();
            if self.current_is_symbol(close) {
                break;
            }
            if self.is_eof() {
                return Err(self.unexpected(vec![TokenKind::Symbol(close)], None));
            }
            args.push(self.parse_value_expr()?);
            if self.current_is_symbol(Symbol::Comma) {
                self.advance();
                if self.current_is_symbol(close) {
                    break;
                }
            } else if !self.current_is_symbol(close) {
                return Err(self.unexpected(
                    vec![TokenKind::Symbol(Symbol::Comma), TokenKind::Symbol(close)],
                    self.current().map(|token| token.token.clone()),
                ));
            }
        }
        self.expect_symbol(close)?;
        Ok(args)
    }

    fn parse_optional_payload(
        &mut self,
        syntax: PayloadSyntax,
    ) -> Result<Option<ast::Payload>, Error> {
        if !self.current_is_symbol(Symbol::Pipe) {
            return Ok(None);
        }
        let pos = self.current_pos();
        self.advance();
        let star = !matches!(syntax, PayloadSyntax::Plain) && self.consume_symbol(Symbol::Asterisk);
        let mut stars = vec![star];
        let mut names = vec![self.expect_ident(self.current_pos())?];
        match syntax {
            PayloadSyntax::Switch if self.consume_symbol(Symbol::Comma) => {
                stars.push(false);
                names.push(self.expect_ident(self.current_pos())?);
            }
            PayloadSyntax::For => {
                while self.consume_symbol(Symbol::Comma) && !self.current_is_symbol(Symbol::Pipe) {
                    stars.push(self.consume_symbol(Symbol::Asterisk));
                    names.push(self.expect_ident(self.current_pos())?);
                }
            }
            _ => {}
        }
        self.expect_symbol(Symbol::Pipe)?;
        Ok(Some(ast::Payload {
            pos,
            star,
            names,
            stars,
        }))
    }

    fn validate_token_stream(&self) -> Result<(), Error> {
        self.validate_balanced_delimiters()?;
        self.validate_rejection_cases()?;
        Ok(())
    }

    fn validate_balanced_delimiters(&self) -> Result<(), Error> {
        let mut stack: Vec<(Symbol, usize)> = Vec::new();
        for token in &self.tokens {
            match token.token {
                Token::Symbol(Symbol::LParen) => {
                    stack.push((Symbol::RParen, token.pos));
                    if stack.len() > MAX_DEPTH {
                        return Err(self.error_at(token.pos, "parser recursion depth exceeded"));
                    }
                }
                Token::Symbol(Symbol::LBracket) => {
                    stack.push((Symbol::RBracket, token.pos));
                    if stack.len() > MAX_DEPTH {
                        return Err(self.error_at(token.pos, "parser recursion depth exceeded"));
                    }
                }
                Token::Symbol(Symbol::LBrace) => {
                    stack.push((Symbol::RBrace, token.pos));
                    if stack.len() > MAX_DEPTH {
                        return Err(self.error_at(token.pos, "parser recursion depth exceeded"));
                    }
                }
                Token::Symbol(Symbol::RParen | Symbol::RBracket | Symbol::RBrace) => {
                    let actual = match token.token {
                        Token::Symbol(symbol) => symbol,
                        _ => unreachable!(),
                    };
                    if stack.pop().map(|(expected, _)| expected) != Some(actual) {
                        return Err(self.error_at(token.pos, "unbalanced delimiter"));
                    }
                }
                _ => {}
            }
        }

        if let Some((_, pos)) = stack.pop() {
            return Err(self.error_at(pos, "unclosed delimiter"));
        }
        Ok(())
    }

    fn validate_rejection_cases(&self) -> Result<(), Error> {
        let significant: Vec<&SpannedToken> = self
            .tokens
            .iter()
            .filter(|token| !matches!(token.token, Token::Comment(_, _)))
            .collect();

        for window in significant.windows(2) {
            if matches!(window[0].token, Token::Symbol(Symbol::Dot))
                && matches!(
                    window[1].token,
                    Token::Literal(LitKind::Integer | LitKind::Float, _)
                )
            {
                return Err(self.error_at(window[0].pos, "dot cannot start a numeric literal"));
            }
            if matches!(window[0].token, Token::Keyword(Keyword::If))
                && !matches!(window[1].token, Token::Symbol(Symbol::LParen))
            {
                return Err(self.error_at(window[0].pos, "if condition must be parenthesized"));
            }
        }

        for window in significant.windows(3) {
            if token_is_comparison(&window[0].token)
                && !is_delimiter(&window[1].token)
                && token_is_comparison(&window[2].token)
            {
                return Err(
                    self.error_at(window[2].pos, "comparison operators are not associative")
                );
            }
        }

        for idx in 0..significant.len() {
            if matches!(
                significant[idx].token,
                Token::Keyword(Keyword::Const | Keyword::Var)
            ) {
                if !starts_var_decl_at(&significant, idx) {
                    continue;
                }
                let mut saw_comma = false;
                let mut saw_equal = false;
                let mut depth = 0usize;
                let mut j = idx + 1;
                while j < significant.len() {
                    match significant[j].token {
                        Token::Symbol(Symbol::LParen | Symbol::LBracket | Symbol::LBrace) => {
                            depth += 1;
                        }
                        Token::Symbol(Symbol::RParen | Symbol::RBracket | Symbol::RBrace) => {
                            depth = depth.saturating_sub(1);
                        }
                        Token::Symbol(Symbol::Comma) if depth == 0 => saw_comma = true,
                        Token::Symbol(Symbol::Equal) if depth == 0 => {
                            saw_equal = true;
                            break;
                        }
                        Token::Symbol(Symbol::Semicolon) if depth == 0 => break,
                        _ => {}
                    }
                    j += 1;
                }
                if saw_comma && !saw_equal {
                    return Err(self.error_at(
                        significant[idx].pos,
                        "destructuring variable declarations require an initializer",
                    ));
                }
            }

            if idx + 3 < significant.len()
                && matches!(significant[idx].token, Token::Symbol(Symbol::LBracket))
                && matches!(significant[idx + 1].token, Token::Symbol(Symbol::Asterisk))
                && matches!(significant[idx + 2].token, Token::Literal(LitKind::Ident, ref name) if name != "c")
                && matches!(significant[idx + 3].token, Token::Symbol(Symbol::RBracket))
            {
                return Err(self.error_at(significant[idx + 2].pos, "[*x] pointer type is invalid"));
            }
        }

        Ok(())
    }

    fn current(&self) -> Option<&SpannedToken> {
        self.tokens
            .get(self.cursor..)?
            .iter()
            .find(|token| !matches!(token.token, Token::Comment(CommentKind::Line, _)))
    }

    fn advance(&mut self) {
        self.skip_comments();
        self.cursor += 1;
    }

    fn check_scan_error(&mut self) -> Result<(), Error> {
        match self.scan_error.take() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn current_pos(&self) -> usize {
        self.current().map_or(self.source_len, |token| token.pos)
    }

    fn current_is_symbol(&self, symbol: Symbol) -> bool {
        matches!(self.current().map(|token| &token.token), Some(Token::Symbol(actual)) if *actual == symbol)
    }

    fn current_is_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.current().map(|token| &token.token), Some(Token::Keyword(actual)) if *actual == keyword)
    }

    fn current_is_kind(&self, kind: TokenKind) -> bool {
        self.current()
            .is_some_and(|token| token.token.kind() == kind)
    }

    fn current_is_block_label(&self) -> bool {
        self.current_is_kind(TokenKind::Literal(LitKind::Ident))
            && self
                .peek_non_comment(1)
                .is_some_and(|token| matches!(token.token, Token::Symbol(Symbol::Colon)))
            && self.peek_non_comment(2).is_some_and(|token| {
                matches!(
                    token.token,
                    Token::Symbol(Symbol::LBrace)
                        | Token::Keyword(
                            Keyword::Inline | Keyword::While | Keyword::For | Keyword::Switch
                        )
                )
            })
    }

    fn current_is_block_expr(&self) -> bool {
        self.current_is_symbol(Symbol::LBrace)
            || (self.current_is_block_label()
                && self
                    .peek_non_comment(2)
                    .is_some_and(|token| matches!(token.token, Token::Symbol(Symbol::LBrace))))
    }

    fn current_stops_optional_expr(&self) -> bool {
        matches!(
            self.current().map(|token| &token.token),
            None | Some(Token::Keyword(
                Keyword::Catch | Keyword::Else | Keyword::Orelse
            )) | Some(Token::Symbol(
                Symbol::Semicolon
                    | Symbol::Comma
                    | Symbol::RParen
                    | Symbol::RBracket
                    | Symbol::RBrace
                    | Symbol::EqualRArrow
            ))
        )
    }

    fn consume_symbol(&mut self, symbol: Symbol) -> bool {
        if self.current_is_symbol(symbol) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn consume_keyword(&mut self, keyword: Keyword) -> bool {
        if self.current_is_keyword(keyword) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn consume_ident_text(&mut self, expected: &str) -> bool {
        match self.current().map(|token| &token.token) {
            Some(Token::Literal(LitKind::Ident, text)) if text == expected => {
                self.advance();
                true
            }
            _ => false,
        }
    }

    fn current_operator(&self) -> Option<Token> {
        let token = self.current()?.token.clone();
        (token.precedence() > 0).then_some(token)
    }

    fn current_is_prefix_op(&self) -> bool {
        matches!(
            self.current().map(|token| &token.token),
            Some(Token::Symbol(
                Symbol::ExclamationMark
                    | Symbol::Minus
                    | Symbol::Tilde
                    | Symbol::MinusPercent
                    | Symbol::Ampersand
            )) | Some(Token::Keyword(Keyword::Try))
        )
    }

    fn bump_token(&mut self) -> Option<Token> {
        let token = self.current()?.token.clone();
        self.advance();
        Some(token)
    }

    fn peek_non_comment(&self, offset: usize) -> Option<&SpannedToken> {
        let mut seen = 0usize;
        for token in self.tokens.iter().skip(self.cursor) {
            if matches!(token.token, Token::Comment(_, _)) {
                continue;
            }
            if seen == offset {
                return Some(token);
            }
            seen += 1;
        }
        None
    }

    fn expect_symbol(&mut self, symbol: Symbol) -> Result<(), Error> {
        match self.current() {
            Some(token) if token.token == Token::Symbol(symbol) => {
                self.advance();
                Ok(())
            }
            Some(token) => {
                Err(self.unexpected(vec![TokenKind::Symbol(symbol)], Some(token.token.clone())))
            }
            None => Err(self.unexpected(vec![TokenKind::Symbol(symbol)], None)),
        }
    }

    fn expect_keyword(&mut self, keyword: Keyword) -> Result<(), Error> {
        match self.current() {
            Some(token) if token.token == Token::Keyword(keyword) => {
                self.advance();
                Ok(())
            }
            Some(token) => {
                Err(self.unexpected(vec![TokenKind::Keyword(keyword)], Some(token.token.clone())))
            }
            None => Err(self.unexpected(vec![TokenKind::Keyword(keyword)], None)),
        }
    }

    fn expect_ident(&mut self, pos: usize) -> Result<ast::Ident, Error> {
        let Some(token) = self.current().cloned() else {
            return Err(self.unexpected(vec![TokenKind::Literal(LitKind::Ident)], None));
        };
        match token.token {
            Token::Literal(LitKind::Ident, name) => {
                self.advance();
                Ok(ast::Ident { pos, name })
            }
            actual => Err(self.unexpected(vec![TokenKind::Literal(LitKind::Ident)], Some(actual))),
        }
    }

    fn is_eof(&self) -> bool {
        self.cursor >= self.tokens.len()
    }

    fn with_depth<T>(
        &mut self,
        parse: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        if self.depth >= MAX_DEPTH {
            return Err(self.error_at(self.current_pos(), "parser recursion depth exceeded"));
        }
        self.depth += 1;
        let result = parse(self);
        self.depth -= 1;
        result
    }

    pub fn preback(&self) -> ParserMark {
        ParserMark {
            cursor: self.cursor,
            depth: self.depth,
            suppress_curly_suffix: self.suppress_curly_suffix,
            comment_count: self.comments.len(),
        }
    }

    pub fn goback(&mut self, mark: ParserMark) {
        self.cursor = mark.cursor;
        self.depth = mark.depth;
        self.suppress_curly_suffix = mark.suppress_curly_suffix;
        self.comments.truncate(mark.comment_count);
    }

    fn tokens_since(&self, mark: ParserMark) -> Vec<Token> {
        self.tokens[mark.cursor..self.cursor]
            .iter()
            .map(|token| token.token.clone())
            .collect()
    }

    fn unexpected(&self, expect: Vec<TokenKind>, actual: Option<Token>) -> Error {
        Error::UnexpectedToken {
            path: self.path.clone(),
            offset: self.current_pos(),
            location: self.location_for_current(),
            expect,
            actual,
        }
    }

    fn error_at(&self, pos: usize, reason: &str) -> Error {
        Error::Syntax {
            path: self.path.clone(),
            offset: pos,
            location: self.line_col(pos),
            reason: reason.to_owned(),
        }
    }

    fn location_for_current(&self) -> (usize, usize) {
        self.line_col(self.current_pos())
    }

    fn line_col(&self, pos: usize) -> (usize, usize) {
        let line_idx = self
            .line_info
            .partition_point(|line_start| *line_start <= pos)
            .saturating_sub(1);
        let line_start = self.line_info.get(line_idx).copied().unwrap_or(0);
        (line_idx + 1, pos.saturating_sub(line_start) + 1)
    }
}

fn token_is_comparison(token: &Token) -> bool {
    matches!(token, Token::Symbol(symbol) if symbol.is_comparison())
}

fn is_delimiter(token: &Token) -> bool {
    matches!(
        token,
        Token::Symbol(
            Symbol::LParen
                | Symbol::RParen
                | Symbol::LBracket
                | Symbol::RBracket
                | Symbol::LBrace
                | Symbol::RBrace
                | Symbol::Comma
                | Symbol::Semicolon
        )
    )
}

fn starts_var_decl_at(tokens: &[&SpannedToken], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }

    matches!(
        tokens[idx - 1].token,
        Token::Symbol(Symbol::LBrace | Symbol::Semicolon)
            | Token::Keyword(
                Keyword::Pub
                    | Keyword::Export
                    | Keyword::Extern
                    | Keyword::Threadlocal
                    | Keyword::Comptime
            )
    )
}

#[cfg(test)]
mod tests {
    use crate::parse_source;

    use super::*;

    #[test]
    fn parses_top_level_members_and_docs() {
        let file = parse_source(
            r#"
//! module docs
/// value docs
pub const answer: u32 = 42;
pub fn main() void {}
test "basic" {}
"#,
        )
        .unwrap();

        assert_eq!(file.container_docs.len(), 1);
        assert_eq!(file.comments.len(), 2);
        assert_eq!(file.members.len(), 3);
        assert!(matches!(file.members[0], ast::ContainerMember::Var(_)));
        assert!(matches!(file.members[1], ast::ContainerMember::Fn(_)));
        assert!(matches!(file.members[2], ast::ContainerMember::Test(_)));
    }

    #[test]
    fn nested_container_docs_belong_to_their_container() {
        let file = parse_source(
            r#"//! file
const A = struct {
    //! outer
    const B = struct {
        //! inner
        value: u8,
    };
};"#,
        )
        .unwrap();
        let ast::ContainerMember::Var(a) = &file.members[0] else {
            panic!("expected A declaration");
        };
        let Some(ast::Expression::Container(outer)) = &a.init else {
            panic!("expected outer container");
        };
        let ast::ContainerMember::Var(b) = &outer.members[0] else {
            panic!("expected B declaration");
        };
        let Some(ast::Expression::Container(inner)) = &b.init else {
            panic!("expected inner container");
        };
        assert_eq!(file.container_docs[0].text, "//! file");
        assert_eq!(outer.container_docs[0].text, "//! outer");
        assert_eq!(inner.container_docs[0].text, "//! inner");
        assert_eq!(file.comments.len(), 3);
    }

    #[test]
    fn modified_declarations_start_at_the_first_modifier() {
        let source = r#"/// docs
pub inline fn f() void {}
export var a: u8 = 1;
extern "c" var b: c_int;"#;
        let file = parse_source(source).unwrap();
        let [ast::ContainerMember::Fn(fun), ast::ContainerMember::Var(exported), ast::ContainerMember::Var(external)] =
            file.members.as_slice()
        else {
            panic!("expected a function and two variables");
        };

        assert_eq!(fun.pos, source.find("pub").unwrap());
        assert!(matches!(
            fun.tokens.as_slice(),
            [
                Token::Keyword(Keyword::Pub),
                Token::Keyword(Keyword::Inline),
                Token::Keyword(Keyword::Fn),
                ..
            ]
        ));
        assert_eq!(exported.pos, source.find("export").unwrap());
        assert!(matches!(
            exported.tokens.as_slice(),
            [
                Token::Keyword(Keyword::Export),
                Token::Keyword(Keyword::Var),
                ..
            ]
        ));
        assert_eq!(external.pos, source.find("extern").unwrap());
        assert!(matches!(
            external.tokens.as_slice(),
            [Token::Keyword(Keyword::Extern), Token::Literal(LitKind::String, abi), Token::Keyword(Keyword::Var), ..]
                if abi == "\"c\""
        ));
    }

    #[test]
    fn parses_expression_precedence_and_rejects_compare_chain() {
        let mut parser = Parser::from("a + b * c");
        let expr = parser.expression().unwrap();
        let ast::Expression::Binary(root) = expr else {
            panic!("expected binary expression");
        };
        assert_eq!(root.op, Token::Symbol(Symbol::Plus));

        let mut parser = Parser::from("a == b == c");
        assert!(parser.expression().is_err());
        assert!(parse_source("pub const bad = a == b == c;").is_err());
    }

    #[test]
    fn rejects_required_negative_cases() {
        for source in [
            "pub const bad = .5;",
            "pub const bad = if x {};",
            "pub const a, b;",
            "pub const bad: [*x]u8 = undefined;",
            "pub const bad = struct { fn f() void {} , };",
            "pub const bad = @import;",
        ] {
            assert!(parse_source(source).is_err(), "{source}");
        }
    }

    #[test]
    fn requires_list_commas_and_contiguous_container_fields() {
        let invalid = [
            "a: u8 b: u8",
            "a: u8, const middle = 1; b: u8,",
            "const S = struct { a: u8 b: u8 };",
            "const S = struct { a: u8, const middle = 1; b: u8, };",
            "const E = error{ A B };",
            "const S = switch (tag) { 1 => 1 else => 2 };",
        ];
        let accepted: Vec<_> = invalid
            .into_iter()
            .filter(|source| parse_source(source).is_ok())
            .collect();
        assert!(accepted.is_empty(), "accepted invalid syntax: {accepted:?}");

        for source in [
            "a: u8",
            "const first = 1; a: u8, const last = 2;",
            "const S = struct { a: u8, const last = 2; };",
            "const S = struct { a: u8 };",
            "const E = error{ A, B };",
            "const S = switch (tag) { 1 => 1, else => 2 };",
        ] {
            parse_source(source).unwrap_or_else(|err| panic!("{source}: {err}"));
        }
    }

    #[test]
    fn rejects_syntax_outside_each_constructs_grammar() {
        let invalid = [
            "fn f() void { for () |x| {} }",
            "fn f() void { if (x) |a, b| {} }",
            "fn f() void { if (x) |a,| {} }",
            "fn f() void { if (x) {} else |*e| {} }",
            "const x = value catch |*e| 0;",
            "fn f() void { errdefer |*e| {} }",
            "const x = switch (value) { inline else => |a, *tag| a };",
            "const x = switch (value) { inline else => |a, tag,| a };",
            "const T = opaque(u8) {};",
            "const T = []align(1:2:3) u8;",
            "const x = .{ .a = 1, 2 };",
            "const x = .{ 1, .a = 2 };",
        ];
        let accepted: Vec<_> = invalid
            .into_iter()
            .filter(|source| parse_source(source).is_ok())
            .collect();
        assert!(accepted.is_empty(), "accepted invalid syntax: {accepted:?}");

        for source in [
            "fn f() void { for (a, b) |*x, *y,| {} }",
            "fn f() void { if (x) |*a| {} else |e| {} }",
            "fn f() void { errdefer |e| {} }",
            "const x = value catch |e| 0;",
            "const x = switch (value) { inline else => |*a, tag| a };",
            "const T = opaque {};",
            "const T = []align(1) u8;",
            "const T = *align(1:2:3) u8;",
            "const x = .{ .a = 1, .b = 2 };",
            "const x = .{ 1, 2 };",
        ] {
            parse_source(source).unwrap_or_else(|err| panic!("{source}: {err}"));
        }
    }

    #[test]
    fn rejects_documentation_comments_without_a_valid_target() {
        let invalid = [
            "const x = 1;\n/// Detached",
            "const x = 1;\n//! Late\nconst y = 2;",
            "const T = struct { const x = 1;\n//! Late\n};",
            "fn f() void {\n/// Local\nconst x = 1; }",
            "/// Test\ntest {}",
            "/// Comptime\ncomptime {}",
            "fn f(\n/// Missing\n) void {}",
            "const E = error{ A,\n/// Missing\n};",
        ];
        let accepted: Vec<_> = invalid
            .into_iter()
            .filter(|source| parse_source(source).is_ok())
            .collect();
        assert!(accepted.is_empty(), "accepted invalid syntax: {accepted:?}");

        let file = parse_source(
            "// License\n//! Module\n/// Function\nfn f(\n/// Parameter\na: u8,\n) void {}\n\
             const T = struct {\n//! Container\n/// Field\nx: u8, };\n\
             const E = error{\n/// Error member\nA, };",
        )
        .unwrap();
        assert_eq!(file.container_docs.len(), 1);
        assert_eq!(file.comments.len(), 7);
    }

    #[test]
    fn reports_eof_at_the_end_of_source() {
        for (source, line, column) in [
            ("const x =", 1, 10),
            ("const x =\n", 2, 1),
            ("const x =\n  ", 2, 3),
            ("const x = \"é\"  ", 1, 17),
        ] {
            let error = parse_source(source).unwrap_err().to_string();
            assert!(
                error.starts_with(&format!("<source>:{line}:{column} ")),
                "{source:?}: {error}"
            );
        }
    }

    #[test]
    fn expression_depth_guard_returns_error() {
        let source = "(".repeat(1100) + "x" + &")".repeat(1100);
        let mut parser = Parser::from(source);
        assert!(parser.expression().is_err());
    }

    #[test]
    fn expression_depth_guard_counts_prefix_operands() {
        for source in ["!flag", "?u8", "[]u8", "*u8", "anyframe->u8", "E!u8"] {
            let mut parser = Parser::from(source);
            parser.depth = MAX_DEPTH - 1;
            assert!(
                matches!(
                    parser.expression(),
                    Err(Error::Syntax { reason, .. }) if reason == "parser recursion depth exceeded"
                ),
                "{source}"
            );
            assert_eq!(parser.depth, MAX_DEPTH - 1);
        }
    }

    #[test]
    fn statement_depth_guard_counts_nested_statements() {
        let mut parser = Parser::from("if (a) {} else if (b) {}");
        parser.depth = MAX_DEPTH - 2;
        let mark = parser.preback();
        assert!(matches!(
            parser.parse_statement(),
            Err(Error::Syntax { reason, .. }) if reason == "parser recursion depth exceeded"
        ));
        assert_eq!(parser.preback(), mark);
    }

    #[test]
    fn expression_depth_guard_restores_depth_on_parse_error() {
        let mut parser = Parser::from("x +");
        assert!(parser.expression().is_err());
        assert_eq!(parser.depth, 0);
    }

    #[test]
    fn parses_structured_function_decl_and_block_statement() {
        let file = parse_source(
            r#"
pub fn add(a: u32, comptime b: anytype) u32 {
    const c: u32 = a + 1;
    return c;
}
"#,
        )
        .unwrap();

        let ast::ContainerMember::Fn(func) = &file.members[0] else {
            panic!("expected fn member");
        };
        assert_eq!(func.visibility, Some(Keyword::Pub));
        assert_eq!(func.name.as_ref().unwrap().name, "add");
        assert_eq!(func.params.len(), 2);
        assert!(func.ret.is_some());
        let body = func.body.as_ref().unwrap();
        assert_eq!(body.statements.len(), 2);
        assert!(matches!(body.statements[0], ast::Statement::Var(_)));
        assert!(matches!(body.statements[1], ast::Statement::Expr(_)));
    }

    #[test]
    fn parses_prefix_type_shapes() {
        let mut parser = Parser::from("*const ?[]align(8) u8");
        let expr = parser.expression().unwrap();
        let ast::Expression::Pointer(ptr) = expr else {
            panic!("expected pointer type");
        };
        assert!(matches!(ptr.kind, ast::PointerKind::One));
        assert!(matches!(ptr.modifiers[0], ast::PointerModifier::Const));
        assert!(matches!(*ptr.child, ast::Expression::Optional(_)));

        let mut parser = Parser::from("[*:0]const u8");
        let expr = parser.expression().unwrap();
        let ast::Expression::Pointer(ptr) = expr else {
            panic!("expected sentinel pointer type");
        };
        assert!(matches!(ptr.kind, ast::PointerKind::Sentinel(_)));
    }

    #[test]
    fn double_asterisk_builds_two_single_item_pointers() {
        let ast::Expression::Pointer(outer) = Parser::from("**const u8").expression().unwrap()
        else {
            panic!("expected outer pointer");
        };
        assert!(matches!(outer.kind, ast::PointerKind::One));
        assert!(outer.modifiers.is_empty());
        assert_eq!(outer.tokens[0], Token::Symbol(Symbol::Asterisk2));
        let ast::Expression::Pointer(inner) = outer.child.as_ref() else {
            panic!("expected inner pointer");
        };
        assert!(matches!(inner.kind, ast::PointerKind::One));
        assert_eq!(inner.pos, outer.pos + 1);
        assert_eq!(inner.modifiers, [ast::PointerModifier::Const]);
        assert_eq!(inner.tokens[0], Token::Symbol(Symbol::Asterisk));
    }

    #[test]
    fn array_initializers_use_the_complete_array_type() {
        for (source, slice_child) in [
            ("[_]u8{ 1, 2, 3 }", false),
            ("[_][]const u8{ \"a\" }", true),
        ] {
            let ast::Expression::InitList(init) = Parser::from(source).expression().unwrap() else {
                panic!("expected initializer for {source}");
            };
            let ast::Expression::ArrayType(array) = init.ty.as_ref() else {
                panic!("expected array type for {source}");
            };
            let child = if slice_child {
                let ast::Expression::SliceType(slice) = array.child.as_ref() else {
                    panic!("expected slice element type for {source}");
                };
                slice.child.as_ref()
            } else {
                array.child.as_ref()
            };
            assert!(matches!(child, ast::Expression::Ident(ident) if ident.name == "u8"));
        }
    }

    #[test]
    fn parses_initializer_arguments_inside_a_return_type() {
        parse_source("fn f() std.meta.Tuple(&[_]type{ u8, u8 }) {}").unwrap();
    }

    #[test]
    fn parses_typed_initializer_in_a_container_field_default() {
        let file = parse_source("var o: struct { c: Cfg = Cfg{} } = .{};").unwrap();
        let ast::ContainerMember::Var(var) = &file.members[0] else {
            panic!("expected variable declaration");
        };
        let Some(ast::Expression::Container(container)) = var.ty() else {
            panic!("expected container type");
        };
        let [ast::ContainerMember::Field(field)] = container.members.as_slice() else {
            panic!("expected one container field");
        };
        assert!(matches!(field.default, Some(ast::Expression::InitList(_))));
    }

    #[test]
    fn parses_control_and_container_expressions() {
        let mut parser = Parser::from("if (a) b else c");
        assert!(matches!(
            parser.expression().unwrap(),
            ast::Expression::If(_)
        ));

        let mut parser = Parser::from("for (items, 0..) |item, i| item");
        let ast::Expression::For(for_expr) = parser.expression().unwrap() else {
            panic!("expected for expression");
        };
        assert_eq!(for_expr.items.len(), 2);
        assert_eq!(for_expr.payload.unwrap().names.len(), 2);

        let mut parser = Parser::from("union(enum) { a: u8, b: u16 }");
        let ast::Expression::Container(container) = parser.expression().unwrap() else {
            panic!("expected container expression");
        };
        assert!(matches!(
            container.arg,
            Some(ast::ContainerArg::Enum { tag_type: None, .. })
        ));
    }

    #[test]
    fn parses_labeled_control_payloads_and_assign_exprs() {
        let mut parser = Parser::from("blk: while (i < n) : (i += 1) continue :blk i");
        let ast::Expression::While(while_expr) = parser.expression().unwrap() else {
            panic!("expected while expression");
        };
        assert_eq!(while_expr.label.as_ref().unwrap().name, "blk");
        assert!(matches!(
            while_expr.continue_expr.as_deref(),
            Some(ast::Expression::Assign(_))
        ));
        let ast::Expression::Continue(continue_expr) = while_expr.body.as_deref().unwrap() else {
            panic!("expected continue expression");
        };
        assert_eq!(continue_expr.label.as_ref().unwrap().name, "blk");
        assert!(continue_expr.value.is_some());

        let mut parser = Parser::from("for (items, other) |*item, *o,| item");
        let ast::Expression::For(for_expr) = parser.expression().unwrap() else {
            panic!("expected for expression");
        };
        let payload = for_expr.payload.unwrap();
        assert_eq!(payload.names.len(), 2);
        assert_eq!(payload.stars, vec![true, true]);

        let mut parser = Parser::from("switch (x) { else => y += 1 }");
        let ast::Expression::Switch(switch_expr) = parser.expression().unwrap() else {
            panic!("expected switch expression");
        };
        assert!(matches!(
            &switch_expr.prongs[0].body,
            ast::Expression::Assign(_)
        ));
    }

    #[test]
    fn rejects_extra_statement_semicolons() {
        for statement in [
            "while (a) {};",
            "defer f();;",
            "comptime {};",
            "switch (a) { else => {} };",
            "if (a) {};",
        ] {
            assert!(
                parse_source(format!("fn f() void {{ {statement} }}")).is_err(),
                "{statement}"
            );
        }
    }

    #[test]
    fn defer_accepts_blocks_and_terminated_expressions() {
        for statement in [
            "defer { f(); }",
            "defer f();",
            "defer if (a) {};",
            "defer x, y = values;",
        ] {
            Parser::from(statement).parse_statement().unwrap();
        }
        for statement in [
            "defer const x = 1;",
            "defer defer f();",
            "defer if (a) {}",
            "defer x, var y = values;",
        ] {
            assert!(
                Parser::from(statement).parse_statement().is_err(),
                "{statement}"
            );
        }
    }

    #[test]
    fn rejects_semicolons_before_control_statement_else() {
        for source in [
            "if (a) b(); else c();",
            "while (a) b(); else c();",
            "for (items) |_| b(); else c();",
        ] {
            let mut parser = Parser::from(source);
            assert!(parser.parse_statement().is_err(), "{source}");
        }
    }

    #[test]
    fn preserves_line_comments_before_postfix_operators() {
        let file = parse_source("const x = map.get(k) // lookup\n.?; // trailing").unwrap();
        let ast::ContainerMember::Var(var) = &file.members[0] else {
            panic!("expected variable declaration");
        };
        assert!(matches!(var.init, Some(ast::Expression::Unwrap(_))));
        assert_eq!(file.comments.len(), 2);
        assert!(var
            .tokens
            .iter()
            .any(|token| matches!(token, Token::Comment(CommentKind::Line, _))));
    }

    #[test]
    fn line_comments_keep_else_if_as_a_statement() {
        let mut parser = Parser::from("if (a) {} else // branch\nif (b) x = 1;");
        let ast::Statement::If(outer) = parser.parse_statement().unwrap() else {
            panic!("expected if statement");
        };
        assert!(matches!(
            outer.else_branch.as_deref(),
            Some(ast::Statement::If(_))
        ));
    }

    #[test]
    fn accepts_line_comments_between_asm_sections() {
        let mut parser =
            Parser::from("asm (\"nop\" : [out] \"=r\" (-> usize) // output\n : [in] \"r\" (x))");
        let ast::Expression::Asm(expr) = parser.expression().unwrap() else {
            panic!("expected asm expression");
        };
        assert_eq!(expr.outputs.len(), 1);
        assert_eq!(expr.inputs.len(), 1);
    }

    #[test]
    fn parses_statement_edge_cases_from_stdlib() {
        parse_source(
            r#"
pub fn f() void {
    if (cond) {
        foo();
    }
    // comment before else
    else {
        bar();
    }

    for (items) |item| (if (item) break) else unreachable;
    defer if (flag) {
        cleanup();
    };
    comptime for (items) |item| {
        _ = item;
    };
    _, const packet_end = findLastPacket();
    const x: [(S{ .a = 1 }).n]u8 = undefined;
}
"#,
        )
        .unwrap();
    }

    #[test]
    fn multiline_strings_preserve_intervening_line_comments() {
        let source = r#"const text =
    \\first
    // interposed
    \\second
;"#;
        let file = parse_source(source).unwrap();
        let ast::ContainerMember::Var(decl) = &file.members[0] else {
            panic!("expected variable declaration");
        };
        let Some(ast::Expression::MultilineStr(string)) = &decl.init else {
            panic!("expected multiline string");
        };
        assert_eq!(string.text, "\\\\first\n\\\\second");
        assert_eq!(file.comments.len(), 1);
        assert_eq!(file.comments[0].text, "// interposed");
        assert_eq!(file.comments[0].pos, source.find("// interposed").unwrap());
    }

    #[test]
    fn parses_asm_expression_shape() {
        let mut parser = Parser::from(
            r#"asm volatile ("syscall"
                : [ret] "={rax}" (-> usize)
                : [number] "{rax}" (number)
                : "rcx"
            )"#,
        );
        let ast::Expression::Asm(asm_expr) = parser.expression().unwrap() else {
            panic!("expected asm expression");
        };
        assert!(asm_expr.volatile);
        assert_eq!(asm_expr.outputs.len(), 1);
        assert_eq!(asm_expr.inputs.len(), 1);
        assert!(asm_expr.clobbers.is_some());
    }

    /// Size of the AST built for `source`, measured by its debug output.
    fn ast_size(source: &str) -> usize {
        format!("{:?}", parse_source(source).unwrap().members).len()
    }

    #[test]
    fn nested_anonymous_initializers_do_not_grow_exponentially() {
        // Each level used to store its entries twice, doubling the AST per
        // level.
        let nested =
            |depth: usize| format!("const x = {}1{};", ".{".repeat(depth), "}".repeat(depth));
        assert!(ast_size(&nested(16)) < 3 * ast_size(&nested(8)));
    }

    #[test]
    fn nested_declaration_types_do_not_grow_exponentially() {
        // Each declaration used to store its type twice, doubling the AST
        // per level of declarations nested inside types. (Each declaration's
        // `tokens` still copies the tokens nested in it, so the size grows
        // quadratically: about 4x when the depth doubles, against 64x before.)
        let nested = |depth: usize| {
            format!(
                "fn f() void {{{}break :blk 1;{}}}",
                "var x: struct { a: u8 = blk: {".repeat(depth),
                "}} = .{};".repeat(depth)
            )
        };
        assert!(ast_size(&nested(12)) < 8 * ast_size(&nested(6)));
    }

    #[test]
    fn anon_init_fields_are_the_entry_values() {
        for source in [".{ .a = x, .b = y }", ".{ x, y }"] {
            let ast::Expression::AnonInit(init) = Parser::from(source).expression().unwrap() else {
                panic!("expected anonymous initializer in {source}");
            };
            let names: Vec<_> = init
                .fields()
                .map(|field| match field {
                    ast::Expression::Ident(ident) => ident.name.as_str(),
                    other => panic!("unexpected field {other:?}"),
                })
                .collect();
            assert_eq!(names, ["x", "y"], "{source}");
        }
    }

    #[test]
    fn retains_modifiers_on_later_destructuring_bindings() {
        let file = parse_source(
            r#"fn f() void {
    var x: u8 align(2), var y: u32 align(8) addrspace(.generic) linksection(".data") = .{ 1, 2 };
    x, var z: u32 align(16) = .{ 3, 4 };
}"#,
        )
        .unwrap();
        let ast::ContainerMember::Fn(function) = &file.members[0] else {
            panic!("expected function");
        };
        let [ast::Statement::Var(bindings), ast::Statement::Var(mixed)] =
            function.body.as_ref().unwrap().statements.as_slice()
        else {
            panic!("expected two destructuring declarations");
        };
        assert!(matches!(&bindings.align, Some(ast::Expression::BasicLit(lit)) if lit.text == "2"));
        assert!(matches!(
            &bindings.lhs[0],
            ast::VarProtoOrExpr::VarProto { align: None, .. }
        ));
        let ast::VarProtoOrExpr::VarProto {
            align: Some(align),
            addrspace: Some(addrspace),
            linksection: Some(linksection),
            ..
        } = &bindings.lhs[1]
        else {
            panic!("expected modifiers on second binding");
        };
        assert!(matches!(align.as_ref(), ast::Expression::BasicLit(lit) if lit.text == "8"));
        assert!(
            matches!(addrspace.as_ref(), ast::Expression::EnumLiteral(lit) if lit.name.name == "generic")
        );
        assert!(
            matches!(linksection.as_ref(), ast::Expression::BasicLit(lit) if lit.text == "\".data\"")
        );

        let ast::VarProtoOrExpr::VarProto {
            align: Some(align), ..
        } = &mixed.lhs[1]
        else {
            panic!("expected alignment on binding after assignment target");
        };
        assert!(matches!(align.as_ref(), ast::Expression::BasicLit(lit) if lit.text == "16"));
        assert!(mixed.align.is_none());
    }

    #[test]
    fn var_decl_ty_uses_the_first_lhs_entry() {
        let file = parse_source(
            "const a: u8 = 1;
             const b = 2;
             fn f() void {
                 var c: u16, const d: u32 = .{ 3, 4 };
                 c, const e: u64 = .{ 5, 6 };
                 _ = d;
                 _ = e;
             }",
        )
        .unwrap();
        let ty_name = |var: &ast::VarDecl| match var.ty() {
            Some(ast::Expression::Ident(ident)) => Some(ident.name.clone()),
            None => None,
            Some(other) => panic!("unexpected type {other:?}"),
        };
        let ast::ContainerMember::Var(a) = &file.members[0] else {
            panic!("expected var");
        };
        let ast::ContainerMember::Var(b) = &file.members[1] else {
            panic!("expected var");
        };
        assert_eq!(ty_name(a).as_deref(), Some("u8"));
        assert_eq!(ty_name(b), None);
        let ast::ContainerMember::Fn(f) = &file.members[2] else {
            panic!("expected fn");
        };
        let [ast::Statement::Var(c), ast::Statement::Var(assignment_first), ..] =
            f.body.as_ref().unwrap().statements.as_slice()
        else {
            panic!("expected two destructuring declarations");
        };
        assert_eq!(ty_name(c).as_deref(), Some("u16"));
        assert!(assignment_first.ty().is_none());
        assert!(matches!(
            &assignment_first.lhs[1],
            ast::VarProtoOrExpr::VarProto { ty: Some(ast::Expression::Ident(ty)), .. }
                if ty.name == "u64"
        ));
    }
}
