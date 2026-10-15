//! Targets declared by `target` statements.

use super::diagnostic::{Diagnostic, Message, Severity, suggest};
use super::statements::Attribute;
use super::statements::Name;
use super::syntax::{Parser, Span};
use super::value::Data;
use super::Project;

/// An attribute of a `target` statement which replaces a field of its specification.
pub(super) struct TargetOption {
    pub(super) attribute: &'static str,
    /// The field of the specification.
    pub(super) field: &'static str,
    pub(super) ty: OptionType,
}

/// The type of the value of a [`TargetOption`].
#[derive(Clone, Copy)]
pub(super) enum OptionType {
    Str,
    /// A flag, which is set by the attribute without a value.
    Bool,
    /// Features, as a `str` of features separated by `,`, or a list of them.
    Features,
}
impl OptionType {
    /// The type of value expected, as in a diagnostic.
    fn expected(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Str | Self::Features => "str",
        }
    }
}

/// The attributes of a `target` statement which replace a field of its specification.
const TARGET_OPTIONS: &[TargetOption] = &[
    TargetOption { attribute: "env", field: "env", ty: OptionType::Str },
    TargetOption { attribute: "os", field: "os", ty: OptionType::Str },
    TargetOption { attribute: "arch", field: "arch", ty: OptionType::Str },
    TargetOption { attribute: "linker", field: "linker", ty: OptionType::Str },
    // stable codegen options
    TargetOption { attribute: "code-model", field: "code-model", ty: OptionType::Str },
    TargetOption { attribute: "relocation-model", field: "relocation-model", ty: OptionType::Str },
    TargetOption { attribute: "target-cpu", field: "cpu", ty: OptionType::Str },
    TargetOption { attribute: "target-feature", field: "features", ty: OptionType::Features },
    TargetOption { attribute: "linker-flavor", field: "linker-flavor", ty: OptionType::Str },
    TargetOption { attribute: "panic", field: "panic-strategy", ty: OptionType::Str },
    TargetOption { attribute: "no-redzone", field: "disable-redzone", ty: OptionType::Bool },
    // the linker script, rather than its path
    TargetOption { attribute: "link-script", field: "link-script", ty: OptionType::Str },
];

/// Returns the option set by an attribute of a `target` statement, if it sets one.
pub(super) fn target_option(attribute: &str) -> Option<&'static TargetOption> {
    for option in TARGET_OPTIONS {
        if option.attribute == attribute {
            return Some(option);
        }
    }
    None
}

/// A field of a target specification.
pub struct Field {
    pub name: String,
    pub value: Data,
}

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
    pub fields: Vec<Field>,
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
        let Some(Name { name, span, .. }) = self.parse_name(parser) else {
            parser.skip_statement();
            return;
        };
        if !self.parse_line_end(parser) {
            parser.skip_statement();
        }
        let mut inherit = None;
        let mut fields: Vec<Field> = Vec::new();
        let mut test = false;
        let mut runner = None;
        let mut overrides = false;
        // the attributes which describe the specification, which a builtin target has already, are reported if it turns
        // out to be overridden
        let mut override_errors = Vec::new();
        while let Some(attribute) = self.next_attribute(parser, &[]) {
            if !matches!(attribute.name, "override" | "test" | "runner") {
                override_errors.push(Diagnostic {
                    span: Some(attribute.name_span),
                    message: Message::OverrideAttribute(attribute.name.into()),
                    severity: Severity::Error,
                });
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
                (name @ ("inherit" | "test" | "runner" | "override"), data) => {
                    let expected = match name {
                        "test" => "bool",
                        "runner" => "str or a list of str",
                        "override" => "none",
                        _ => "str",
                    };
                    self.invalid_operand(&Attribute { data, ..attribute }.operand(), expected, false);
                },
                (name, data) => {
                    let Some(option) = target_option(name) else {
                        self.diagnostics.push(Diagnostic {
                            span: Some(attribute.name_span),
                            message: Message::UnknownAttribute { statement: "target", name: name.into() },
                            severity: Severity::Error,
                        });
                        continue;
                    };
                    let value = match (option.ty, data) {
                        // a flag without a value is set
                        (OptionType::Bool, Data::None) => Data::Bool(true),
                        // features may be given separately
                        (OptionType::Features, Data::List(items))
                            if items.iter().all(|item| matches!(item, Data::Str(_))) =>
                        {
                            let features: Vec<_> = items.iter().map(Data::coerce_string).collect();
                            Data::Str(features.join(","))
                        },
                        (OptionType::Str | OptionType::Features, data @ Data::Str(_))
                        | (OptionType::Bool, data @ Data::Bool(_)) => data,
                        (ty, data) => {
                            self.invalid_operand(&Attribute { data, ..attribute }.operand(), ty.expected(), false);
                            continue;
                        },
                    };
                    match fields.iter_mut().find(|field| field.name == option.field) {
                        Some(field) => field.value = value,
                        None => fields.push(Field { name: option.field.into(), value }),
                    }
                },
            }
        }
        if overrides {
            // a builtin target is only given the attributes which do not describe its specification
            self.diagnostics.extend(override_errors);
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
