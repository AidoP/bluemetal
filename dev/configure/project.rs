use std::{collections::{HashMap, hash_map::Entry}, path::{Path, PathBuf}};

mod crates;
mod customs;
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

pub use customs::{Custom, Runner};
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
    /// Add the runner of a target, returning its index.
    fn add_runner(&mut self, command: Vec<String>, file: usize) -> usize {
        let runner = self.resolve_command(command, file);
        self.runners.push(runner);
        self.runners.len() - 1
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
