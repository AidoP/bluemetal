//! Statements, and the parts of them which report what is wrong when they cannot be taken.

use std::path::PathBuf;

use super::syntax::{Parser, Span, invalid_ident_char, is_crate_reference};
use super::meta::Meta;
use super::value::{Coercion, Operand};
use super::{Data, Diagnostic, File, Message, Project, Severity, Value};

pub(super) struct Attribute<'a> {
    pub(super) name: &'a str,
    pub(super) name_span: Span,
    pub(super) data: Data,
    pub(super) data_span: Span,
    /// Where the value was set, if the data is the value of a property.
    pub(super) origin: Option<Span>,
}
impl Attribute<'_> {
    /// The value of the attribute as an operand, which is not converted to another type.
    pub(super) fn operand(self) -> Operand {
        Operand { data: self.data, span: self.data_span, coerce: Coercion::Strict, origin: self.origin }
    }
}

impl Project {
    pub(super) fn parse_file(&mut self, idx: usize) {
        let data = std::mem::take(&mut self.files[idx].data);
        let mut parser = Parser::new(idx, &data);
        let mut chain = None;
        loop {
            parser.skip_empty();
            if parser.is_empty() {
                break;
            }
            self.parse_statement(&mut parser, &mut chain);
        }
        self.files[idx].data = data;
    }
    /// Take a statement, with the meta-statements before it, and the statements chained after it with `@then`, which
    /// are processed or skipped with it.
    pub(super) fn parse_statement(&mut self, parser: &mut Parser, chain: &mut Option<bool>) {
        let process = match self.parse_meta(parser, chain) {
            Meta::Skip => false,
            // `@for` processes the chained statements too
            Meta::Done => return,
            Meta::Process => true,
        };
        if process {
            self.parse_single_statement(parser);
        } else {
            parser.skip_statement();
        }
        while self.parse_then(parser) {
            if process {
                self.parse_statement(parser, &mut None);
            } else {
                Self::skip_chained(parser);
            }
        }
    }
    /// Take a `@then` directly after a statement, returning whether there is one.
    fn parse_then(&mut self, parser: &mut Parser) -> bool {
        let Some(rest) = parser.as_str().strip_prefix("@then") else {
            return false;
        };
        if !rest.is_empty() && !rest.starts_with(['\n', ' ']) {
            return false;
        }
        let span = Span { file: parser.file, offset: parser.offset, len: 5 };
        parser.offset += 5;
        if !self.parse_line_end(parser) {
            parser.skip_line();
        }
        if matches!(parser.peek(), None | Some('\n' | '#')) {
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::ExpectedStatementAfterMeta,
                severity: Severity::Error,
            });
            return false;
        }
        true
    }
    /// Skip a statement chained with `@then` to a skipped statement, with its meta-statements.
    fn skip_chained(parser: &mut Parser) {
        while parser.peek() == Some('@') {
            parser.skip_line();
        }
        parser.skip_statement();
    }
    /// Take a statement, after its meta-statements.
    fn parse_single_statement(&mut self, parser: &mut Parser) {
        let start = parser.offset;
        let Some((keyword, span)) = self.parse_ident(parser) else {
            parser.skip_statement();
            return;
        };
        match keyword {
            "declare" => self.parse_declare(parser),
            "use" => self.parse_use(parser),
            "set" => self.parse_set(parser),
            "log" => self.parse_log(parser),
            "crate" => self.parse_crate(parser),
            "custom" => self.parse_custom(parser, false),
            "test" => self.parse_custom(parser, true),
            "target" => self.parse_target(parser),
            _ => {
                parser.offset = start;
                let _ = parser.skip_statement();
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::UnknownStatement(keyword.into()),
                    severity: Severity::Error,
                });
            },
        }
    }
    fn parse_use(&mut self, parser: &mut Parser) {
        if !self.parse_space(parser) {
            parser.skip_statement();
            return;
        }
        let Some((mut path, span)) = self.parse_path(parser) else {
            parser.skip_statement();
            return;
        };
        if !self.parse_line_end(parser) {
            parser.skip_statement();
        }
        path = path.join("Build");
        match File::read(path.clone()) {
            Ok(mut file) => {
                file.used = Some(span);
                // using a file again is fine unless it is still being processed, which is a cycle
                if let Some(idx) = self.parse(file) && self.files[idx].active {
                    self.diagnostics.push(Diagnostic {
                        span: Some(span),
                        message: Message::RepeatedUse { path, file: idx },
                        severity: Severity::Warning,
                    });
                }
            },
            Err(error) => {
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::ReadFailure(path, error),
                    severity: Severity::Error,
                });
            },
        };
    }
    fn parse_log(&mut self, parser: &mut Parser) {
        if !self.parse_space(parser) {
            parser.skip_statement();
            return;
        }
        let Some((severity, span)) = self.parse_ident(parser) else {
            parser.skip_statement();
            return;
        };
        let severity = match severity {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            "note" => Severity::Note,
            "suggestion" => Severity::Suggestion,
            _ => {
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::UnknownSeverity(severity.into()),
                    severity: Severity::Error,
                });
                Severity::Error
            },
        };
        if !self.parse_space(parser) {
            parser.skip_statement();
            return;
        }
        let Some(operand) = self.parse_expression(parser) else {
            parser.skip_statement();
            return;
        };
        if !self.parse_line_end(parser) {
            parser.skip_statement();
        }
        // any value can be logged, as it would be in a string template
        self.diagnostics.push(Diagnostic {
            span: Some(operand.span),
            message: Message::User(operand.data.coerce_string()),
            severity,
        });
    }
    /// Take the next identifier from the parser.
    ///
    /// If the parser is not pointing at an identifier, a diagnostic is created, naming the first invalid character if
    /// the parser is pointing at a word.
    pub(super) fn parse_ident<'a>(&mut self, parser: &mut Parser<'a>) -> Option<(&'a str, Span)> {
        let rest = parser.as_str();
        let word = &rest[..rest.find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-').unwrap_or(rest.len())];
        let start = parser.offset;
        if let Some((ident, span)) = parser.ident() && ident.len() == word.len() {
            return Some((ident, span));
        }
        parser.offset = start;
        let (span, message) = match word.char_indices().find(invalid_ident_char) {
            Some((i, character)) => (
                Span { file: parser.file, offset: start + i, len: character.len_utf8() },
                Message::InvalidIdentifier { name: word.into(), character },
            ),
            None => (parser.span_until(|c| c == ' ' || c == '\n'), Message::ExpectedIdentifier),
        };
        self.diagnostics.push(Diagnostic {
            span: Some(span),
            message,
            severity: Severity::Error,
        });
        None
    }
    /// Take the name of a property or crate, which is an identifier, or an expression in `{}` whose value is one.
    ///
    /// Also returns whether the name was written as an identifier, so that its characters are at known offsets.
    pub(super) fn parse_name(&mut self, parser: &mut Parser) -> Option<(String, Span, bool)> {
        if parser.peek() != Some('{') {
            let (ident, span) = self.parse_ident(parser)?;
            return Some((ident.into(), span, true));
        }
        let operand = self.parse_operand(parser)?;
        let name = operand.data.coerce_string();
        let message = match name.char_indices().find(invalid_ident_char) {
            Some((_, character)) => Message::InvalidIdentifier { name, character },
            None if name.is_empty() => Message::ExpectedIdentifier,
            None => return Some((name, operand.span, false)),
        };
        self.diagnostics.push(Diagnostic {
            span: Some(operand.span),
            message,
            severity: Severity::Error,
        });
        None
    }
    /// Takes the next space from the parser and returns true.
    /// If the parser is not pointing at a space, a diagnostic is created and false is returned.
    pub(super) fn parse_space(&mut self, parser: &mut Parser) -> bool {
        if parser.expect(' ') {
            true
        } else {
            self.diagnostics.push(Diagnostic {
                span: Some(parser.span_char()),
                message: Message::ExpectedExact(" "),
                severity: Severity::Error,
            });
            false
        }
    }
    /// Takes the next new line character from the parser and returns true.
    /// If the parser is not pointing at a new line, a diagnostic is created and false is returned.
    pub(super) fn parse_line_end(&mut self, parser: &mut Parser) -> bool {
        if parser.expect('\n') {
            true
        } else {
            self.diagnostics.push(Diagnostic {
                span: Some(parser.span_char()),
                message: Message::ExpectedExact("\n"),
                severity: Severity::Error,
            });
            false
        }
    }
    /// Take the next path from the parser.
    ///
    /// A path is a string literal which resolves to a path relative to the project root.
    pub(super) fn parse_path(&mut self, parser: &mut Parser) -> Option<(PathBuf, Span)> {
        let Some((path, span)) = parser.string() else {
            self.diagnostics.push(Diagnostic {
                span: Some(parser.span_until(|c| c == ' ' || c == '\n')),
                message: Message::ExpectedPath,
                severity: Severity::Error,
            });
            return None;
        };
        let path_buf;
        if path.starts_with('/') {
            let path = self.evaluate_str(&path[1..], span.file, span.offset + 2);
            path_buf = self.root.join(path);
        } else {
            let path = self.evaluate_str(path, span.file, span.offset + 1);
            path_buf = self.file(parser.file).relative_path(path)
        }
        Some((path_buf, span))
    }
    /// Take the next indented attribute of a statement, skipping those which are invalid, which have been reported.
    pub(super) fn next_attribute<'a>(&mut self, parser: &mut Parser<'a>, names: &[&str]) -> Option<Attribute<'a>> {
        while parser.indent().is_some() {
            if let Some(attribute) = self.parse_attribute(parser, names) {
                return Some(attribute);
            }
            parser.skip_line();
        }
        None
    }
    /// Take an indented attribute, with an optional value.
    ///
    /// The value of an attribute in `names` is a name, as taken by [`Self::parse_name`], and is required. Otherwise it
    /// is an expression.
    fn parse_attribute<'a>(&mut self, parser: &mut Parser<'a>, names: &[&str]) -> Option<Attribute<'a>> {
        let (name, name_span) = self.parse_ident(parser)?;
        let (data, data_span, origin) = if names.contains(&name) {
            if !self.parse_space(parser) {
                return None;
            }
            let (mut value, mut span, _) = self.parse_name(parser)?;
            // a crate can only use crates for the same target, so it is not given, unlike in a reference to a crate
            if is_crate_reference(parser) {
                parser.expect('/');
                let (crate_name, crate_span) = self.parse_ident(parser)?;
                // the target is a property, such as `host-target`, or the name of a target, as in a reference
                let target = match self.value(&value, span.file) {
                    Some(Value { data: Data::Str(target), .. }) => target.clone(),
                    _ => value,
                };
                let declared = self.crates.get(&(crate_name.into(), target)).map(|c| c.name_span);
                value = crate_name.into();
                span = Span { file: span.file, offset: span.offset, len: crate_span.offset + crate_span.len - span.offset };
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::UseWithTarget { declared },
                    severity: Severity::Error,
                });
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::Replacement { description: "use the crate by its name", replacement: value.clone() },
                    severity: Severity::Suggestion,
                });
            }
            (Data::Str(value), span, None)
        } else if parser.expect(' ') {
            let operand = self.parse_expression(parser)?;
            (operand.data, operand.span, operand.origin)
        } else {
            (Data::None, name_span, None)
        };
        if !self.parse_line_end(parser) {
            parser.skip_line();
        }
        Some(Attribute {
            name,
            name_span,
            data,
            data_span,
            origin,
        })
    }
}
