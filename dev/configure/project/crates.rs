//! Crates, and the tests of their test harnesses.

use std::collections::hash_map::Entry;
use std::path::{Path, PathBuf};

use super::diagnostic::{Diagnostic, Message, Severity};
use super::syntax::{Parser, Span};
use super::value::Data;
use super::customs::{Custom, CustomKind};
use super::{Project, rust};

/// The name of the test of a crate, which is the name of the crate, with its target if that is not the host target.
fn crate_test_name(c: &rust::Crate, host: &str) -> String {
    if c.target == host { c.name.clone() } else { format!("{}@{}", c.name, c.target) }
}

impl Project {
    pub(super) fn parse_crate(&mut self, parser: &mut Parser) {
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
        let has_path = parser.peek() == Some(' ') && self.parse_space(parser);
        let path = if has_path && let Some((path, _)) = self.parse_path(parser) {
            path
        } else {
            self.files[parser.file].relative_path(&name).join("lib.rs")
        };
        if !self.parse_line_end(parser) {
            parser.skip_statement();
        }
        let mut builtin = false;
        let mut default = false;
        let mut test = false;
        let mut target = None;
        let mut uses = Vec::new();
        let mut kind = rust::Kind::Lib;
        while let Some(attribute) = self.next_attribute(parser, &["use"]) {
            match attribute.name {
                "builtin" => builtin = true,
                "default" => default = true,
                "test" => test = true,
                "crate-type" => {
                    let span = attribute.data_span;
                    let Some(crate_type) = self.str_operand(attribute.operand()) else {
                        continue;
                    };
                    match crate_type.as_str() {
                        "lib" => kind = rust::Kind::Lib,
                        "bin" => kind = rust::Kind::Bin,
                        "proc-macro" => kind = rust::Kind::ProcMacro,
                        _ => self.diagnostics.push(Diagnostic {
                            span: Some(span),
                            message: Message::InvalidCrateType(crate_type),
                            severity: Severity::Error,
                        }),
                    }
                },
                "target" => match attribute.data {
                    Data::Str(string) => target = Some((vec![string], attribute.data_span)),
                    // a crate is declared for each of a list of targets
                    Data::List(items) if items.iter().all(|item| matches!(item, Data::Str(_))) => {
                        let targets = items.iter().map(Data::coerce_string).collect();
                        target = Some((targets, attribute.data_span));
                    },
                    _ => self.invalid_operand(&attribute.operand(), "str", false),
                },
                "use" => {
                    let span = attribute.data_span;
                    if let Some(name) = self.str_operand(attribute.operand()) {
                        uses.push(rust::Use { name, span });
                    }
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
            command,
            program: None,
            kind: CustomKind::Test { runner: None },
        });
    }
    /// Complete the tests of the crates with a `test` attribute, which were forward declared, with the runner of their
    /// target. Tests run on the host target, or a custom target which allows tests, and the others are removed.
    pub(super) fn add_crate_tests(&mut self) {
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
                test.is_test() && test.name == name && (test.name_span.file, test.name_span.offset) == (span.file, span.offset)
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
                self.customs[declared].kind = CustomKind::Test { runner };
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
}
