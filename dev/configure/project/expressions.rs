//! Expressions and string templates, which are evaluated as they are parsed.

use super::diagnostic::{Diagnostic, Message, Severity};
use super::syntax::{Operator, Parser, Span, expression_end, is_attribute_access, is_crate_reference};
use super::value::{Coercion, Data, Operand, Value};
use super::Project;

impl Project {
    /// Take the next expression from the parser and evaluate it.
    ///
    /// An expression is an operand, or operands joined by operators.
    /// An operator must have a single space on either side of it.
    pub(super) fn parse_expression(&mut self, parser: &mut Parser) -> Option<Operand> {
        self.parse_binary(parser, 0)
    }
    /// Take operands joined by operators with a precedence of at least `min_precedence`.
    pub(super) fn parse_binary(&mut self, parser: &mut Parser, min_precedence: u8) -> Option<Operand> {
        let mut lhs = self.parse_operand(parser)?;
        while let Some(operator) = parser.operator(min_precedence) {
            let rhs = self.parse_binary(parser, operator.precedence() + 1)?;
            lhs = self.evaluate_binary(operator, lhs, rhs)?;
        }
        Some(lhs)
    }
    /// Take the next operand from the parser.
    ///
    /// An operand is a literal, a property name, an expression in `{}` to permit coercion, or an expression in `()`.
    /// It may be preceded by `!` to negate a boolean.
    pub(super) fn parse_operand(&mut self, parser: &mut Parser) -> Option<Operand> {
        let start = parser.offset;
        match parser.peek() {
            Some('\'') => {
                let (string, span) = self.parse_string(parser)?;
                Some(Operand { data: Data::Str(string), span, coerce: Coercion::Strict, origin: None })
            },
            Some('0'..='9' | '-' | '+') => {
                let span = parser.span_until(|c| c == ' ' || c == '\n');
                let Some((data, span)) = parser.integer() else {
                    self.diagnostics.push(Diagnostic {
                        span: Some(span),
                        message: Message::InvalidInteger,
                        severity: Severity::Error,
                    });
                    return None;
                };
                Some(Operand { data, span, coerce: Coercion::Sign, origin: None })
            },
            // the complement of an integer, or the negation of a boolean, as which `{}` is taken
            Some('!') => {
                parser.expect('!');
                let operand = self.parse_operand(parser)?;
                let data = match operand.data {
                    Data::Unsigned(i) if operand.coerce != Coercion::Any => Data::Unsigned(!i),
                    Data::Signed(i) if operand.coerce != Coercion::Any => Data::Signed(!i),
                    _ => Data::Bool(!self.bool_operand(operand)?),
                };
                Some(Operand { data, span: parser.span_from(start), coerce: Coercion::Strict, origin: None })
            },
            Some('(') => {
                parser.expect('(');
                let inner = self.parse_binary(parser, 0)?;
                if !parser.expect(')') {
                    self.diagnostics.push(Diagnostic {
                        span: Some(parser.span_char()),
                        message: Message::ExpectedExact(")"),
                        severity: Severity::Error,
                    });
                    return None;
                }
                Some(Operand { span: parser.span_from(start), ..inner })
            },
            Some('[') => {
                parser.expect('[');
                let items = self.parse_arguments(parser, ']', |this, parser| this.parse_binary(parser, 0))?;
                let data = Data::List(items.into_iter().map(|item| item.data).collect());
                Some(Operand { data, span: parser.span_from(start), coerce: Coercion::Strict, origin: None })
            },
            Some('{') => {
                parser.expect('{');
                let inner = self.parse_binary(parser, 0)?;
                if !parser.expect('}') {
                    self.diagnostics.push(Diagnostic {
                        span: Some(parser.span_char()),
                        message: Message::ExpectedExact("}"),
                        severity: Severity::Error,
                    });
                    return None;
                }
                // the target of a crate may be computed
                if is_crate_reference(parser) {
                    return self.parse_crate_reference(parser, inner.data.coerce_string(), start);
                }
                Some(Operand { span: parser.span_from(start), coerce: Coercion::Any, ..inner })
            },
            // any word is parsed as an identifier, to report its invalid characters
            Some(c) if c.is_alphabetic() || c == '_' => {
                let (name, span) = self.parse_ident(parser)?;
                if parser.peek() == Some('(') {
                    return self.parse_call(parser, name, span);
                }
                if is_crate_reference(parser) {
                    // the target is a property, such as `host-target`, or the name of a target
                    let target = match self.value(name, span.file) {
                        Some(Value { data: Data::Str(target), .. }) => target.clone(),
                        _ => name.into(),
                    };
                    return self.parse_crate_reference(parser, target, start);
                }
                if is_attribute_access(parser) {
                    return self.parse_named_attribute(parser, name, span, start);
                }
                let (data, origin) = match name {
                    "true" => (Data::Bool(true), None),
                    "false" => (Data::Bool(false), None),
                    _ => self.property(name, span)?,
                };
                Some(Operand { data, span, coerce: Coercion::Strict, origin })
            },
            _ => {
                self.diagnostics.push(Diagnostic {
                    span: Some(parser.span_until(|c| c == ' ' || c == '\n')),
                    message: Message::ExpectedExpression,
                    severity: Severity::Error,
                });
                None
            },
        }
    }
    /// Apply an operator.
    ///
    /// Operands must have the type required by the operator, and the same type as each other.
    /// An operand is only converted to satisfy this as far as its [`Coercion`] permits.
    pub(super) fn evaluate_binary(&mut self, operator: Operator, lhs: Operand, rhs: Operand) -> Option<Operand> {
        let span = Span {
            file: lhs.span.file,
            offset: lhs.span.offset,
            len: rhs.span.offset + rhs.span.len - lhs.span.offset,
        };
        let rhs_span = rhs.span;
        let mut coerce = Coercion::Strict;
        let data = match operator {
            Operator::Or | Operator::And => {
                let lhs = self.bool_operand(lhs)?;
                let rhs = self.bool_operand(rhs)?;
                Data::Bool(match operator {
                    Operator::Or => lhs || rhs,
                    _ => lhs && rhs,
                })
            },
            Operator::Equal | Operator::NotEqual => {
                let (lhs, rhs) = self.comparable_operands(lhs, rhs, span)?;
                Data::Bool((lhs == rhs) == matches!(operator, Operator::Equal))
            },
            Operator::Less | Operator::LessEqual | Operator::Greater | Operator::GreaterEqual => {
                let ordering = match self.integer_operands(lhs, rhs, span)? {
                    (Data::Unsigned(lhs), Data::Unsigned(rhs)) => lhs.cmp(&rhs),
                    (Data::Signed(lhs), Data::Signed(rhs)) => lhs.cmp(&rhs),
                    _ => unreachable!(),
                };
                Data::Bool(match operator {
                    Operator::Less => ordering.is_lt(),
                    Operator::LessEqual => ordering.is_le(),
                    Operator::Greater => ordering.is_gt(),
                    _ => ordering.is_ge(),
                })
            },
            // lists are concatenated, but only with other lists
            Operator::Add if matches!(lhs.data, Data::List(_)) || matches!(rhs.data, Data::List(_)) => {
                let (Data::List(mut lhs_items), Data::List(rhs_items)) = (lhs.data.clone(), rhs.data.clone()) else {
                    self.diagnostics.push(Diagnostic {
                        span: Some(span),
                        message: Message::MismatchedOperands {
                            lhs: lhs.data.type_name(),
                            rhs: rhs.data.type_name(),
                            lhs_origin: lhs.origin,
                            rhs_origin: rhs.origin,
                        },
                        severity: Severity::Error,
                    });
                    return None;
                };
                lhs_items.extend(rhs_items);
                Data::List(lhs_items)
            },
            Operator::Add
            | Operator::Subtract
            | Operator::Multiply
            | Operator::Divide
            | Operator::Remainder
            | Operator::BitOr
            | Operator::BitXor
            | Operator::BitAnd
            | Operator::ShiftLeft
            | Operator::ShiftRight => {
                coerce = lhs.coerce.min(rhs.coerce);
                let (lhs, rhs) = self.integer_operands(lhs, rhs, span)?;
                if matches!(operator, Operator::Divide | Operator::Remainder) && matches!(rhs, Data::Unsigned(0) | Data::Signed(0)) {
                    self.diagnostics.push(Diagnostic {
                        span: Some(rhs_span),
                        message: Message::DivisionByZero,
                        severity: Severity::Error,
                    });
                    return None;
                }
                macro_rules! checked {
                    ($lhs:expr, $rhs:expr) => {
                        match operator {
                            Operator::Add => $lhs.checked_add($rhs),
                            Operator::Subtract => $lhs.checked_sub($rhs),
                            Operator::Multiply => $lhs.checked_mul($rhs),
                            Operator::Divide => $lhs.checked_div($rhs),
                            Operator::Remainder => $lhs.checked_rem($rhs),
                            Operator::BitOr => Some($lhs | $rhs),
                            Operator::BitXor => Some($lhs ^ $rhs),
                            Operator::BitAnd => Some($lhs & $rhs),
                            // the amount must be less than the number of bits, while the bits shifted out are lost
                            Operator::ShiftLeft => u32::try_from($rhs).ok().and_then(|rhs| $lhs.checked_shl(rhs)),
                            _ => u32::try_from($rhs).ok().and_then(|rhs| $lhs.checked_shr(rhs)),
                        }
                    };
                }
                let data = match (lhs, rhs) {
                    (Data::Unsigned(lhs), Data::Unsigned(rhs)) => checked!(lhs, rhs).map(Data::Unsigned),
                    (Data::Signed(lhs), Data::Signed(rhs)) => checked!(lhs, rhs).map(Data::Signed),
                    _ => unreachable!(),
                };
                let Some(data) = data else {
                    let shift = matches!(operator, Operator::ShiftLeft | Operator::ShiftRight);
                    self.diagnostics.push(Diagnostic {
                        span: Some(if shift { rhs_span } else { span }),
                        message: if shift { Message::ShiftAmount } else { Message::IntegerOverflow },
                        severity: Severity::Error,
                    });
                    return None;
                };
                data
            },
        };
        Some(Operand { data, span, coerce, origin: None })
    }
    /// Ensure an operand is a boolean, converting it if it permits coercion.
    pub(super) fn bool_operand(&mut self, operand: Operand) -> Option<bool> {
        let value = match &operand.data {
            Data::Bool(value) => Some(*value),
            data if operand.coerce == Coercion::Any => data.coerce_bool(),
            _ => None,
        };
        if value.is_none() {
            self.diagnostics.push(Diagnostic {
                span: Some(operand.span),
                message: Message::InvalidOperand {
                    ty: operand.data.type_name(),
                    expected: "bool",
                    coerce: operand.coerce == Coercion::Any,
                    origin: operand.origin,
                },
                severity: Severity::Error,
            });
        }
        value
    }
    /// Ensure an operand is an integer, converting it if it permits coercion.
    pub(super) fn integer_operand(&mut self, operand: Operand) -> Option<Operand> {
        let data = match &operand.data {
            Data::Unsigned(_) | Data::Signed(_) => return Some(operand),
            data if operand.coerce == Coercion::Any => data.coerce_integer(),
            _ => None,
        };
        let Some(data) = data else {
            self.diagnostics.push(Diagnostic {
                span: Some(operand.span),
                message: Message::InvalidOperand {
                    ty: operand.data.type_name(),
                    expected: "integer",
                    coerce: operand.coerce == Coercion::Any,
                    origin: operand.origin,
                },
                severity: Severity::Error,
            });
            return None;
        };
        Some(Operand { data, ..operand })
    }
    /// Ensure both operands are integers of the same type.
    pub(super) fn integer_operands(&mut self, lhs: Operand, rhs: Operand, span: Span) -> Option<(Data, Data)> {
        let lhs = self.integer_operand(lhs)?;
        let rhs = self.integer_operand(rhs)?;
        self.unify_sign(lhs, rhs, span)
    }
    /// Ensure two integer operands are either both signed or both unsigned.
    pub(super) fn unify_sign(&mut self, lhs: Operand, rhs: Operand, span: Span) -> Option<(Data, Data)> {
        if lhs.data.type_name() == rhs.data.type_name() {
            return Some((lhs.data, rhs.data));
        }
        if let Some(data) = rhs.data.convert_sign() && rhs.coerce >= Coercion::Sign {
            return Some((lhs.data, data));
        }
        if let Some(data) = lhs.data.convert_sign() && lhs.coerce >= Coercion::Sign {
            return Some((data, rhs.data));
        }
        self.diagnostics.push(Diagnostic {
            span: Some(span),
            message: Message::MismatchedOperands {
                lhs: lhs.data.type_name(),
                rhs: rhs.data.type_name(),
                lhs_origin: lhs.origin,
                rhs_origin: rhs.origin,
            },
            severity: Severity::Error,
        });
        None
    }
    /// Ensure both operands have the same type, of any kind.
    pub(super) fn comparable_operands(&mut self, lhs: Operand, rhs: Operand, span: Span) -> Option<(Data, Data)> {
        if lhs.data.type_name() == rhs.data.type_name() {
            return Some((lhs.data, rhs.data));
        }
        if lhs.data.is_integer() && rhs.data.is_integer() {
            return self.unify_sign(lhs, rhs, span);
        }
        let (from, to, swap) = if rhs.coerce == Coercion::Any {
            (rhs, lhs, false)
        } else if lhs.coerce == Coercion::Any {
            (lhs, rhs, true)
        } else {
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::MismatchedOperands {
                    lhs: lhs.data.type_name(),
                    rhs: rhs.data.type_name(),
                    lhs_origin: lhs.origin,
                    rhs_origin: rhs.origin,
                },
                severity: Severity::Error,
            });
            return None;
        };
        let Some(data) = from.data.coerce_like(&to.data) else {
            self.diagnostics.push(Diagnostic {
                span: Some(from.span),
                message: Message::InvalidOperand {
                    ty: from.data.type_name(),
                    expected: to.data.type_name(),
                    coerce: true,
                    origin: from.origin,
                },
                severity: Severity::Error,
            });
            return None;
        };
        Some(if swap { (data, to.data) } else { (to.data, data) })
    }
    /// Ensure an operand is a list.
    pub(super) fn list_operand(&mut self, operand: Operand) -> Option<Vec<Data>> {
        match operand.data {
            Data::List(items) => Some(items),
            data => {
                self.diagnostics.push(Diagnostic {
                    span: Some(operand.span),
                    message: Message::InvalidOperand {
                        ty: data.type_name(),
                        expected: "list",
                        coerce: false,
                        origin: operand.origin,
                    },
                    severity: Severity::Error,
                });
                None
            },
        }
    }
    /// Ensure an operand is a string, converting it if it permits coercion.
    pub(super) fn str_operand(&mut self, operand: Operand) -> Option<String> {
        match operand.data {
            Data::Str(string) => Some(string),
            data if operand.coerce == Coercion::Any && data != Data::None => Some(data.coerce_string()),
            data => {
                self.diagnostics.push(Diagnostic {
                    span: Some(operand.span),
                    message: Message::InvalidOperand {
                        ty: data.type_name(),
                        expected: "str",
                        coerce: operand.coerce == Coercion::Any,
                        origin: operand.origin,
                    },
                    severity: Severity::Error,
                });
                None
            },
        }
    }
    /// Take the next string from the parser.
    pub(super) fn parse_string(&mut self, parser: &mut Parser) -> Option<(String, Span)> {
        if parser.as_str().starts_with("'''\n") {
            return self.parse_multiline_string(parser);
        }
        let Some((string, span)) = parser.string() else {
            self.diagnostics.push(Diagnostic {
                span: Some(parser.span_until(|c| c == ' ' || c == '\n')),
                message: Message::ExpectedString,
                severity: Severity::Error,
            });
            return None;
        };
        Some((self.evaluate_str(string, span.file, span.offset + 1), span))
    }
    /// Take a multi-line string, which starts with `'''` at the end of a line, and ends with `'''` at the
    /// indentation of that line.
    ///
    /// Each line between must be indented by 4 spaces more than the line it starts on, which is removed, or be blank.
    /// A comment, with `#` in the first column, is skipped.
    /// The lines are templates, like other strings, and are joined by newlines, without one at the end.
    pub(super) fn parse_multiline_string(&mut self, parser: &mut Parser) -> Option<(String, Span)> {
        let start = parser.offset;
        let line_start = parser.data[..start].rfind('\n').map_or(0, |i| i + 1);
        let indent = parser.data[line_start..start].len() - parser.data[line_start..start].trim_start_matches(' ').len();
        let content_indent = " ".repeat(indent + 4);
        let closing = format!("{}'''", " ".repeat(indent));
        let span = Span { file: parser.file, offset: start, len: 3 };
        parser.offset += 4;
        let mut lines = Vec::new();
        let mut valid = true;
        loop {
            let rest = parser.as_str();
            let len = rest.find('\n').unwrap_or(rest.len());
            let line = &rest[..len];
            if line.starts_with(&closing) {
                parser.offset += closing.len();
                break;
            }
            // a comment in the first column is not part of the string
            if line.starts_with('#') {
                parser.offset += len + 1;
                continue;
            }
            let spaces = line.len() - line.trim_start_matches(' ').len();
            // a line which is not indented beyond the start is not part of the string, which was not ended
            if rest.is_empty() || !line.trim().is_empty() && spaces <= indent {
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::UnterminatedString,
                    severity: Severity::Error,
                });
                // the end of the previous line, so that skipping the rest of the statement stops at this one
                if !rest.is_empty() {
                    parser.offset -= 1;
                }
                return None;
            }
            if line.trim().is_empty() {
                lines.push(("", parser.offset));
            } else if let Some(content) = line.strip_prefix(&content_indent) {
                lines.push((content, parser.offset + content_indent.len()));
            } else {
                self.diagnostics.push(Diagnostic {
                    span: Some(Span { file: parser.file, offset: parser.offset, len: spaces.max(1) }),
                    message: Message::StringIndentation { expected: indent + 4 },
                    severity: Severity::Error,
                });
                valid = false;
            }
            parser.offset += (len + 1).min(rest.len());
        }
        if !valid {
            return None;
        }
        let lines: Vec<_> = lines.iter().map(|(line, offset)| self.evaluate_str(line, parser.file, *offset)).collect();
        Some((lines.join("\n"), span))
    }
    /// Evaluate string templates.
    ///
    /// An expression in `{}` is replaced with its value, while `{{` and `}}` are replaced with a single brace.
    /// `offset` is the offset of `s` within the file.
    pub(super) fn evaluate_str(&mut self, s: &str, file: usize, offset: usize) -> String {
        let mut string = String::new();
        let mut rest = s;
        while let Some(i) = rest.find(['{', '}']) {
            string.push_str(&rest[..i]);
            let brace = &rest[i..i + 1];
            let after = &rest[i + 1..];
            if after.starts_with(brace) {
                string.push_str(brace);
                rest = &after[1..];
                continue;
            }
            // the offset of `after` within the file
            let offset = offset + s.len() - after.len();
            if brace == "}" {
                self.diagnostics.push(Diagnostic {
                    span: Some(Span { file, offset: offset - 1, len: 1 }),
                    message: Message::InvalidStringTemplate,
                    severity: Severity::Error,
                });
                rest = after;
                continue;
            }
            let Some(end) = expression_end(after) else {
                self.diagnostics.push(Diagnostic {
                    span: Some(Span { file, offset: offset - 1, len: after.len() + 1 }),
                    message: Message::InvalidStringTemplate,
                    severity: Severity::Error,
                });
                return string;
            };
            let first_diagnostic = self.diagnostics.len();
            let mut parser = Parser::new(file, &after[..end]);
            match self.parse_expression(&mut parser) {
                Some(operand) if parser.is_empty() => string.push_str(&operand.data.coerce_string()),
                Some(_) => self.diagnostics.push(Diagnostic {
                    span: Some(parser.span_until(|_| false)),
                    message: Message::InvalidStringTemplate,
                    severity: Severity::Error,
                }),
                None => (),
            }
            // the parser's spans are relative to the expression
            for diagnostic in &mut self.diagnostics[first_diagnostic..] {
                if let Some(span) = &mut diagnostic.span {
                    span.offset += offset;
                }
            }
            rest = &after[end + 1..];
        }
        string.push_str(rest);
        string
    }
}
