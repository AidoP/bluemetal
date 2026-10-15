//! Custom build commands and tests, which run a command.

use std::path::{Path, PathBuf};

use super::diagnostic::{Diagnostic, Message, Severity};
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
    pub outputs: Vec<PathBuf>,
    /// The program and its arguments, where a program with a `/` is relative to the project root.
    pub command: Vec<String>,
    /// The program, if it is a path, which the outputs depend on.
    pub program: Option<PathBuf>,
    /// Whether the outputs are built by `ninja` without targets.
    pub default: bool,
    /// Whether this is a test, which has no outputs and is run by `configure --test` rather than ninja.
    pub test: bool,
    /// The index of the runner of the test's target, which runs its command, as with the test harness of a crate.
    pub runner: Option<usize>,
}
impl Custom {
    /// The command which is run, by the runner, if there is one.
    pub fn command(&self, runners: &[Runner]) -> Vec<String> {
        match self.runner {
            Some(runner) => runners[runner].command.iter().chain(&self.command).cloned().collect(),
            None => self.command.clone(),
        }
    }
    /// The inputs, which include the program of the runner, if there is one.
    pub fn inputs(&self, runners: &[Runner]) -> Vec<PathBuf> {
        let program = self.runner.and_then(|runner| runners[runner].program.clone());
        self.inputs.iter().cloned().chain(program).collect()
    }
}

impl Project {
    /// Take a custom build command, with `input`, `output`, `command` and `default` attributes, or a test, which has
    /// `input` and `command` attributes.
    ///
    /// While its attributes are evaluated, the paths of the inputs and outputs before them are the values of the
    /// `input` and `output` properties, as lists.
    pub(super) fn parse_custom(&mut self, parser: &mut Parser, test: bool) {
        let statement = if test { "test" } else { "custom" };
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
                        message: Message::UnknownAttribute { statement, name: name.into() },
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
        let Runner { command, program } = self.resolve_command(command.unwrap_or_default(), parser.file);
        let declared = (name.clone(), outputs.clone());
        let custom = Custom { name, name_span: span, inputs, outputs, command, program, default, test, runner: None };
        if self.add_custom(custom) {
            self.declare_outputs(declared.0, span, "custom command", declared.1);
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
        let previous = self.customs.iter().find(|previous| previous.name == custom.name && previous.test == custom.test);
        if let Some(previous) = previous {
            let previous = previous.name_span;
            self.diagnostics.push(Diagnostic {
                span: Some(custom.name_span),
                message: Message::RedeclareCustom {
                    statement: if custom.test { "test" } else { "custom" },
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
