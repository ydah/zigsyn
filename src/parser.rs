use std::path::{Path, PathBuf};
use std::rc::Rc;

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
}

#[derive(Debug)]
pub struct Parser {
    tokens: Vec<SpannedToken>,
    cursor: usize,
    path: PathBuf,
    line_info: Vec<usize>,
    comments: Vec<Rc<ast::Comment>>,
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

impl Parser {
    pub fn from<S: AsRef<str>>(source: S) -> Self {
        let scanner = Scanner::from(source);
        Self::from_scanner(scanner)
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let scanner = Scanner::from_file(path)?;
        Ok(Self::from_scanner(scanner))
    }

    fn from_scanner(scanner: Scanner) -> Self {
        let path = scanner.path().to_path_buf();
        let line_info = scanner.lines().to_vec();
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
            comments: Vec::new(),
            depth: 0,
            suppress_curly_suffix: 0,
            scan_error,
        }
    }

    pub fn parse_file(mut self) -> anyhow::Result<ast::File> {
        self.check_scan_error()?;
        self.validate_token_stream()?;

        let mut members = Vec::new();
        let mut container_docs = Vec::new();
        while !self.is_eof() {
            let docs = self.collect_comments(&mut container_docs);
            if self.is_eof() {
                break;
            }
            members.push(self.parse_container_member(docs)?);
        }

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
        self.collect_comments(&mut Vec::new());
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
            if self.consume_symbol(Symbol::Semicolon) {
                let tokens = self.tokens_since(mark);
                return Ok(ast::Statement::Expr(Box::new(ast::ExprStmt {
                    pos,
                    expr: ast::Expression::Switch(expr),
                    semicolon: true,
                    tokens,
                })));
            }
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

        self.parse_expr_or_assign_statement()
    }

