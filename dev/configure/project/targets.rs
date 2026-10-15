//! Targets declared by `target` statements.

use super::diagnostic::{Diagnostic, Message, Severity, suggest};
use super::syntax::{Parser, Span};
use super::value::Data;
use super::Project;

/// The attributes of a `target` statement which replace a field of its specification, with the field and the type of
/// its value, where `features` is a `str` of features separated by `,`, or a list of them.
pub(super) const TARGET_OPTIONS: &[(&str, &str, &str)] = &[
    ("env", "env", "str"),
    ("os", "os", "str"),
    ("arch", "arch", "str"),
    ("linker", "linker", "str"),
    // stable codegen options
    ("code-model", "code-model", "str"),
    ("relocation-model", "relocation-model", "str"),
    ("target-cpu", "cpu", "str"),
    ("target-feature", "features", "features"),
    ("linker-flavor", "linker-flavor", "str"),
    ("panic", "panic-strategy", "str"),
    ("no-redzone", "disable-redzone", "bool"),
    // the linker script, rather than its path
    ("link-script", "link-script", "str"),
];

/// A target declared by a `target` statement.
pub struct CustomTarget {
    pub name: String,
    pub name_span: Span,
    /// Whether this sets the attributes of the builtin target of the same name, rather than declaring a target with its
    /// own specification.
    pub overrides: bool,
    /// The builtin target whose specification is extended.
    pub inherit: Option<String>,
    /// Fields of the target specification, in order, which replace those inherited.
    pub fields: Vec<(String, Data)>,
    /// Whether crates for the target can be tested.
    pub test: bool,
    /// The index of the runner of a test harness for the target, which is its last argument, or none to run it
    /// directly.
    pub runner: Option<usize>,
}

