//! Declaring properties and giving them values.

use std::path::PathBuf;

use super::diagnostic::{Diagnostic, Message, Severity};
use super::syntax::{Parser, Span};
use super::value::{Data, Property, Type, Value};
use super::Project;

impl Project {
    pub(super) fn parse_declare(&mut self, parser: &mut Parser) {
        if !self.parse_space(parser) {
            parser.skip_statement();
            return;
        }
        let Some((name, span, _)) = self.parse_name(parser) else {
            parser.skip_statement();
            return;
        };
        let ident = name.as_str();
        let idx = self.declare(ident.into(), None);
        if self.properties[idx].is_some() {
            // skip the attributes too, so that a duplicate cannot change the property or its value
            parser.skip_statement();
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::RedeclareProperty {
                    id: idx,
                    name: ident.into(),
                    builtin: self.properties[idx].as_ref().is_some_and(|property| property.span.is_none()),
                },
                severity: Severity::Error,
            });
            return;
        }
        if !self.parse_line_end(parser) {
            parser.skip_statement();
        }
        // values set before the declaration, which are errors if the property is read-only and already has a value
        let set_before: Vec<_> = (0..self.values.len()).filter(|&value| self.values[value].property == idx).collect();
        let mut ty = Type::Any;
        let mut read_only = false;
        let mut default = None;
        // the default is assigned immediately so that later attributes can use it, but its type is only checked once the
        // whole declaration is known
        let mut default_value = None;
        let mut info = None;
        while let Some(_) = parser.indent() {
            let Some(attribute) = self.parse_attribute(parser, &[]) else {
                parser.skip_line();
                continue;
            };
            match attribute.name {
                "type" => {
                    let name = attribute.data.coerce_string();
                    match name.as_str() {
                        "str" => ty = Type::String,
                        "bool" => ty = Type::Bool,
                        "unsigned" => ty = Type::Unsigned,
                        "signed" => ty = Type::Signed,
                        "list" => ty = Type::List,
                        _ => self.diagnostics.push(Diagnostic {
                            span: Some(attribute.data_span),
                            message: Message::InvalidType(name),
                            severity: Severity::Error,
                        }),
                    }
                },
                "read-only" => read_only = true,
                "info" => match attribute.data {
                    Data::Str(string) => info = Some(string),
                    data => self.diagnostics.push(Diagnostic {
                        span: Some(attribute.data_span),
                        message: Message::InvalidOperand {
                            ty: data.type_name(),
                            expected: "str",
                            coerce: false,
                            origin: attribute.origin,
                        },
                        severity: Severity::Error,
                    }),
                },
                "default" => {
                    default = Some(attribute.name_span);
                    default_value = Some(self.set(ident.into(), Value {
                        property: idx,
                        span: Some(attribute.name_span),
                        data: attribute.data,
                        data_span: Some(attribute.data_span),
                        origin: attribute.origin.or(Some(attribute.data_span)),
                        local: false,
                    }));
                }
                name => self.diagnostics.push(Diagnostic {
                    span: Some(attribute.name_span),
                    message: Message::UnknownDeclarationAttribute {
                        property: idx,
                        name: name.into(),
                    },
                    severity: Severity::Error,
                }),
            }
        }
        self.properties[idx] = Some(Property {
            span: Some(span),
            name: ident.into(),
            ty,
            read_only,
            info,
            default: None,
        });
        // values set before the declaration could not be checked against its type, so remove any which do not match
        let mut valid_before = Vec::new();
        for value in set_before {
            if self.check_type(value, ident) {
                valid_before.push(value);
            }
        }
        let valid_default = default_value.filter(|&value| self.check_type(value, ident));
        if valid_default.is_none() {
            default = None;
        }
        if let Some(value) = valid_default {
            self.properties[idx].as_mut().unwrap().default = Some(self.values[value].data.clone());
        }
        match valid_default.or(valid_before.last().copied()) {
            Some(value) => self.values_idx.insert(ident.into(), value),
            None => self.values_idx.remove(ident),
        };
        if !read_only {
            return;
        }
        // a default leaves no room for any earlier `set`, otherwise only the first is permitted
        let (assigned, invalid) = match default {
            Some(span) => (Some(span), &valid_before[..]),
            None => match valid_before.split_first() {
                Some((first, rest)) => {
                    // the rejected values were set before they were known to be invalid, so restore the first
                    self.values_idx.insert(ident.into(), *first);
                    (self.values[*first].span, rest)
                },
                None => return,
            },
        };
        for &value in invalid {
            self.diagnostics.push(Diagnostic {
                span: self.values[value].span,
                message: Message::ReadOnlyProperty { id: idx, name: ident.into(), assigned, builtin: false },
                severity: Severity::Error,
            });
        }
    }
    /// Returns the span of the value of a read-only property which already has a value.
    ///
    /// A read-only property may be assigned once, by a `default` or a `set`, so further assignments are errors.
    /// The span is [`None`] for a builtin value.
    pub(super) fn read_only_value(&self, id: usize, name: &str) -> Option<Option<Span>> {
        if !self.properties[id].as_ref().is_some_and(|property| property.read_only) {
            return None;
        }
        let value = self.values_idx.get(name)?;
        Some(self.values[*value].span)
    }
    pub(super) fn parse_set(&mut self, parser: &mut Parser) {
        if !self.parse_space(parser) {
            parser.skip_statement();
            return;
        }
        let Some((name, span, _)) = self.parse_name(parser) else {
            parser.skip_statement();
            return;
        };
        let ident = name.as_str();
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
        let property = self.declare(ident.into(), None);
        if let Some(assigned) = self.read_only_value(property, ident) {
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::ReadOnlyProperty {
                    id: property,
                    name: ident.into(),
                    assigned,
                    builtin: self.properties[property].as_ref().is_some_and(|property| property.span.is_none()),
                },
                severity: Severity::Error,
            });
            return;
        }
        let value = Value {
            property,
            span: Some(span),
            data: operand.data,
            data_span: Some(operand.span),
            origin: operand.origin.or(Some(operand.span)),
            local: false,
        };
        self.assign(ident, value);
    }
    /// Assign a value to a property, unless it does not have the property's type, returning whether it was assigned.
    ///
    /// The type of a property which has not been declared yet is checked by its declaration.
    pub(super) fn assign(&mut self, name: &str, value: Value) -> bool {
        let Some(property) = &self.properties[value.property] else {
            self.set(name.into(), value);
            return true;
        };
        match property.ty.accept(value.data) {
            Ok(data) => {
                self.set(name.into(), Value { data, ..value });
                true
            },
            Err(data) => {
                // keep the value for the diagnostic, but not as the property's value
                self.values.push(Value { data, ..value });
                self.mismatched_type(self.values.len() - 1, name);
                false
            },
        }
    }
    /// Check that an assigned value has the type of its property, converting it if needed, and report it if not.
    pub(super) fn check_type(&mut self, value: usize, name: &str) -> bool {
        let property = self.properties[self.values[value].property].as_ref().unwrap();
        match property.ty.accept(self.values[value].data.clone()) {
            Ok(data) => {
                self.values[value].data = data;
                true
            },
            Err(_) => {
                self.mismatched_type(value, name);
                false
            },
        }
    }
    /// Report a value which does not have the type of its property.
    pub(super) fn mismatched_type(&mut self, value: usize, name: &str) {
        self.diagnostics.push(Diagnostic {
            span: self.values[value].data_span,
            message: Message::MismatchedType { value, name: name.into() },
            severity: Severity::Error,
        });
    }
    /// Declare a read-only property whose value is the path of an output, or a list of them, which is how crates and
    /// custom commands are referred to.
    pub(super) fn declare_outputs(&mut self, name: String, span: Span, statement: &'static str, outputs: Vec<PathBuf>) {
        if outputs.is_empty() {
            return;
        }
        let mut items: Vec<_> = outputs.iter().map(|path| Data::Str(path.display().to_string())).collect();
        let property = Property {
            span: Some(span),
            name: name.clone(),
            ty: Type::Any,
            read_only: true,
            info: Some(format!("the output of the {statement}")),
            default: None,
        };
        let reported = self.diagnostics.len();
        let id = self.declare(name.clone(), Some(property));
        // a property with the name has been declared already
        if self.diagnostics.len() > reported {
            return;
        }
        let data = if items.len() == 1 { items.remove(0) } else { Data::List(items) };
        self.set(name, Value { property: id, span: Some(span), data, data_span: Some(span), origin: Some(span), local: false });
    }
    /// Give a property a value until the end of the statement in a file, without declaring it.
    pub(super) fn set_local(&mut self, name: &str, data: Data, file: usize) {
        let id = self.values.len();
        // the value is not a property's, so it has no declaration
        self.values.push(Value { property: usize::MAX, span: None, data, data_span: None, origin: None, local: true });
        self.files[file].locals.insert(name.into(), id);
    }
    /// Returns the value of a property referenced by an expression, and where the value was set.
    pub(super) fn property(&mut self, name: &str, span: Span) -> Option<(Data, Option<Span>)> {
        let Some(value) = self.value(name, span.file) else {
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::UnsetProperty(name.into()),
                severity: Severity::Error,
            });
            return None;
        };
        Some((value.data.clone(), value.origin))
    }
}
