use std::{collections::{HashMap, hash_map::Entry}, path::{Path, PathBuf}};

mod diagnostic;
mod expressions;
mod functions;
pub mod rust;
mod meta;
mod properties;
mod references;
mod statements;
mod syntax;
mod value;

pub use diagnostic::{Diagnostic, Message, Severity};
pub use syntax::Span;
pub use value::{Data, Property, Type, Value};
use diagnostic::suggest;
use syntax::Parser;

pub struct Project {
    root: PathBuf,
    pub files: Vec<File>,
    /// Files read by expressions, which the configuration depends on as well as `files`.
    pub dependencies: Vec<PathBuf>,
    files_idx: HashMap<PathBuf, usize>,
    properties: Vec<Option<Property>>,
    properties_idx: HashMap<String, usize>,
    values: Vec<Value>,
    values_idx: HashMap<String, usize>,
    pub diagnostics: Vec<Diagnostic>,
    /// The target triples supported by the compiler.
    targets: Vec<String>,
    /// Whether the supported targets have been listed by a diagnostic.
    targets_listed: bool,

    /// Crates keyed by name and target, as a crate may be compiled for several targets.
    pub crates: HashMap<(String, String), rust::Crate>,
    /// Custom build commands, in the order they were declared.
    pub customs: Vec<Custom>,
    /// Targets declared by `target` statements, in the order they were declared.
    pub custom_targets: Vec<CustomTarget>,
    /// The runners of targets, which targets and tests refer to by index.
    pub runners: Vec<Runner>,
}
impl Project {
    pub fn new(root: PathBuf, targets: Vec<String>) -> Self {
        Self {
            root,
            files: Vec::new(),
            dependencies: Vec::new(),
            files_idx: HashMap::new(),
            properties: Vec::new(),
            properties_idx: HashMap::new(),
            values: Vec::new(),
            values_idx: HashMap::new(),
            diagnostics: Vec::new(),
            targets,
            targets_listed: false,
            crates: HashMap::new(),
            customs: Vec::new(),
            custom_targets: Vec::new(),
            runners: Vec::new(),
        }
    }
    pub fn file(&self, idx: usize) -> &File {
        &self.files[idx]
    }
    /// Process a file, unless it has already been processed.
    ///
    /// Returns the index of the file if it has already been processed.
    pub fn parse(&mut self, file: File) -> Option<usize> {
        let idx = self.files.len();
        // a file can be referred to by many paths, so identify it by its canonical path
        let path = std::fs::canonicalize(&file.path).unwrap_or_else(|_| file.path.clone());
        match self.files_idx.entry(path) {
            Entry::Occupied(entry) => return Some(*entry.get()),
            Entry::Vacant(entry) => entry.insert(idx),
        };
        self.files.push(file);
        self.files[idx].active = true;
        self.parse_file(idx);
        self.files[idx].active = false;
        None
    }
    /// Declare a property, returning its ID.
    ///
    /// If `property` is [`None`], the proprty is forward-declared.
    pub fn declare(&mut self, name: String, property: Option<Property>) -> usize {
        match self.properties_idx.entry(name) {
            Entry::Occupied(entry) => {
                let id = *entry.get();
                let Some(property_ref) = &property else {
                    // forward declaration, permit an existing property
                    return id;
                };
                match &mut self.properties[id] {
                    Some(existing) => self.diagnostics.push(Diagnostic {
                        span: property_ref.span,
                        message: Message::RedeclareProperty {
                            id,
                            name: property_ref.name.clone(),
                            builtin: existing.span.is_none(),
                        },
                        severity: Severity::Error,
                    }),
                    value @ None => {
                        *value = property;
                    },
                }
                return id;
            },
            Entry::Vacant(entry) => {
                let id = self.properties.len();
                entry.insert(id);
                self.properties.push(property);
                return id;
            },
        }
    }
    /// Assign a value to a property, returning the ID of the value handle.
    pub fn set(&mut self, name: String, value: Value) -> usize {
        let id = self.values.len();
        if let Some(span) = &value.span {
            let file = &mut self.files[span.file];
            if value.local {
                file.locals.insert(name.clone(), id);
            } else {
                let _ = file.locals.remove(&name);
            }
        }
        self.values.push(value);
        self.values_idx.insert(name, id);
        id
    }
    fn value(&self, name: &str, file: usize) -> Option<&Value> {
        let mut idx = self.files[file].locals.get(name);
        if idx.is_none() {
            idx = self.values_idx.get(name);
        }
        Some(&self.values[*idx?])
    }
    /// Check for various conditions that cannot be evaluated as errors until reading has completed.
    /// - Unknown targets for crates, as a `target` statement may come later.
    /// - Missing declarations for `set` statements.
    /// - Missing crates for `use` attributes.
    pub fn check(&mut self) {
        // in the order they were declared, as the crates are not
        let mut targets: Vec<_> = self.crates.values().map(|c| (c.target.clone(), c.target_span)).collect();
        targets.sort_by_key(|(_, span)| (span.file, span.offset));
        for (target, span) in targets {
            self.check_target(&target, span);
        }
        self.add_crate_tests();
        for (name, &idx) in &self.values_idx {
            let value = &self.values[idx];
            if self.properties[value.property].is_none() {
                self.diagnostics.push(Diagnostic {
                    span: value.span,
                    message: Message::UndeclaredProperty(name.clone()),
                    severity: Severity::Error,
                });
            }
        }
        let host = self.values_idx.get("host-target").map_or_default(|&value| self.values[value].data.coerce_string());
        for c in self.crates.values() {
            for (name, span) in &c.uses {
                // a procedural macro is for the host target, but can be used by a crate for any target
                let used = self.crates.get(&(name.clone(), c.target.clone()))
                    .or_else(|| self.crates.get(&(name.clone(), host.clone())).filter(|d| d.kind == rust::Kind::ProcMacro));
                let message = match used {
                    None => Message::UnknownCrate { name: name.clone(), target: c.target.clone() },
                    Some(d) if d.kind == rust::Kind::Bin => Message::UseBinary { name: name.clone(), declared: d.name_span },
                    Some(_) => continue,
                };
                self.diagnostics.push(Diagnostic { span: Some(*span), message, severity: Severity::Error });
            }
        }
    }
}