impl Project {
    /// Take a custom target, whose attributes are `inherit`, the builtin target whose specification it is based on, and
    /// the fields of its specification which may be replaced.
    ///
    /// Besides `env`, `os`, `arch` and `linker`, the fields are those with stable codegen options, named after the
    /// options rather than the fields.
    pub(super) fn parse_target(&mut self, parser: &mut Parser) {
        if !self.parse_space(parser) {
            parser.skip_statement();
            return;
        }
        let Some((name, span, _)) = self.parse_name(parser) else {
            parser.skip_statement();
            return;
        };
        if !self.parse_line_end(parser) {
            parser.skip_statement();
        }
        let mut inherit = None;
        let mut fields: Vec<(String, Data)> = Vec::new();
        let mut test = false;
        let mut runner = None;
        let mut overrides = false;
        // the attributes which describe the specification, which a builtin target has already
        let mut spec_attributes = Vec::new();
        while parser.indent().is_some() {
            let Some(attribute) = self.parse_attribute(parser, &[]) else {
                parser.skip_line();
                continue;
            };
            if !matches!(attribute.name, "override" | "test" | "runner") {
                spec_attributes.push((attribute.name, attribute.name_span));
            }
            match (attribute.name, attribute.data) {
                ("override", Data::None) => overrides = true,
                ("inherit", Data::Str(base)) => {
                    if self.targets.contains(&base) {
                        inherit = Some(base);
                    } else {
                        self.diagnostics.push(Diagnostic {
                            span: Some(attribute.data_span),
                            message: Message::UnknownTarget {
                                suggestion: suggest(&base, &self.targets),
                                target: base,
                            },
                            severity: Severity::Error,
                        });
                    }
                },
                // tests are only run where they are allowed, and without a runner, they are run directly
                ("test", Data::None) => test = true,
                ("test", Data::Bool(value)) => test = value,
                ("runner", Data::Str(program)) => runner = Some(self.add_runner(vec![program], parser.file)),
                ("runner", Data::List(items)) if !items.is_empty() && items.iter().all(|item| matches!(item, Data::Str(_))) => {
                    runner = Some(self.add_runner(items.iter().map(Data::coerce_string).collect(), parser.file));
                },
                (name @ ("inherit" | "test" | "runner" | "override"), data) => self.diagnostics.push(Diagnostic {
                    span: Some(attribute.data_span),
                    message: Message::InvalidOperand {
                        ty: data.type_name(),
                        expected: match name {
                            "test" => "bool",
                            "runner" => "str or a list of str",
                            "override" => "none",
                            _ => "str",
                        },
                        coerce: false,
                        origin: attribute.origin,
                    },
                    severity: Severity::Error,
                }),
                (option, data) => {
                    let Some(&(_, field, ty)) = TARGET_OPTIONS.iter().find(|(name, ..)| *name == option) else {
                        self.diagnostics.push(Diagnostic {
                            span: Some(attribute.name_span),
                            message: Message::UnknownAttribute { statement: "target", name: option.into() },
                            severity: Severity::Error,
                        });
                        continue;
                    };
                    let data = match (ty, data) {
                        // a flag without a value is set
                        ("bool", Data::None) => Data::Bool(true),
                        // features may be given separately
                        ("features", Data::List(items)) if items.iter().all(|item| matches!(item, Data::Str(_))) => {
                            let features: Vec<_> = items.iter().map(Data::coerce_string).collect();
                            Data::Str(features.join(","))
                        },
                        ("str" | "features", data @ Data::Str(_)) | ("bool", data @ Data::Bool(_)) => data,
                        (ty, data) => {
                            self.diagnostics.push(Diagnostic {
                                span: Some(attribute.data_span),
                                message: Message::InvalidOperand {
                                    ty: data.type_name(),
                                    expected: if ty == "bool" { "bool" } else { "str" },
                                    coerce: false,
                                    origin: attribute.origin,
                                },
                                severity: Severity::Error,
                            });
                            continue;
                        },
                    };
                    match fields.iter_mut().find(|(name, _)| name == field) {
                        Some((_, value)) => *value = data,
                        None => fields.push((field.into(), data)),
                    }
                },
            }
        }
        if overrides {
            // a builtin target is only given the attributes which do not describe its specification
            for (attribute, attribute_span) in spec_attributes {
                self.diagnostics.push(Diagnostic {
                    span: Some(attribute_span),
                    message: Message::OverrideAttribute(attribute.into()),
                    severity: Severity::Error,
                });
            }
            if !self.targets.contains(&name) {
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::UnknownTarget { suggestion: suggest(&name, &self.targets), target: name },
                    severity: Severity::Error,
                });
                return;
            }
        } else if self.targets.contains(&name) {
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::BuiltinTargetName(name),
                severity: Severity::Error,
            });
            return;
        }
        if let Some(previous) = self.custom_targets.iter().find(|target| target.name == name) {
            let previous = previous.name_span;
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::RedeclareCustom { statement: "target", name, previous },
                severity: Severity::Error,
            });
            return;
        }
        self.custom_targets.push(CustomTarget { name, name_span: span, overrides, inherit, fields, test, runner });
    }
    /// Report a target which is neither builtin nor declared, listing the builtin targets the first time.
    pub(super) fn check_target(&mut self, target: &str, span: Span) {
        if self.targets.iter().any(|t| t == target) || self.custom_targets.iter().any(|t| t.name == target) {
            return;
        }
        self.diagnostics.push(Diagnostic {
            span: Some(span),
            message: Message::UnknownTarget {
                target: target.into(),
                suggestion: suggest(target, &self.targets),
            },
            severity: Severity::Error,
        });
        if !self.targets_listed {
            self.targets_listed = true;
            self.diagnostics.push(Diagnostic {
                span: None,
                message: Message::AvailableTargets(self.targets.clone()),
                severity: Severity::Note,
            });
        }
    }
    /// Add the runner of a target, returning its index.
    fn add_runner(&mut self, command: Vec<String>, file: usize) -> usize {
        let runner = self.resolve_command(command, file);
        self.runners.push(runner);
        self.runners.len() - 1
    }
}
