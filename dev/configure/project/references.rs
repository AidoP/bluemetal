//! References to crates, and to the attributes of what has been declared.

use std::path::PathBuf;

use super::diagnostic::{Diagnostic, Message, Severity};
use super::syntax::{Parser, Span, is_attribute_access};
use super::value::{Coercion, Data, Operand};
use super::customs::CustomKind;
use super::targets::target_option;
use super::Project;

impl Project {
    /// Take the rest of a reference to a crate, `<target>/<name>`, after its target, returning the path of its output.
    ///
    /// An attribute of the crate can be taken instead with `.`, as in `<target>/<name>.crate-type`.
    pub(super) fn parse_crate_reference(&mut self, parser: &mut Parser, target: String, start: usize) -> Option<Operand> {
        parser.expect('/');
        let (name, _) = self.parse_ident(parser)?;
        let span = parser.span_from(start);
        let Some(value) = self.value(&format!("{target}/{name}"), span.file) else {
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::UnknownCrate { name: name.into(), target },
                severity: Severity::Error,
            });
            return None;
        };
        let operand = Operand { data: value.data.clone(), span, coerce: Coercion::Strict, origin: value.origin };
        if !is_attribute_access(parser) {
            return Some(operand);
        }
        let (attribute, attribute_span) = self.parse_attribute_access(parser)?;
        let c = &self.crates[&(name.to_string(), target)];
        let value = match attribute {
            "target" => AttributeValue::Set(Data::Str(c.target.clone())),
            "crate-type" => AttributeValue::Set(Data::Str(c.kind.crate_type().into())),
            "path" => AttributeValue::Set(Data::Str(c.path.display().to_string())),
            "builtin" => AttributeValue::Set(Data::Bool(c.builtin)),
            "default" => AttributeValue::Set(Data::Bool(c.default)),
            "test" => AttributeValue::Set(Data::Bool(c.test)),
            "use" => {
                let mut names = Vec::new();
                for used in &c.uses {
                    names.push(Data::Str(used.name.clone()));
                }
                AttributeValue::Set(Data::List(names))
            },
            _ => AttributeValue::Unknown,
        };
        self.attribute_value("crate", name, attribute, attribute_span, value, start, parser)
    }
    /// Take the name of an attribute after a `.`, which is at the parser.
    fn parse_attribute_access<'a>(&mut self, parser: &mut Parser<'a>) -> Option<(&'a str, Span)> {
        parser.expect('.');
        self.parse_ident(parser)
    }
    /// Returns the value of an attribute as an operand, or reports an attribute which is unknown or not set.
    fn attribute_value(
        &mut self,
        kind: &'static str,
        name: &str,
        attribute: &str,
        attribute_span: Span,
        value: AttributeValue,
        start: usize,
        parser: &Parser,
    ) -> Option<Operand> {
        let message = match value {
            AttributeValue::Set(data) => {
                return Some(Operand { data, span: parser.span_from(start), coerce: Coercion::Strict, origin: None });
            },
            AttributeValue::Unset => Message::AttributeNotSet { kind, name: name.into(), attribute: attribute.into() },
            AttributeValue::Unknown => {
                Message::UnknownAttributeOf { kind, name: name.into(), attribute: attribute.into() }
            },
        };
        self.diagnostics.push(Diagnostic { span: Some(attribute_span), message, severity: Severity::Error });
        None
    }
    /// Take the attribute of a custom command, a property, a declared target or a test, after its name.
    ///
    /// A custom command has a property with its name, so it is looked for first, while a test has its own names, so it
    /// is looked for last.
    pub(super) fn parse_named_attribute(&mut self, parser: &mut Parser, name: &str, span: Span, start: usize) -> Option<Operand> {
        let (attribute, attribute_span) = self.parse_attribute_access(parser)?;
        let custom = self.customs.iter().find(|custom| !custom.is_test() && custom.name == name);
        if let Some(custom) = custom && let CustomKind::Command { outputs, default } = &custom.kind {
            let value = match attribute {
                "input" => AttributeValue::Set(paths(&custom.inputs)),
                "output" => AttributeValue::Set(paths(outputs)),
                "command" => AttributeValue::Set(strings(&custom.command)),
                "default" => AttributeValue::Set(Data::Bool(*default)),
                _ => AttributeValue::Unknown,
            };
            return self.attribute_value("custom command", name, attribute, attribute_span, value, start, parser);
        }
        if let Some(&id) = self.properties_idx.get(name) && let Some(property) = &self.properties[id] {
            let value = match attribute {
                "default" => AttributeValue::optional(property.default.clone()),
                "type" => AttributeValue::Set(Data::Str(property.ty.name().into())),
                "read-only" => AttributeValue::Set(Data::Bool(property.read_only)),
                "info" => AttributeValue::optional(property.info.clone().map(Data::Str)),
                _ => AttributeValue::Unknown,
            };
            return self.attribute_value("property", name, attribute, attribute_span, value, start, parser);
        }
        if let Some(target) = self.custom_targets.iter().find(|target| target.name == name) {
            let value = match attribute {
                "inherit" => AttributeValue::optional(target.inherit.clone().map(Data::Str)),
                "test" => AttributeValue::Set(Data::Bool(target.test)),
                "override" => AttributeValue::Set(Data::Bool(target.overrides)),
                "runner" => match target.runner {
                    Some(runner) => AttributeValue::Set(strings(&self.runners[runner].command)),
                    None => AttributeValue::Unset,
                },
                // a field of the specification, which is named after its option
                option => match target_option(option) {
                    Some(option) => AttributeValue::optional(target.field(option.field).cloned()),
                    None => AttributeValue::Unknown,
                },
            };
            return self.attribute_value("target", name, attribute, attribute_span, value, start, parser);
        }
        // a test has its own name, which is looked for last
        if let Some(test) = self.customs.iter().find(|test| test.is_test() && test.name == name) {
            let value = match attribute {
                "input" => AttributeValue::Set(paths(&test.inputs(&self.runners))),
                "command" => AttributeValue::Set(strings(&test.command(&self.runners))),
                _ => AttributeValue::Unknown,
            };
            return self.attribute_value("test", name, attribute, attribute_span, value, start, parser);
        }
        self.diagnostics.push(Diagnostic {
            span: Some(span),
            message: Message::UnknownReference(name.into()),
            severity: Severity::Error,
        });
        None
    }
}

/// The value of an attribute of something which is referred to.
enum AttributeValue {
    Set(Data),
    /// The attribute is known, but has no value.
    Unset,
    /// There is no attribute with the name.
    Unknown,
}
impl AttributeValue {
    /// The value of an attribute which may not be set.
    fn optional(data: Option<Data>) -> Self {
        match data {
            Some(data) => Self::Set(data),
            None => Self::Unset,
        }
    }
}

/// Returns paths as a list of strings.
fn paths(paths: &[PathBuf]) -> Data {
    let mut items = Vec::new();
    for path in paths {
        items.push(Data::Str(path.display().to_string()));
    }
    Data::List(items)
}

/// Returns strings as a list.
fn strings(strings: &[String]) -> Data {
    let mut items = Vec::new();
    for string in strings {
        items.push(Data::Str(string.clone()));
    }
    Data::List(items)
}
