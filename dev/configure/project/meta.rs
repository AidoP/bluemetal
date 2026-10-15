//! Meta-statements, which decide whether, and how often, the statement after them is processed.

use super::diagnostic::{Diagnostic, Message, Severity};
use super::syntax::{Parser, Span};
use super::{Data, Project};

/// What remains of a statement after its meta-statements.
pub(super) enum Meta {
    /// The statement is skipped.
    Skip,
    /// The statement is processed.
    Process,
    /// The statement has been processed already, by `@for`.
    Done,
}

impl Project {
    /// Take the meta-statements which precede a statement, returning whether the statement should be processed.
    ///
    /// Each meta-statement is a line starting with `@`, and must be followed directly by another meta-statement or by
    /// the statement it applies to. Once the statement is known to be skipped, later conditions are not evaluated.
    ///
    /// `chain` is whether a statement has been processed in the current chain of conditional statements, which `@else`
    /// continues, or [`None`] if the previous statement was unconditional.
    ///
    /// `@for` processes the meta-statements and statement after it for each value, leaving nothing more to process.
    pub(super) fn parse_meta(&mut self, parser: &mut Parser, chain: &mut Option<bool>) -> Meta {
        let mut active = true;
        // whether the statement is conditional, so that it starts or continues a chain
        let mut conditional = false;
        // whether the chain had already processed a statement, if this statement continues it with `@else`
        let mut continues = None;
        // an invalid meta-statement skips the statement, and any `@else` after it
        let mut failed = false;
        let mut first = true;
        while parser.peek() == Some('@') {
            let start = parser.offset;
            parser.expect('@');
            let Some((name, span)) = self.parse_ident(parser) else {
                parser.skip_line();
                failed = true;
                continue;
            };
            let meta_span = Span { file: span.file, offset: start, len: span.offset + span.len - start };
            match name {
                "if" => {
                    conditional = true;
                    if !active {
                        parser.skip_line();
                    } else if let Some(condition) = self.parse_condition(parser) {
                        active = condition;
                    } else {
                        failed = true;
                    }
                },
                // a skipped statement is not repeated
                "for" if !active || failed => {
                    parser.skip_line();
                },
                "for" => {
                    let Some((name, values)) = self.parse_for(parser) else {
                        failed = true;
                        continue;
                    };
                    if matches!(parser.peek(), None | Some('\n' | '#')) {
                        self.diagnostics.push(Diagnostic {
                            span: Some(meta_span),
                            message: Message::ExpectedStatementAfterMeta,
                            severity: Severity::Error,
                        });
                        *chain = None;
                        return Meta::Skip;
                    }
                    *chain = match continues {
                        Some(_) => Some(true),
                        None if conditional => Some(true),
                        None => None,
                    };
                    let body = parser.offset;
                    let previous = self.files[parser.file].locals.get(&name).copied();
                    for value in &values {
                        parser.offset = body;
                        self.set_local(&name, value.clone(), parser.file);
                        self.parse_statement(parser, &mut None);
                    }
                    match previous {
                        Some(previous) => self.files[parser.file].locals.insert(name, previous),
                        None => self.files[parser.file].locals.remove(&name),
                    };
                    if !values.is_empty() {
                        return Meta::Done;
                    }
                    // without any values, the rest is skipped
                    while parser.peek() == Some('@') {
                        parser.skip_line();
                    }
                    return Meta::Skip;
                },
                // `@then` is taken with the statement before it
                "then" => {
                    parser.skip_line();
                    self.diagnostics.push(Diagnostic {
                        span: Some(meta_span),
                        message: Message::ThenWithoutStatement,
                        severity: Severity::Error,
                    });
                    failed = true;
                },
                "else" | "else-if" => {
                    conditional = true;
                    match *chain {
                        Some(processed) if first => {
                            continues = Some(processed);
                            active &= !processed;
                        },
                        _ => {
                            let message = if first {
                                Message::ElseWithoutIf(name.into())
                            } else {
                                Message::ElseNotFirst(name.into())
                            };
                            self.diagnostics.push(Diagnostic {
                                span: Some(meta_span),
                                message,
                                severity: Severity::Error,
                            });
                            failed = true;
                        },
                    }
                    if name == "else" {
                        if !self.parse_line_end(parser) {
                            parser.skip_line();
                            failed = true;
                        }
                    } else if !active || failed {
                        parser.skip_line();
                    } else if let Some(condition) = self.parse_condition(parser) {
                        active = condition;
                    } else {
                        failed = true;
                    }
                },
                _ => {
                    parser.skip_line();
                    self.diagnostics.push(Diagnostic {
                        span: Some(meta_span),
                        message: Message::UnknownMetaStatement(name.into()),
                        severity: Severity::Error,
                    });
                    failed = true;
                },
            }
            first = false;
            if matches!(parser.peek(), None | Some('\n' | '#')) {
                self.diagnostics.push(Diagnostic {
                    span: Some(meta_span),
                    message: Message::ExpectedStatementAfterMeta,
                    severity: Severity::Error,
                });
                *chain = None;
                return Meta::Skip;
            }
        }
        if failed {
            *chain = Some(true);
            return Meta::Skip;
        }
        *chain = match continues {
            Some(processed) => Some(processed || active),
            None if conditional => Some(active),
            None => None,
        };
        if active { Meta::Process } else { Meta::Skip }
    }
    /// Take the rest of a `@for`, a property and the values it takes, separated by spaces, where each item of a list is
    /// a value.
    fn parse_for(&mut self, parser: &mut Parser) -> Option<(String, Vec<Data>)> {
        let header = (|| {
            if !self.parse_space(parser) {
                return None;
            }
            let (name, _, _) = self.parse_name(parser)?;
            if !self.parse_space(parser) {
                return None;
            }
            if !matches!(parser.as_str().strip_prefix("in"), Some(rest) if rest.starts_with([' ', '\n'])) {
                self.diagnostics.push(Diagnostic {
                    span: Some(parser.span_until(|c| c == ' ' || c == '\n')),
                    message: Message::ExpectedExact("in"),
                    severity: Severity::Error,
                });
                return None;
            }
            parser.offset += 2;
            let mut values = Vec::new();
            loop {
                if !self.parse_space(parser) {
                    return None;
                }
                match self.parse_expression(parser)?.data {
                    Data::List(items) => values.extend(items),
                    data => values.push(data),
                }
                if parser.peek() != Some(' ') {
                    break;
                }
            }
            self.parse_line_end(parser).then_some((name, values))
        })();
        if header.is_none() {
            parser.skip_line();
        }
        header
    }
    /// Take the condition of an `@if` to the end of the line, returning [`None`] if it is invalid.
    fn parse_condition(&mut self, parser: &mut Parser) -> Option<bool> {
        if !self.parse_space(parser) {
            parser.skip_line();
            return None;
        }
        let Some(operand) = self.parse_binary(parser, 0) else {
            parser.skip_line();
            return None;
        };
        if !self.parse_line_end(parser) {
            parser.skip_line();
            return None;
        }
        self.bool_operand(operand)
    }
}
