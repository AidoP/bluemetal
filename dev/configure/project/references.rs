//! References to crates, and to the attributes of what has been declared.

use std::path::PathBuf;

use super::diagnostic::{Diagnostic, Message, Severity};
use super::syntax::{Parser, Span, is_attribute_access};
use super::value::{Coercion, Data, Operand};
use super::{Project, TARGET_OPTIONS};

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
        let data = match attribute {
            "target" => Some(Data::Str(c.target.clone())),
            "crate-type" => Some(Data::Str(c.crate_type().into())),
            "path" => Some(Data::Str(c.path.display().to_string())),
            "builtin" => Some(Data::Bool(c.builtin)),
            "default" => Some(Data::Bool(c.default)),
            "test" => Some(Data::Bool(c.test)),
            "use" => Some(Data::List(c.uses.iter().map(|(name, _)| Data::Str(name.clone())).collect())),
            _ => None,
        };
        self.attribute_value("crate", name, attribute, attribute_span, data.map(Some), start, parser)
    }
    /// Take the name of an attribute after a `.`, which is at the parser.
    fn parse_attribute_access<'a>(&mut self, parser: &mut Parser<'a>) -> Option<(&'a str, Span)> {
        parser.expect('.');
        self.parse_ident(parser)
    }
    /// Returns the value of an attribute, where `data` is [`None`] for an unknown attribute, or [`Some`] of [`None`] for
    /// one which is not set.
    fn attribute_value(
        &mut self,
        kind: &'static str,
        name: &str,
        attribute: &str,
        attribute_span: Span,
        data: Option<Option<Data>>,
        start: usize,
        parser: &Parser,
    ) -> Option<Operand> {
        let message = match data {
            Some(Some(data)) => {
                return Some(Operand { data, span: parser.span_from(start), coerce: Coercion::Strict, origin: None });
            },
            Some(None) => Message::AttributeNotSet { kind, name: name.into(), attribute: attribute.into() },
            None => Message::UnknownAttributeOf { kind, name: name.into(), attribute: attribute.into() },
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
        if let Some(custom) = self.customs.iter().find(|custom| !custom.test && custom.name == name) {
            let paths = |paths: &[PathBuf]| Data::List(paths.iter().map(|path| Data::Str(path.display().to_string())).collect());
            let data = match attribute {
                "input" => Some(Some(paths(&custom.inputs))),
                "output" => Some(Some(paths(&custom.outputs))),
                "command" => Some(Some(Data::List(custom.command.iter().map(|s| Data::Str(s.clone())).collect()))),
                "default" => Some(Some(Data::Bool(custom.default))),
                _ => None,
            };
            return self.attribute_value("custom command", name, attribute, attribute_span, data, start, parser);
        }
        if let Some(property) = self.properties_idx.get(name).and_then(|&id| self.properties[id].as_ref()) {
            let data = match attribute {
                "default" => Some(property.default.clone()),
                "type" => Some(Some(Data::Str(property.ty.name().into()))),
                "read-only" => Some(Some(Data::Bool(property.read_only))),
                "info" => Some(property.info.clone().map(Data::Str)),
                _ => None,
            };
            return self.attribute_value("property", name, attribute, attribute_span, data, start, parser);
        }
        let Some(target) = self.custom_targets.iter().find(|target| target.name == name) else {
            // a test has its own name, which is looked for last
            if let Some(test) = self.customs.iter().find(|test| test.test && test.name == name) {
                let strings = |strings: Vec<String>| Data::List(strings.into_iter().map(Data::Str).collect());
                let data = match attribute {
                    "input" => {
                        let inputs = test.inputs(&self.runners).iter().map(|path| path.display().to_string()).collect();
                        Some(Some(strings(inputs)))
                    },
                    "command" => Some(Some(strings(test.command(&self.runners)))),
                    _ => None,
                };
                return self.attribute_value("test", name, attribute, attribute_span, data, start, parser);
            }
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::UnknownReference(name.into()),
                severity: Severity::Error,
            });
            return None;
        };
        let strings = |strings: &[String]| Data::List(strings.iter().map(|s| Data::Str(s.clone())).collect());
        let data = match attribute {
            "inherit" => Some(target.inherit.clone().map(Data::Str)),
            "test" => Some(Some(Data::Bool(target.test))),
            "override" => Some(Some(Data::Bool(target.overrides))),
            "runner" => Some(target.runner.map(|runner| strings(&self.runners[runner].command))),
            // a field of the specification, which is named after its option
            option => TARGET_OPTIONS.iter().find(|(name, ..)| *name == option).map(|&(_, field, _)| {
                target.fields.iter().find(|(name, _)| name == field).map(|(_, data)| data.clone())
            }),
        };
        self.attribute_value("target", name, attribute, attribute_span, data, start, parser)
    }
}