impl Project {
    fn parse_crate(&mut self, parser: &mut Parser) {
        if !self.parse_space(parser) {
            parser.skip_statement();
            return;
        }
        let Some((name, span, literal)) = self.parse_name(parser) else {
            parser.skip_statement();
            return;
        };
        // the name is passed to rustc, which rejects `-` even though it is valid in an identifier
        if let Some((i, character)) = name.char_indices().find(|&(_, c)| c == '-') {
            // point at the character if the name was written as an identifier, otherwise its position is unknown
            let span = if literal {
                Span { file: span.file, offset: span.offset + i, len: character.len_utf8() }
            } else {
                span
            };
            parser.skip_statement();
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::InvalidCrateName { name, character },
                severity: Severity::Error,
            });
            return;
        }
        let path;
        let path_span;
        if parser.peek() == Some(' ') && self.parse_space(parser) && let Some((p, s)) = self.parse_path(parser) {
            path = p;
            path_span = s;
        } else {
            let mut rel_path = self.files[parser.file].relative_path(&name);
            rel_path.push("lib.rs");
            path = rel_path;
            path_span = span;
        }
        if !self.parse_line_end(parser) {
            parser.skip_statement();
        }
        let mut builtin = false;
        let mut default = false;
        let mut test = false;
        let mut target = None;
        let mut uses = Vec::new();
        let mut kind = rust::Kind::Lib;
        while let Some(_) = parser.indent() {
            let Some(attribute) = self.parse_attribute(parser, &["use"]) else {
                parser.skip_line();
                continue;
            };
            match attribute.name {
                "builtin" => builtin = true,
                "default" => default = true,
                "test" => test = true,
                "crate-type" => match attribute.data {
                    Data::Str(crate_type) => match crate_type.as_str() {
                        "lib" => kind = rust::Kind::Lib,
                        "bin" => kind = rust::Kind::Bin,
                        "proc-macro" => kind = rust::Kind::ProcMacro,
                        _ => self.diagnostics.push(Diagnostic {
                            span: Some(attribute.data_span),
                            message: Message::InvalidCrateType(crate_type),
                            severity: Severity::Error,
                        }),
                    },
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
                "target" | "use" => match attribute.data {
                    Data::Str(string) if attribute.name == "target" => {
                        target = Some((vec![string], attribute.data_span));
                    },
                    // a crate is declared for each of a list of targets
                    Data::List(items)
                        if attribute.name == "target" && items.iter().all(|item| matches!(item, Data::Str(_))) =>
                    {
                        let targets = items.iter().map(Data::coerce_string).collect();
                        target = Some((targets, attribute.data_span));
                    },
                    Data::Str(string) => uses.push((string, attribute.data_span)),
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
                name => self.diagnostics.push(Diagnostic {
                    span: Some(attribute.name_span),
                    message: Message::UnknownCrateAttribute {
                        name: name.into(),
                    },
                    severity: Severity::Error,
                }),
            }
        }
        // a procedural macro is loaded by rustc, so it is compiled for the host target, which is otherwise the `target`
        // property in effect at its declaration
        let host = self.property("host-target", span).map_or_default(|(data, _)| data.coerce_string());
        let default_target = if kind == rust::Kind::ProcMacro { "host-target" } else { "target" };
        let (targets, target_span) = target.unwrap_or_else(|| {
            (vec![self.property(default_target, span).map_or_default(|(data, _)| data.coerce_string())], span)
        });
        if kind == rust::Kind::ProcMacro && let Some(target) = targets.iter().find(|target| **target != host) {
            self.diagnostics.push(Diagnostic {
                span: Some(target_span),
                message: Message::ProcMacroTarget { target: target.clone(), host },
                severity: Severity::Error,
            });
            return;
        }
        let mut outputs = Vec::new();
        for (i, target) in targets.iter().enumerate() {
            if targets[..i].contains(target) {
                self.diagnostics.push(Diagnostic {
                    span: Some(target_span),
                    message: Message::RepeatedTarget(target.clone()),
                    severity: Severity::Error,
                });
                continue;
            }
            let target = target.clone();
            // whether the tests can run depends on the target, which may be declared later, so they are registered by
            // `check`
            match self.crates.entry((name.clone(), target.clone())) {
                Entry::Occupied(_) => self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::RedeclareCrate {
                        name: name.clone(),
                        target,
                    },
                    severity: Severity::Error,
                }),
                Entry::Vacant(entry) => {
                    let c = entry.insert(rust::Crate {
                        name: name.clone(),
                        name_span: span,
                        path: path.clone(),
                        path_span,
                        target,
                        target_span,
                        kind,
                        output: None,
                        builtin,
                        implicit: false,
                        default,
                        test,
                        uses: uses.clone(),
                    });
                    // a crate is referred to as `<target>/<name>`, as it may be declared for several targets
                    let output = c.output().map(|output| (format!("{}/{}", c.target, c.name), output));
                    outputs.extend(output);
                    // the test is forward declared, so that it can be referred to, and completed once the target is known
                    if test {
                        let name = crate_test_name(c, &host);
                        let harness = c.test_harness();
                        self.add_crate_test(name, span, harness, Vec::new());
                    }
                },
            }
        }
        for (property, output) in outputs {
            self.declare_outputs(property, span, "crate", vec![output]);
        }
    }
    /// Register the test harness of a crate, like a `test` statement which runs it, which also needs `inputs`.
    ///
    /// The harness is run directly, until the runner of its target is known.
    fn add_crate_test(&mut self, name: String, span: Span, harness: PathBuf, mut inputs: Vec<PathBuf>) {
        let command = vec![Path::new(".").join(&harness).display().to_string()];
        inputs.insert(0, harness);
        self.add_custom(Custom {
            name,
            name_span: span,
            inputs,
            outputs: Vec::new(),
            command,
            program: None,
            default: false,
            test: true,
            runner: None,
        });
    }
    /// Complete the tests of the crates with a `test` attribute, which were forward declared, with the runner of their
    /// target. Tests run on the host target, or a custom target which allows tests, and the others are removed.
    fn add_crate_tests(&mut self) {
        let host = self.values_idx.get("host-target").map_or_default(|&value| self.values[value].data.coerce_string());
        // in the order they were declared, as the crates are not
        let mut keys: Vec<_> = self.crates.iter()
            .filter(|(_, c)| c.test && !c.implicit)
            .map(|(key, c)| (key.clone(), (c.name_span.file, c.name_span.offset)))
            .collect();
        keys.sort_by(|(lhs, lhs_position), (rhs, rhs_position)| (lhs_position, lhs).cmp(&(rhs_position, rhs)));
        for (key, _) in keys {
            let c = &self.crates[&key];
            let (name, span, target_span) = (crate_test_name(c, &host), c.name_span, c.target_span);
            // the test which was forward declared, unless another had its name
            let declared = self.customs.iter().position(|test| {
                test.test && test.name == name && (test.name_span.file, test.name_span.offset) == (span.file, span.offset)
            });
            let custom = self.custom_targets.iter().find(|target| target.name == c.target);
            let runner = match custom {
                Some(target) if target.test => target.runner,
                _ if c.target == host => None,
                // an unknown target is reported anyway
                None if !self.targets.contains(&c.target) => continue,
                _ => {
                    self.diagnostics.push(Diagnostic {
                        span: Some(target_span),
                        message: Message::TestTarget { target: c.target.clone(), host: host.clone() },
                        severity: Severity::Error,
                    });
                    // nor is its harness built
                    self.crates.get_mut(&key).unwrap().test = false;
                    if let Some(declared) = declared {
                        self.customs.remove(declared);
                    }
                    continue;
                },
            };
            if let Some(declared) = declared {
                self.customs[declared].runner = runner;
            }
        }
    }
    /// Add a crate which is part of the build system rather than declared by the configuration, after reading it.
    ///
    /// Its test, if it has one, also needs `test_inputs`. A declared crate with the same name and target is reported.
    pub fn add_implicit_crate(&mut self, c: rust::Crate, test_inputs: Vec<PathBuf>) {
        let key = (c.name.clone(), c.target.clone());
        if let Some(declared) = self.crates.get(&key) {
            self.diagnostics.push(Diagnostic {
                span: Some(declared.name_span),
                message: Message::ReservedCrate { name: c.name, target: c.target },
                severity: Severity::Error,
            });
            return;
        }
        if c.test {
            self.add_crate_test(c.name.clone(), c.name_span, c.test_harness(), test_inputs);
        }
        self.crates.insert(key, c);
    }
    /// Take a custom build command, with `input`, `output`, `command` and `default` attributes, or a test, which has
    /// `input` and `command` attributes.
    ///
    /// While its attributes are evaluated, the paths of the inputs and outputs before them are the values of the
    /// `input` and `output` properties, as lists.
    fn parse_custom(&mut self, parser: &mut Parser, test: bool) {
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
    fn resolve_command(&self, mut command: Vec<String>, file: usize) -> Runner {
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
    /// Add the runner of a target, returning its index.
    fn add_runner(&mut self, command: Vec<String>, file: usize) -> usize {
        let runner = self.resolve_command(command, file);
        self.runners.push(runner);
        self.runners.len() - 1
    }
    /// Add a custom build command or a test, unless one of the same kind has the same name.
    fn add_custom(&mut self, custom: Custom) -> bool {
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
    /// Take a custom target, whose attributes are `inherit`, the builtin target whose specification it is based on, and
    /// the fields of its specification which may be replaced.
    ///
    /// Besides `env`, `os`, `arch` and `linker`, the fields are those with stable codegen options, named after the
    /// options rather than the fields.
    fn parse_target(&mut self, parser: &mut Parser) {
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
    fn check_target(&mut self, target: &str, span: Span) {
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
}

/// Remove `.` and resolve `..` in a path without accessing the file system.
///
/// A `..` which cannot be resolved is kept in a relative path, and removed at the root of an absolute path.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => (),
            Component::ParentDir => match normal.components().next_back() {
                Some(Component::Normal(_)) => {
                    normal.pop();
                },
                Some(Component::RootDir | Component::Prefix(_)) => (),
                _ => normal.push(".."),
            },
            component => normal.push(component),
        }
    }
    if normal.as_os_str().is_empty() {
        normal.push(".");
    }
    normal
}

/// The name of the test of a crate, which is the name of the crate, with its target if that is not the host target.
fn crate_test_name(c: &rust::Crate, host: &str) -> String {
    if c.target == host { c.name.clone() } else { format!("{}@{}", c.name, c.target) }
}

pub struct File {
    pub path: PathBuf,
    data: String,
    locals: HashMap<String, usize>,
    /// Whether the file is currently being processed.
    active: bool,
    /// The `use` statement which caused the file to be processed.
    used: Option<Span>,
}
impl File {
    pub fn read(path: std::path::PathBuf) -> std::io::Result<Self> {
        Ok(Self {
            data: std::fs::read_to_string(&path)?,
            path,
            locals: HashMap::new(),
            active: false,
            used: None,
        })
    }
    /// Returns the path relative to the file.
    pub fn relative_path<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        self.path.parent().unwrap().join(path)
    }
    /// Returns the line and column of an offset within the file.
    pub fn position(&self, offset: usize) -> (usize, usize) {
        let before = &self.data[..offset];
        let line = before.matches('\n').count() + 1;
        let col = offset - before.rfind('\n').map_or(0, |i| i + 1) + 1;
        (line, col)
    }
}

/// The attributes of a `target` statement which replace a field of its specification, with the field and the type of
/// its value, where `features` is a `str` of features separated by `,`, or a list of them.
const TARGET_OPTIONS: &[(&str, &str, &str)] = &[
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
