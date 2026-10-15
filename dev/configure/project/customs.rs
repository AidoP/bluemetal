//! Custom build commands and tests, which run a command.

use std::path::{Path, PathBuf};

use super::diagnostic::{Diagnostic, Item, Message, Severity};
use super::statements::Name;
use super::syntax::{Parser, Span};
use super::value::Data;
use super::{Project, normalize};

/// A command, with its program if that is a path, which it depends on.
#[derive(Clone, Default)]
pub struct Runner {
    pub command: Vec<String>,
    pub program: Option<PathBuf>,
}

/// A custom build command, or a test.
pub struct Custom {
    pub name: String,
    pub name_span: Span,
    pub inputs: Vec<PathBuf>,
    /// The program and its arguments, where a program with a `/` is relative to the project root.
    pub command: Vec<String>,
    /// The program, if it is a path, which the outputs depend on.
    pub program: Option<PathBuf>,
    pub kind: CustomKind,
}

/// Whether a [`Custom`] is a build command or a test, with what only that kind has.
pub enum CustomKind {
    /// A build command, which ninja runs to build its outputs.
    Command {
        outputs: Vec<PathBuf>,
        /// Whether the outputs are built by `ninja` without targets.
        default: bool,
    },
    /// A test, which has no outputs and is run by `configure --test` rather than ninja.
    Test {
        /// The index of the runner of the test's target, which runs its command, as with the test harness of a crate.
        runner: Option<usize>,
    },
}

impl Custom {
    pub fn is_test(&self) -> bool {
        matches!(self.kind, CustomKind::Test { .. })
    }
    /// The index of the runner of a test, if it has one.
    fn runner(&self) -> Option<usize> {
        match self.kind {
            CustomKind::Test { runner } => runner,
            CustomKind::Command { .. } => None,
        }
    }
    /// The command which is run, by the runner, if there is one.
    pub fn command(&self, runners: &[Runner]) -> Vec<String> {
        let Some(runner) = self.runner() else {
            return self.command.clone();
        };
        let mut command = runners[runner].command.clone();
        command.extend(self.command.iter().cloned());
        command
    }
    /// The inputs, which include the program of the runner, if there is one.
    pub fn inputs(&self, runners: &[Runner]) -> Vec<PathBuf> {
        let mut inputs = self.inputs.clone();
        if let Some(runner) = self.runner() && let Some(program) = &runners[runner].program {
            inputs.push(program.clone());
        }
        inputs
    }
}