    fn parse_prefixed_statement(&mut self, keyword: Keyword) -> Result<ast::Statement, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        self.expect_keyword(keyword)?;
        let body = Box::new(self.try_parse_statement()?);
        self.consume_symbol(Symbol::Semicolon);
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
            self.parse_optional_payload()?
        } else {
            None
        };
        let body = Box::new(self.try_parse_statement()?);
        self.consume_symbol(Symbol::Semicolon);
        let tokens = self.tokens_since(mark);
        Ok(ast::Statement::Defer(Box::new(ast::DeferStmt {
            pos,
            err,
            payload,
            body,
            tokens,
        })))
    }

    fn parse_if_statement(&mut self) -> Result<ast::Statement, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        let (condition, payload) = self.parse_if_prefix()?;
        let (then_branch, then_semicolon) = self.parse_control_body_statement()?;
        self.skip_comments();
        let (else_payload, else_branch) = if self.consume_keyword(Keyword::Else) {
            let payload = self.parse_optional_payload()?;
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
        if self.current_is_symbol(Symbol::LBrace)
            || (self.current_is_block_label()
                && self
                    .peek_non_comment(2)
                    .is_some_and(|token| matches!(token.token, Token::Symbol(Symbol::LBrace))))
        {
            return Ok((self.try_parse_statement()?, false));
        }

        let mark = self.preback();
        let pos = self.current_pos();
        let expr = self.parse_assign_expr()?;
        let semicolon = self.consume_symbol(Symbol::Semicolon);
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
                ast::LoopKind::While { .. } => self.parse_optional_payload()?,
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
        self.consume_symbol(Symbol::Semicolon);
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

    fn parse_expr_or_assign_statement(&mut self) -> Result<ast::Statement, Error> {
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
                    let proto = self.parse_local_var_proto()?;
                    if !has_var_proto {
                        first_keyword = proto.keyword;
                    }
                    has_var_proto = true;
                    var_lhs.push(ast::VarProtoOrExpr::VarProto {
                        pos: proto.name.pos,
                        keyword: proto.keyword,
                        name: proto.name,
                        ty: proto.ty,
                    });
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

        let semicolon = self.consume_symbol(Symbol::Semicolon);
        if !semicolon && !statement_expr_can_omit_semicolon(&lhs) {
            return Err(self.unexpected(
                vec![TokenKind::Symbol(Symbol::Semicolon)],
                self.current().map(|token| token.token.clone()),
            ));
        }
        let tokens = self.tokens_since(mark);
        Ok(ast::Statement::Expr(Box::new(ast::ExprStmt {
            pos,
            expr: lhs,
            semicolon,
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
            name: first.name.clone(),
            ty: first.ty,
        }];
        let mut destructuring = false;
        while self.consume_symbol(Symbol::Comma) {
            destructuring = true;
            if self.current_is_keyword(Keyword::Const) || self.current_is_keyword(Keyword::Var) {
                let proto = self.parse_local_var_proto()?;
                lhs.push(ast::VarProtoOrExpr::VarProto {
                    pos: proto.name.pos,
                    keyword: proto.keyword,
                    name: proto.name,
                    ty: proto.ty,
                });
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
        docs: Vec<Rc<ast::Comment>>,
    ) -> Result<ast::ContainerMember, Error> {
        if self.current_is_symbol(Symbol::Comma) || self.current_is_symbol(Symbol::Semicolon) {
            return Err(self.error_at(self.current_pos(), "unexpected container member separator"));
        }
        self.try_parse_container_member(docs)
    }

    fn try_parse_container_member(
        &mut self,
        docs: Vec<Rc<ast::Comment>>,
    ) -> Result<ast::ContainerMember, Error> {
        if self.current_is_keyword(Keyword::Test) {
            return self.parse_test_decl(docs);
        }

        if self.current_is_keyword(Keyword::Comptime)
            && self
                .peek_non_comment(1)
                .is_some_and(|token| matches!(token.token, Token::Symbol(Symbol::LBrace)))
        {
            return self.parse_comptime_decl(docs);
        }

        let visibility = self.consume_keyword(Keyword::Pub).then_some(Keyword::Pub);
        let modifier = self.parse_decl_modifier()?;
        if self.current_is_keyword(Keyword::Fn) {
            return self.parse_fn_decl(docs, visibility, modifier);
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
            if matches!(
                modifier,
                Some(ast::FnModifier::Inline | ast::FnModifier::NoInline)
            ) {
                return Err(self.error_at(self.current_pos(), "inline modifier requires fn"));
            }
            return self.parse_global_var_decl(docs, visibility, modifier_to_var(modifier)?);
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
        docs: Vec<Rc<ast::Comment>>,
    ) -> Result<ast::ContainerMember, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
        self.expect_keyword(Keyword::Test)?;
        let name = match self.current().cloned().map(|token| token.token) {
            Some(Token::Literal(LitKind::String, text)) => {
                let pos = self.current_pos();
                self.cursor += 1;
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
        docs: Vec<Rc<ast::Comment>>,
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
                    self.cursor += 1;
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
        docs: Vec<Rc<ast::Comment>>,
        visibility: Option<Keyword>,
        modifier: Option<ast::FnModifier>,
    ) -> Result<ast::ContainerMember, Error> {
        let mark = self.preback();
        let mut decl = self.parse_fn_proto(docs, visibility, modifier)?;
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
        docs: Vec<Rc<ast::Comment>>,
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
            let docs = self.collect_comments(&mut Vec::new());
            if self.current_is_symbol(Symbol::RParen) {
                break;
            }
            let pos = self.current_pos();
            if self.consume_symbol(Symbol::Dot3) {
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

    fn parse_global_var_decl(
        &mut self,
        docs: Vec<Rc<ast::Comment>>,
        visibility: Option<Keyword>,
        extern_export: Option<ast::VarModifier>,
    ) -> Result<ast::ContainerMember, Error> {
        let decl = self.parse_var_decl(docs, visibility, extern_export, true)?;
        Ok(ast::ContainerMember::Var(Box::new(decl)))
    }

    /// VarDeclProto <- (KEYWORD_const / KEYWORD_var) IDENTIFIER (COLON TypeExpr)? ByteAlign? AddrSpace? LinkSection?
    fn parse_var_decl(
        &mut self,
        docs: Vec<Rc<ast::Comment>>,
        visibility: Option<Keyword>,
        extern_export: Option<ast::VarModifier>,
        expect_semicolon: bool,
    ) -> Result<ast::VarDecl, Error> {
        let mark = self.preback();
        let pos = self.current_pos();
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
        if expect_semicolon {
            self.expect_symbol(Symbol::Semicolon)?;
        }
        let tokens = self.tokens_since(mark);
        let lhs = vec![ast::VarProtoOrExpr::VarProto {
            pos,
            keyword,
            name,
            ty,
        }];
        Ok(ast::VarDecl {
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
        })
    }

    fn parse_container_field(
        &mut self,
        docs: Vec<Rc<ast::Comment>>,
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
            Some(self.parse_expr_bp(0)?)
        } else {
            None
        };
        self.consume_symbol(Symbol::Comma);
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

    fn collect_comments(
        &mut self,
        container_docs: &mut Vec<Rc<ast::Comment>>,
    ) -> Vec<Rc<ast::Comment>> {
        let mut docs = Vec::new();
        loop {
            let Some(current) = self.current() else {
                return docs;
            };
            let Token::Comment(kind, text) = &current.token else {
                return docs;
            };
            let comment = Rc::new(ast::Comment {
                pos: current.pos,
                kind: *kind,
                text: text.clone(),
            });
            if *kind == CommentKind::Doc {
                docs.push(Rc::clone(&comment));
            } else if *kind == CommentKind::ContainerDoc {
                container_docs.push(Rc::clone(&comment));
            }
            self.comments.push(comment);
            self.cursor += 1;
        }
    }

    fn skip_comments(&mut self) {
        let mut container_docs = Vec::new();
        self.collect_comments(&mut container_docs);
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
        self.enter_depth()?;
        let mut lhs = self.parse_prefix_expr()?;

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
            self.cursor += 1;

            if op == Token::Keyword(Keyword::Catch) {
                let payload = self.parse_optional_payload()?;
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

        self.leave_depth();
        Ok(lhs)
    }

    /// PrefixExpr <- PrefixOp* PrimaryExpr
    fn parse_prefix_expr(&mut self) -> Result<ast::Expression, Error> {
        if self.current_is_keyword(Keyword::Anyframe)
            && self
                .peek_non_comment(1)
                .is_some_and(|token| matches!(token.token, Token::Symbol(Symbol::MinusRArrow)))
        {
            let pos = self.current_pos();
            self.cursor += 1;
            self.expect_symbol(Symbol::MinusRArrow)?;
            let result = self.parse_prefix_expr()?;
            return Ok(ast::Expression::AnyframeType(ast::AnyframeType {
                pos,
                result: Some(Box::new(result)),
            }));
        }

        if self.current_is_symbol(Symbol::QuestionMark) {
            let pos = self.current_pos();
            self.cursor += 1;
            let child = self.parse_prefix_expr()?;
            return Ok(ast::Expression::Optional(ast::OptionalType {
                pos,
                child: Box::new(child),
            }));
        }

        if self.starts_type_prefix() {
            return self.parse_prefix_type_op();
        }

        if self.current_is_prefix_op() {
            let pos = self.current_pos();
            let op = self.bump_token().expect("current token exists");
            let expr = self.parse_prefix_expr()?;
            return Ok(ast::Expression::Unary(ast::UnaryOp {
                pos,
                op,
                expr: Box::new(expr),
            }));
        }

        self.parse_suffix_expr()
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
        let kind = if self.consume_symbol(Symbol::Asterisk) {
            ast::PointerKind::One
        } else if self.consume_symbol(Symbol::Asterisk2) {
            ast::PointerKind::Many
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
        let modifiers = self.parse_pointer_modifiers()?;
        let child = self.parse_prefix_expr()?;
        let tokens = self.tokens_since(mark);
        Ok(ast::Expression::Pointer(ast::PointerType {
            pos,
            kind,
            modifiers,
            child: Box::new(child),
            tokens,
        }))
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
        let modifiers = self.parse_pointer_modifiers()?;
        let child = self.parse_prefix_expr()?;
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
        let child = self.parse_prefix_expr()?;
        let tokens = self.tokens_since(mark);
        Ok(ast::Expression::ArrayType(ast::ArrayType {
            pos,
            len: Box::new(len),
            sentinel,
            child: Box::new(child),
            tokens,
        }))
    }

    fn parse_pointer_modifiers(&mut self) -> Result<Vec<ast::PointerModifier>, Error> {
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
                if self.consume_symbol(Symbol::Colon) {
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
    fn parse_suffix_expr(&mut self) -> Result<ast::Expression, Error> {
        let mut expr = self.parse_primary_expr()?;

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
                self.cursor += 1;
                expr = ast::Expression::Deref(ast::Deref {
                    pos,
                    expr: Box::new(expr),
                });
                continue;
            }

            if self.current_is_symbol(Symbol::DotQuestionMark) {
                let pos = self.current_pos();
                self.cursor += 1;
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
                self.cursor += 1;
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

            if self.current_is_symbol(Symbol::LBrace) && self.suppress_curly_suffix == 0 {
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
            self.cursor += 1;
            let payload = self.parse_prefix_expr()?;
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
            if self.current_is_symbol(Symbol::Dot)
                && self
                    .peek_non_comment(1)
                    .is_some_and(|token| matches!(token.token, Token::Literal(LitKind::Ident, _)))
                && self
                    .peek_non_comment(2)
                    .is_some_and(|token| matches!(token.token, Token::Symbol(Symbol::Equal)))
            {
                self.expect_symbol(Symbol::Dot)?;
                let name = self.expect_ident(self.current_pos())?;
                self.expect_symbol(Symbol::Equal)?;
                let value = self.parse_expr_bp(0)?;
                entries.push(ast::InitEntry::Field { name, value });
            } else {
                entries.push(ast::InitEntry::Expr(self.parse_expr_bp(0)?));
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
                        self.cursor += 1;
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
                self.cursor += 1;
                Ok(ast::Expression::Ident(ast::Ident {
                    pos: current.pos,
                    name,
                }))
            }
            Token::Literal(LitKind::BuiltinIdent, name) => {
                self.cursor += 1;
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
                self.cursor += 1;
                Ok(ast::Expression::BasicLit(ast::BasicLit {
                    pos: current.pos,
                    kind,
                    text,
                }))
            }
            Token::Literal(LitKind::MultilineString, text) => {
                self.cursor += 1;
                Ok(ast::Expression::MultilineStr(ast::MultilineStr {
                    pos: current.pos,
                    text,
                }))
            }
            Token::Keyword(Keyword::Unreachable) => {
                self.cursor += 1;
                Ok(ast::Expression::Unreachable(current.pos))
            }
            Token::Keyword(Keyword::Anyframe) => {
                self.cursor += 1;
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
                self.cursor += 1;
                let expr = self.parse_expr_bp(0)?;
                self.expect_symbol(Symbol::RParen)?;
                Ok(ast::Expression::Grouped(Box::new(expr)))
            }
            Token::Symbol(Symbol::Dot) => {
                self.cursor += 1;
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
                self.cursor += 1;
                let expr = self.parse_expr_bp(0)?;
                Ok(ast::Expression::Comptime(Box::new(expr)))
            }
            Token::Keyword(Keyword::Nosuspend) => {
                self.cursor += 1;
                let expr = self.parse_expr_bp(0)?;
                Ok(ast::Expression::Nosuspend(Box::new(expr)))
            }
            Token::Keyword(Keyword::Resume) => {
                self.cursor += 1;
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
                self.cursor += 1;
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
                self.cursor += 1;
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
                self.cursor += 1;
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
                self.cursor += 1;
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
            self.collect_comments(&mut Vec::new());
            if self.current_is_symbol(Symbol::RBrace) {
                break;
            }
            names.push(self.expect_ident(self.current_pos())?);
            self.consume_symbol(Symbol::Comma);
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
                self.cursor += 1;
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
        let arg = if self.consume_symbol(Symbol::LParen) {
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
        let members = self.parse_container_members_until(Symbol::RBrace)?;
        self.expect_symbol(Symbol::RBrace)?;
        let tokens = self.tokens_since(mark);
        Ok(ast::Expression::Container(ast::ContainerDecl {
            pos,
            layout,
            kind,
            arg,
            members,
            tokens,
        }))
    }

    fn parse_container_members_until(
        &mut self,
        end: Symbol,
    ) -> Result<Vec<ast::ContainerMember>, Error> {
        let mut members = Vec::new();
        let mut container_docs = Vec::new();
        while !self.current_is_symbol(end) {
            if self.is_eof() {
                return Err(self.unexpected(vec![TokenKind::Symbol(end)], None));
            }
            let docs = self.collect_comments(&mut container_docs);
            if self.current_is_symbol(end) {
                break;
            }
            members.push(self.parse_container_member(docs)?);
        }
        Ok(members)
    }

    /// IfPrefix <- KEYWORD_if LPAREN Expr RPAREN PtrPayload?
    fn parse_if_prefix(&mut self) -> Result<(ast::Expression, Option<ast::Payload>), Error> {
        self.expect_keyword(Keyword::If)?;
        self.expect_symbol(Symbol::LParen)?;
        let condition = self.parse_expr_bp(0)?;
        self.expect_symbol(Symbol::RParen)?;
        let payload = self.parse_optional_payload()?;
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
            let payload = self.parse_optional_payload()?;
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
        let payload = self.parse_optional_payload()?;
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
            let payload = self.parse_optional_payload()?;
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
        let mut items = Vec::new();
        if !self.current_is_symbol(Symbol::RParen) {
            loop {
                self.skip_comments();
                if self.current_is_symbol(Symbol::RParen) {
                    break;
                }
                items.push(self.parse_for_item()?);
                if !self.consume_symbol(Symbol::Comma) {
                    break;
                }
                self.skip_comments();
                if self.current_is_symbol(Symbol::RParen) {
                    break;
                }
            }
        }
        self.expect_symbol(Symbol::RParen)?;
        let payload = self.parse_optional_payload()?.ok_or_else(|| {
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
            self.consume_symbol(Symbol::Comma);
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
        let payload = self.parse_optional_payload()?;
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
                self.cursor += 1;
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
            args.push(self.parse_expr_bp(0)?);
            if self.current_is_symbol(Symbol::Comma) {
                self.cursor += 1;
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

    fn parse_optional_payload(&mut self) -> Result<Option<ast::Payload>, Error> {
        if !self.current_is_symbol(Symbol::Pipe) {
            return Ok(None);
        }
        let pos = self.current_pos();
        self.cursor += 1;
        let mut stars = Vec::new();
        let star = self.consume_symbol(Symbol::Asterisk);
        stars.push(star);
        let mut names = Vec::new();
        names.push(self.expect_ident(self.current_pos())?);
        while self.current_is_symbol(Symbol::Comma) {
            self.cursor += 1;
            if self.current_is_symbol(Symbol::Pipe) {
                break;
            }
            let star = self.consume_symbol(Symbol::Asterisk);
            stars.push(star);
            names.push(self.expect_ident(self.current_pos())?);
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
        self.tokens.get(self.cursor)
    }

    fn check_scan_error(&mut self) -> Result<(), Error> {
        match self.scan_error.take() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn current_pos(&self) -> usize {
        self.current()
            .map(|token| token.pos)
            .unwrap_or_else(|| *self.line_info.last().unwrap_or(&0))
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

    fn current_stops_optional_expr(&self) -> bool {
        self.is_eof()
            || matches!(
                self.current().map(|token| &token.token),
                Some(Token::Keyword(
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
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn consume_keyword(&mut self, keyword: Keyword) -> bool {
        if self.current_is_keyword(keyword) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn consume_ident_text(&mut self, expected: &str) -> bool {
        match self.current().map(|token| &token.token) {
            Some(Token::Literal(LitKind::Ident, text)) if text == expected => {
                self.cursor += 1;
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
        self.cursor += 1;
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
                self.cursor += 1;
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
                self.cursor += 1;
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
                self.cursor += 1;
                Ok(ast::Ident { pos, name })
            }
            actual => Err(self.unexpected(vec![TokenKind::Literal(LitKind::Ident)], Some(actual))),
        }
    }

    fn is_eof(&self) -> bool {
        self.cursor >= self.tokens.len()
    }

    fn enter_depth(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error_at(self.current_pos(), "parser recursion depth exceeded"));
        }
        Ok(())
    }

    fn leave_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    pub fn preback(&self) -> ParserMark {
        ParserMark {
            cursor: self.cursor,
            depth: self.depth,
            suppress_curly_suffix: self.suppress_curly_suffix,
        }
    }

    pub fn goback(&mut self, mark: ParserMark) {
        self.cursor = mark.cursor;
        self.depth = mark.depth;
        self.suppress_curly_suffix = mark.suppress_curly_suffix;
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
            location: self.location_for_current(),
            expect,
            actual,
        }
    }

    fn error_at(&self, pos: usize, reason: &str) -> Error {
        Error::Else {
            path: self.path.clone(),
            location: self.line_col(pos),
            reason: reason.to_owned(),
        }
    }

    fn location_for_current(&self) -> (usize, usize) {
        self.current()
            .map(|token| self.line_col(token.pos))
            .unwrap_or_else(|| self.line_col(*self.line_info.last().unwrap_or(&0)))
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

fn modifier_to_var(modifier: Option<ast::FnModifier>) -> Result<Option<ast::VarModifier>, Error> {
    Ok(match modifier {
        Some(ast::FnModifier::Export) => Some(ast::VarModifier::Export),
        Some(ast::FnModifier::Extern(abi)) => Some(ast::VarModifier::Extern(abi)),
        Some(ast::FnModifier::Inline | ast::FnModifier::NoInline) => {
            return Err(Error::Else {
                path: PathBuf::from("<source>"),
                location: (1, 1),
                reason: "function-only modifier used on variable".to_owned(),
            })
        }
        None => None,
    })
}

fn statement_expr_can_omit_semicolon(expr: &ast::Expression) -> bool {
    matches!(
        expr,
        ast::Expression::Block(_)
            | ast::Expression::If(_)
            | ast::Expression::While(_)
            | ast::Expression::For(_)
            | ast::Expression::Switch(_)
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
    fn expression_depth_guard_returns_error() {
        let source = "(".repeat(1100) + "x" + &")".repeat(1100);
        let mut parser = Parser::from(source);
        assert!(parser.expression().is_err());
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
        let mut parser = Parser::from(".{ .a = x, y }");
        let ast::Expression::AnonInit(init) = parser.expression().unwrap() else {
            panic!("expected anonymous initializer");
        };
        let names: Vec<_> = init
            .fields()
            .map(|field| match field {
                ast::Expression::Ident(ident) => ident.name.as_str(),
                other => panic!("unexpected field {other:?}"),
            })
            .collect();
        assert_eq!(names, ["x", "y"]);
    }

    #[test]
    fn var_decl_ty_is_the_first_prototype_type() {
        let file = parse_source(
            "const a: u8 = 1;\nconst b = 2;\nfn f() void { var c: u16, var d: u32 = g(); }",
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
        let ast::Statement::Var(c) = &f.body.as_ref().unwrap().statements[0] else {
            panic!("expected var statement");
        };
        assert_eq!(ty_name(c).as_deref(), Some("u16"));
    }
}