impl Project {
    /// Take a custom build command, with `input`, `output`, `command` and `default` attributes, or a test, which has
    /// `input` and `command` attributes.
    ///
    /// While its attributes are evaluated, the paths of the inputs and outputs before them are the values of the
    /// `input` and `output` properties, as lists.
    pub(super) fn parse_custom(&mut self, parser: &mut Parser, test: bool) {
        let item = if test { Item::Test } else { Item::CustomCommand };
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
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        let mut command: Option<Vec<String>> = None;
        let mut default = false;
        // an attribute which failed has been reported, so it is not also reported as missing
        let mut failed = false;
        while parser.indent().is_some() {
            let Some((attribute, attribute_span)) = self.parse_ident(parser) else {
                parser.skip_line();
                continue;
            };
            match attribute {
                "input" | "output" if attribute == "input" || !test => {
                    let Some(new) = self.parse_custom_paths(parser) else {
                        parser.skip_line();
                        failed = true;
                        continue;
                    };
                    let paths = if attribute == "input" { &mut inputs } else { &mut outputs };
                    paths.extend(new);
                    let value = paths.iter().map(|path: &PathBuf| Data::Str(path.display().to_string())).collect();
                    self.set_local(attribute, Data::List(value), parser.file);
                },
                "default" if !test => default = true,
                "command" => {
                    let Some(arguments) = self.parse_command(parser) else {
                        parser.skip_line();
                        failed = true;
                        continue;
                    };
                    if command.replace(arguments).is_some() {
                        self.diagnostics.push(Diagnostic {
                            span: Some(attribute_span),
                            message: Message::RepeatedAttribute(attribute.into()),
                            severity: Severity::Error,
                        });
                    }
                },
                name => {
                    parser.skip_line();
                    self.diagnostics.push(Diagnostic {
                        span: Some(attribute_span),
                        message: Message::UnknownAttribute { item, name: name.into() },
                        severity: Severity::Error,
                    });
                    continue;
                },
            }
            if !self.parse_line_end(parser) {
                parser.skip_line();
            }
        }
        for local in ["input", "output"] {
            self.files[parser.file].locals.remove(local);
        }
        let missing = if outputs.is_empty() && !test {
            Some("output")
        } else if command.is_none() {
            Some("command")
        } else {
            None
        };
        if let Some(attribute) = missing {
            if failed {
                return;
            }
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::MissingAttribute(attribute),
                severity: Severity::Error,
            });
            return;
        }
        let runner = self.resolve_command(command.unwrap_or_default(), parser.file);
        // the outputs are declared as a property named after the command, once it is known to be unique
        let declared = name.clone();
        let declared_outputs = outputs.clone();
        let kind = if test { CustomKind::Test { runner: None } } else { CustomKind::Command { outputs, default } };
        let custom = Custom { name, name_span: span, inputs, command: runner.command, program: runner.program, kind };
        if self.add_custom(custom) {
            self.declare_outputs(declared, span, Item::CustomCommand, declared_outputs);
        }
    }
    /// Returns a command whose program, as in `run`, is a path relative to a file if it has a `/`, and which it then
    /// depends on, while otherwise it is searched for in `PATH`.
    pub(super) fn resolve_command(&self, mut command: Vec<String>, file: usize) -> Runner {
        let mut program = None;
        if command[0].contains('/') {
            let mut path = normalize(&self.file(file).relative_path(&command[0]));
            program = Some(path.clone());
            if path.is_relative() {
                path = Path::new(".").join(path);
            }
            command[0] = path.display().to_string();
        }
        Runner { command, program }
    }
    /// Add a custom build command or a test, unless one of the same kind has the same name.
    pub(super) fn add_custom(&mut self, custom: Custom) -> bool {
        let previous = self.customs.iter().find(|previous| previous.name == custom.name && previous.is_test() == custom.is_test());
        if let Some(previous) = previous {
            let previous = previous.name_span;
            self.diagnostics.push(Diagnostic {
                span: Some(custom.name_span),
                message: Message::RedeclareCustom {
                    item: if custom.is_test() { Item::Test } else { Item::CustomCommand },
                    name: custom.name,
                    previous,
                },
                severity: Severity::Error,
            });
            return false;
        }
        self.customs.push(custom);
        true
    }
    /// Take the paths of an `input` or `output` attribute, a path or a list of them, which are relative to the file.
    fn parse_custom_paths(&mut self, parser: &mut Parser) -> Option<Vec<PathBuf>> {
        if !self.parse_space(parser) {
            return None;
        }
        let operand = self.parse_expression(parser)?;
        let paths = match operand.data {
            Data::List(items) => items.iter().map(Data::coerce_string).collect(),
            _ => vec![self.str_operand(operand)?],
        };
        Some(paths.iter().map(|path| normalize(&self.file(parser.file).relative_path(path))).collect())
    }
    /// Take the arguments of a `command` attribute, which are expressions separated by spaces.
    ///
    /// Any value is an argument, converted to a string as in a string template, while each item of a list is a separate
    /// argument.
    fn parse_command(&mut self, parser: &mut Parser) -> Option<Vec<String>> {
        let mut arguments = Vec::new();
        while parser.peek() == Some(' ') || arguments.is_empty() {
            if !self.parse_space(parser) {
                return None;
            }
            let operand = self.parse_expression(parser)?;
            for argument in operand.data.spread() {
                // an argument is written to `build.ninja` on a single line
                if argument.contains('\n') {
                    self.diagnostics.push(Diagnostic {
                        span: Some(operand.span),
                        message: Message::NewlineInArgument,
                        severity: Severity::Error,
                    });
                    return None;
                }
                arguments.push(argument);
            }
        }
        Some(arguments)
    }
}
