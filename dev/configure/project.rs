use std::{collections::{HashMap, hash_map::Entry}, path::{Path, PathBuf}};

pub mod rust;
mod syntax;
mod value;

pub use syntax::Span;
pub use value::{Data, Property, Type, Value};
use syntax::{
    Operator, Parser, argument_end, expression_end, invalid_ident_char, is_attribute_access, is_crate_reference,
};
use value::{Coercion, Operand};

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
    /// Write a diagnostic in a human readable form.
    pub fn write_diagnostic(&self, w: &mut dyn std::io::Write, diagnostic: &Diagnostic, pretty: bool) -> std::io::Result<()> {
        let severity = diagnostic.severity;
        let mut style_accent = "";
        let mut style_white = "";
        let mut style_note = "";
        let mut style_reset = "";
        if pretty {
            style_accent = severity.accent();
            style_white = "\x1b[97m";
            style_note = Severity::Note.accent();
            style_reset = "\x1b[0m";
        }

        // the lines of a multi-line message, such as the output of a program, are aligned with the first
        let message = diagnostic.message.to_string();
        let mut lines = message.lines();
        writeln!(w, "{style_accent}{severity}{style_white}: {}{style_reset}", lines.next().unwrap_or_default())?;
        let indent = " ".repeat(severity.to_string().len() + 2);
        for line in lines {
            if line.is_empty() {
                writeln!(w)?;
            } else {
                writeln!(w, "{indent}{style_white}{line}{style_reset}")?;
            }
        }
        if let Some(span) = diagnostic.span.as_ref() {
            match &diagnostic.message {
                // the line is shown with the replacement
                Message::Replacement { replacement, .. } => {
                    self.write_replacement(w, span, replacement, style_accent, style_reset)?;
                },
                _ => self.write_location(w, span, style_accent, style_reset)?,
            }
        }
        match &diagnostic.message {
            Message::RedeclareProperty { id, .. } => {
                if let Some(span) = &self.properties[*id].as_ref().unwrap().span {
                    writeln!(w, "{style_note}note{style_white}: previously declared here{style_reset}")?;
                    self.write_location(w, span, style_note, style_reset)?;
                }
            },
            Message::ReadOnlyProperty { id, assigned, .. } => {
                if let Some(span) = assigned {
                    writeln!(w, "{style_note}note{style_white}: value assigned here{style_reset}")?;
                    self.write_location(w, span, style_note, style_reset)?;
                }
                if let Some(span) = &self.properties[*id].as_ref().unwrap().span {
                    writeln!(w, "{style_note}note{style_white}: declared read-only here{style_reset}")?;
                    self.write_location(w, span, style_note, style_reset)?;
                }
            },
            Message::UseBinary { declared, .. } | Message::UseWithTarget { declared: Some(declared) } => {
                writeln!(w, "{style_note}note{style_white}: declared here{style_reset}")?;
                self.write_location(w, declared, style_note, style_reset)?;
            },
            Message::RedeclareCustom { previous, .. } => {
                writeln!(w, "{style_note}note{style_white}: previously declared here{style_reset}")?;
                self.write_location(w, previous, style_note, style_reset)?;
            },
            Message::RepeatedUse { file, .. } => {
                if let Some(span) = &self.files[*file].used {
                    writeln!(w, "{style_note}note{style_white}: first used here{style_reset}")?;
                    self.write_location(w, span, style_note, style_reset)?;
                }
            },
            Message::RedeclareCrate { name, target } => {
                let span = &self.crates[&(name.clone(), target.clone())].name_span;
                writeln!(w, "{style_note}note{style_white}: previously declared here{style_reset}")?;
                self.write_location(w, span, style_note, style_reset)?;
            },
            Message::UnknownDeclarationAttribute { property, .. } => {
                let property = self.properties[*property].as_ref().unwrap();
                if let Some(span) = &property.span {
                    writeln!(w, "{style_note}note{style_white}: attribute specified for declaration of {:?}{style_reset}", property.name)?;
                    self.write_location(w, span, style_note, style_reset)?;
                }
            },
            Message::RunFailed { stderr, .. } if !stderr.is_empty() => {
                // shown like a location, with a gutter as wide as the one before it
                let width = diagnostic.span.map_or(1, |span| {
                    self.files[span.file].position(span.offset).0.to_string().len()
                });
                let gutter = " ".repeat(width);
                writeln!(w, "{gutter}--> stderr")?;
                writeln!(w, "{gutter} |")?;
                for line in stderr.lines() {
                    if line.is_empty() {
                        writeln!(w, "{gutter} |")?;
                    } else {
                        writeln!(w, "{gutter} | {line}")?;
                    }
                }
                writeln!(w, "{gutter} |")?;
            },
            Message::InvalidOperand { ty, origin: Some(span), .. } => {
                writeln!(w, "{style_note}note{style_white}: {ty} value set here{style_reset}")?;
                self.write_location(w, span, style_note, style_reset)?;
            },
            Message::MismatchedOperands { lhs, rhs, lhs_origin, rhs_origin } => {
                for (ty, origin) in [(lhs, lhs_origin), (rhs, rhs_origin)] {
                    if let Some(span) = origin {
                        writeln!(w, "{style_note}note{style_white}: {ty} value set here{style_reset}")?;
                        self.write_location(w, span, style_note, style_reset)?;
                    }
                }
            },
            Message::MismatchedType { value, .. } => {
                let value = &self.values[*value];
                // the value of a referenced property was set elsewhere
                if let Some(origin) = value.origin && value.data_span.is_some_and(|span| span.offset != origin.offset || span.file != origin.file) {
                    writeln!(w, "{style_note}note{style_white}: {} value set here{style_reset}", value.data.type_name())?;
                    self.write_location(w, &origin, style_note, style_reset)?;
                }
                if let Some(span) = &self.properties[value.property].as_ref().unwrap().span {
                    writeln!(w, "{style_note}note{style_white}: declared here{style_reset}")?;
                    self.write_location(w, span, style_note, style_reset)?;
                }
            },
            _ => (),
        }
        if let Some(property) = self.message_property(&diagnostic.message) && let Some(info) = &property.info {
            writeln!(w, "{style_note}note{style_white}: {}: {info}{style_reset}", property.name)?;
        }
        Ok(())
    }
    /// Returns the property a message is about, if it has been declared.
    fn message_property(&self, message: &Message) -> Option<&Property> {
        let id = match message {
            Message::RedeclareProperty { id, .. } | Message::ReadOnlyProperty { id, .. } => *id,
            Message::UnknownDeclarationAttribute { property, .. } => *property,
            Message::MismatchedType { value, .. } => self.values[*value].property,
            Message::UnsetProperty(name) => *self.properties_idx.get(name)?,
            _ => return None,
        };
        self.properties[id].as_ref()
    }
    /// Write the location of a span followed by its first line with the span underlined.
    /// Write the location of a span followed by its first line with the span replaced, and the replacement underlined.
    fn write_replacement(
        &self,
        w: &mut dyn std::io::Write,
        span: &Span,
        replacement: &str,
        style_accent: &str,
        style_reset: &str,
    ) -> std::io::Result<()> {
        let file = &self.files[span.file];
        let (line, col) = file.position(span.offset);
        let line = line.to_string();
        let gutter = " ".repeat(line.len());
        writeln!(w, "{gutter}--> {}:{line}:{col}", file.path.display())?;

        let start = span.offset - (col - 1);
        let rest = &file.data[start..];
        let text = rest[..rest.find('\n').unwrap_or(rest.len())].trim_end_matches('\r');
        let (before, after) = text.split_at((col - 1).min(text.len()));
        let after = &after[span.len.min(after.len())..];
        let indent = " ".repeat(before.chars().count());
        let width = replacement.chars().count().max(1);
        writeln!(w, "{gutter} |")?;
        writeln!(w, "{line} | {before}{replacement}{after}")?;
        writeln!(w, "{gutter} | {indent}{style_accent}{}{style_reset}", "~".repeat(width))
    }
    fn write_location(&self, w: &mut dyn std::io::Write, span: &Span, style_accent: &str, style_reset: &str) -> std::io::Result<()> {
        let file = &self.files[span.file];
        let (line, col) = file.position(span.offset);
        let line = line.to_string();
        let gutter = " ".repeat(line.len());
        writeln!(w, "{gutter}--> {}:{line}:{col}", file.path.display())?;

        let start = span.offset - (col - 1);
        let rest = &file.data[start..];
        let text = rest[..rest.find('\n').unwrap_or(rest.len())].trim_end_matches('\r');
        let (before, after) = text.split_at((col - 1).min(text.len()));
        let indent = " ".repeat(before.len());
        let end = span.len.min(after.len());
        let width = after[..end].chars().count().max(1);
        writeln!(w, "{gutter} |")?;
        writeln!(w, "{line} | {text}")?;
        writeln!(w, "{gutter} | {indent}{style_accent}{}{style_reset}", "^".repeat(width))
    }
}

impl Project {
    fn parse_file(&mut self, idx: usize) {
        let data = std::mem::take(&mut self.files[idx].data);
        let mut parser = Parser::new(idx, &data);
        let mut chain = None;
        loop {
            parser.skip_empty();
            if parser.is_empty() {
                break;
            }
            self.parse_statement(&mut parser, &mut chain);
        }
        self.files[idx].data = data;
    }
    /// Take a statement, with the meta-statements before it, and the statements chained after it with `@then`, which
    /// are processed or skipped with it.
    fn parse_statement(&mut self, parser: &mut Parser, chain: &mut Option<bool>) {
        let process = match self.parse_meta(parser, chain) {
            Meta::Skip => false,
            // `@for` processes the chained statements too
            Meta::Done => return,
            Meta::Process => true,
        };
        if process {
            self.parse_single_statement(parser);
        } else {
            parser.skip_statement();
        }
        while self.parse_then(parser) {
            if process {
                self.parse_statement(parser, &mut None);
            } else {
                Self::skip_chained(parser);
            }
        }
    }
    /// Take a `@then` directly after a statement, returning whether there is one.
    fn parse_then(&mut self, parser: &mut Parser) -> bool {
        let Some(rest) = parser.as_str().strip_prefix("@then") else {
            return false;
        };
        if !rest.is_empty() && !rest.starts_with(['\n', ' ']) {
            return false;
        }
        let span = Span { file: parser.file, offset: parser.offset, len: 5 };
        parser.offset += 5;
        if !self.parse_line_end(parser) {
            parser.skip_line();
        }
        if matches!(parser.peek(), None | Some('\n' | '#')) {
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::ExpectedStatementAfterMeta,
                severity: Severity::Error,
            });
            return false;
        }
        true
    }
    /// Skip a statement chained with `@then` to a skipped statement, with its meta-statements.
    fn skip_chained(parser: &mut Parser) {
        while parser.peek() == Some('@') {
            parser.skip_line();
        }
        parser.skip_statement();
    }
    /// Take a statement, after its meta-statements.
    fn parse_single_statement(&mut self, parser: &mut Parser) {
        let start = parser.offset;
        let Some((keyword, span)) = self.parse_ident(parser) else {
            parser.skip_statement();
            return;
        };
        match keyword {
            "declare" => self.parse_declare(parser),
            "use" => self.parse_use(parser),
            "set" => self.parse_set(parser),
            "log" => self.parse_log(parser),
            "crate" => self.parse_crate(parser),
            "custom" => self.parse_custom(parser, false),
            "test" => self.parse_custom(parser, true),
            "target" => self.parse_target(parser),
            _ => {
                parser.offset = start;
                let _ = parser.skip_statement();
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::UnknownStatement(keyword.into()),
                    severity: Severity::Error,
                });
            },
        }
    }
    /// Take the meta-statements which precede a statement, returning whether the statement should be processed.
    ///
    /// Each meta-statement is a line starting with `@`, and must be followed directly by another meta-statement or by
    /// the statement it applies to. Once the statement is known to be skipped, later conditions are not evaluated.
    ///
    /// `chain` is whether a statement has been processed in the current chain of conditional statements, which `@else`
    /// continues, or [`None`] if the previous statement was unconditional.
    ///
    /// `@for` processes the meta-statements and statement after it for each value, leaving nothing more to process.
    fn parse_meta(&mut self, parser: &mut Parser, chain: &mut Option<bool>) -> Meta {
        let mut active = true;
        // whether the statement is conditional, so that it starts or continues a chain
        let mut conditional = false;
        // whether the chain had already processed a statement, if this statement continues it with `@else`
        let mut continues = None;
        // an invalid meta-statement skips the statement, and any `@else` after it
        let mut failed = false;
        let mut first = true;
        while parser.peek() == Some('@') {
            let start = parser.offset;
            parser.expect('@');
            let Some((name, span)) = self.parse_ident(parser) else {
                parser.skip_line();
                failed = true;
                continue;
            };
            let meta_span = Span { file: span.file, offset: start, len: span.offset + span.len - start };
            match name {
                "if" => {
                    conditional = true;
                    if !active {
                        parser.skip_line();
                    } else if let Some(condition) = self.parse_condition(parser) {
                        active = condition;
                    } else {
                        failed = true;
                    }
                },
                // a skipped statement is not repeated
                "for" if !active || failed => {
                    parser.skip_line();
                },
                "for" => {
                    let Some((name, values)) = self.parse_for(parser) else {
                        failed = true;
                        continue;
                    };
                    if matches!(parser.peek(), None | Some('\n' | '#')) {
                        self.diagnostics.push(Diagnostic {
                            span: Some(meta_span),
                            message: Message::ExpectedStatementAfterMeta,
                            severity: Severity::Error,
                        });
                        *chain = None;
                        return Meta::Skip;
                    }
                    *chain = match continues {
                        Some(_) => Some(true),
                        None if conditional => Some(true),
                        None => None,
                    };
                    let body = parser.offset;
                    let previous = self.files[parser.file].locals.get(&name).copied();
                    for value in &values {
                        parser.offset = body;
                        self.set_local(&name, value.clone(), parser.file);
                        self.parse_statement(parser, &mut None);
                    }
                    match previous {
                        Some(previous) => self.files[parser.file].locals.insert(name, previous),
                        None => self.files[parser.file].locals.remove(&name),
                    };
                    if !values.is_empty() {
                        return Meta::Done;
                    }
                    // without any values, the rest is skipped
                    while parser.peek() == Some('@') {
                        parser.skip_line();
                    }
                    return Meta::Skip;
                },
                // `@then` is taken with the statement before it
                "then" => {
                    parser.skip_line();
                    self.diagnostics.push(Diagnostic {
                        span: Some(meta_span),
                        message: Message::ThenWithoutStatement,
                        severity: Severity::Error,
                    });
                    failed = true;
                },
                "else" | "else-if" => {
                    conditional = true;
                    match *chain {
                        Some(processed) if first => {
                            continues = Some(processed);
                            active &= !processed;
                        },
                        _ => {
                            let message = if first {
                                Message::ElseWithoutIf(name.into())
                            } else {
                                Message::ElseNotFirst(name.into())
                            };
                            self.diagnostics.push(Diagnostic {
                                span: Some(meta_span),
                                message,
                                severity: Severity::Error,
                            });
                            failed = true;
                        },
                    }
                    if name == "else" {
                        if !self.parse_line_end(parser) {
                            parser.skip_line();
                            failed = true;
                        }
                    } else if !active || failed {
                        parser.skip_line();
                    } else if let Some(condition) = self.parse_condition(parser) {
                        active = condition;
                    } else {
                        failed = true;
                    }
                },
                _ => {
                    parser.skip_line();
                    self.diagnostics.push(Diagnostic {
                        span: Some(meta_span),
                        message: Message::UnknownMetaStatement(name.into()),
                        severity: Severity::Error,
                    });
                    failed = true;
                },
            }
            first = false;
            if matches!(parser.peek(), None | Some('\n' | '#')) {
                self.diagnostics.push(Diagnostic {
                    span: Some(meta_span),
                    message: Message::ExpectedStatementAfterMeta,
                    severity: Severity::Error,
                });
                *chain = None;
                return Meta::Skip;
            }
        }
        if failed {
            *chain = Some(true);
            return Meta::Skip;
        }
        *chain = match continues {
            Some(processed) => Some(processed || active),
            None if conditional => Some(active),
            None => None,
        };
        if active { Meta::Process } else { Meta::Skip }
    }
    /// Take the rest of a `@for`, a property and the values it takes, separated by spaces, where each item of a list is
    /// a value.
    fn parse_for(&mut self, parser: &mut Parser) -> Option<(String, Vec<Data>)> {
        let header = (|| {
            if !self.parse_space(parser) {
                return None;
            }
            let (name, _, _) = self.parse_name(parser)?;
            if !self.parse_space(parser) {
                return None;
            }
            if !matches!(parser.as_str().strip_prefix("in"), Some(rest) if rest.starts_with([' ', '\n'])) {
                self.diagnostics.push(Diagnostic {
                    span: Some(parser.span_until(|c| c == ' ' || c == '\n')),
                    message: Message::ExpectedExact("in"),
                    severity: Severity::Error,
                });
                return None;
            }
            parser.offset += 2;
            let mut values = Vec::new();
            loop {
                if !self.parse_space(parser) {
                    return None;
                }
                match self.parse_expression(parser)?.data {
                    Data::List(items) => values.extend(items),
                    data => values.push(data),
                }
                if parser.peek() != Some(' ') {
                    break;
                }
            }
            self.parse_line_end(parser).then_some((name, values))
        })();
        if header.is_none() {
            parser.skip_line();
        }
        header
    }
    /// Take the condition of an `@if` to the end of the line, returning [`None`] if it is invalid.
    fn parse_condition(&mut self, parser: &mut Parser) -> Option<bool> {
        if !self.parse_space(parser) {
            parser.skip_line();
            return None;
        }
        let Some(operand) = self.parse_binary(parser, 0) else {
            parser.skip_line();
            return None;
        };
        if !self.parse_line_end(parser) {
            parser.skip_line();
            return None;
        }
        self.bool_operand(operand)
    }
    fn parse_use(&mut self, parser: &mut Parser) {
        if !self.parse_space(parser) {
            parser.skip_statement();
            return;
        }
        let Some((mut path, span)) = self.parse_path(parser) else {
            parser.skip_statement();
            return;
        };
        if !self.parse_line_end(parser) {
            parser.skip_statement();
        }
        path = path.join("Build");
        match File::read(path.clone()) {
            Ok(mut file) => {
                file.used = Some(span);
                // using a file again is fine unless it is still being processed, which is a cycle
                if let Some(idx) = self.parse(file) && self.files[idx].active {
                    self.diagnostics.push(Diagnostic {
                        span: Some(span),
                        message: Message::RepeatedUse { path, file: idx },
                        severity: Severity::Warning,
                    });
                }
            },
            Err(error) => {
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::ReadFailure(path, error),
                    severity: Severity::Error,
                });
            },
        };
    }
    fn parse_declare(&mut self, parser: &mut Parser) {
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
    fn read_only_value(&self, id: usize, name: &str) -> Option<Option<Span>> {
        if !self.properties[id].as_ref().is_some_and(|property| property.read_only) {
            return None;
        }
        let value = self.values_idx.get(name)?;
        Some(self.values[*value].span)
    }
    fn parse_set(&mut self, parser: &mut Parser) {
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
    fn assign(&mut self, name: &str, value: Value) -> bool {
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
    fn check_type(&mut self, value: usize, name: &str) -> bool {
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
    fn mismatched_type(&mut self, value: usize, name: &str) {
        self.diagnostics.push(Diagnostic {
            span: self.values[value].data_span,
            message: Message::MismatchedType { value, name: name.into() },
            severity: Severity::Error,
        });
    }
    fn parse_log(&mut self, parser: &mut Parser) {
        if !self.parse_space(parser) {
            parser.skip_statement();
            return;
        }
        let Some((severity, span)) = self.parse_ident(parser) else {
            parser.skip_statement();
            return;
        };
        let severity = match severity {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            "note" => Severity::Note,
            "suggestion" => Severity::Suggestion,
            _ => {
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::UnknownSeverity(severity.into()),
                    severity: Severity::Error,
                });
                Severity::Error
            },
        };
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
        // any value can be logged, as it would be in a string template
        self.diagnostics.push(Diagnostic {
            span: Some(operand.span),
            message: Message::User(operand.data.coerce_string()),
            severity,
        });
    }
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
    /// Declare a read-only property whose value is the path of an output, or a list of them, which is how crates and
    /// custom commands are referred to.
    fn declare_outputs(&mut self, name: String, span: Span, statement: &'static str, outputs: Vec<PathBuf>) {
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
    /// Give a property a value until the end of the statement in a file, without declaring it.
    fn set_local(&mut self, name: &str, data: Data, file: usize) {
        let id = self.values.len();
        // the value is not a property's, so it has no declaration
        self.values.push(Value { property: usize::MAX, span: None, data, data_span: None, origin: None, local: true });
        self.files[file].locals.insert(name.into(), id);
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
    /// Take the next identifier from the parser.
    ///
    /// If the parser is not pointing at an identifier, a diagnostic is created, naming the first invalid character if
    /// the parser is pointing at a word.
    fn parse_ident<'a>(&mut self, parser: &mut Parser<'a>) -> Option<(&'a str, Span)> {
        let rest = parser.as_str();
        let word = &rest[..rest.find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-').unwrap_or(rest.len())];
        let start = parser.offset;
        if let Some((ident, span)) = parser.ident() && ident.len() == word.len() {
            return Some((ident, span));
        }
        parser.offset = start;
        let (span, message) = match word.char_indices().find(invalid_ident_char) {
            Some((i, character)) => (
                Span { file: parser.file, offset: start + i, len: character.len_utf8() },
                Message::InvalidIdentifier { name: word.into(), character },
            ),
            None => (parser.span_until(|c| c == ' ' || c == '\n'), Message::ExpectedIdentifier),
        };
        self.diagnostics.push(Diagnostic {
            span: Some(span),
            message,
            severity: Severity::Error,
        });
        None
    }
    /// Take the name of a property or crate, which is an identifier, or an expression in `{}` whose value is one.
    ///
    /// Also returns whether the name was written as an identifier, so that its characters are at known offsets.
    fn parse_name(&mut self, parser: &mut Parser) -> Option<(String, Span, bool)> {
        if parser.peek() != Some('{') {
            let (ident, span) = self.parse_ident(parser)?;
            return Some((ident.into(), span, true));
        }
        let operand = self.parse_operand(parser)?;
        let name = operand.data.coerce_string();
        let message = match name.char_indices().find(invalid_ident_char) {
            Some((_, character)) => Message::InvalidIdentifier { name, character },
            None if name.is_empty() => Message::ExpectedIdentifier,
            None => return Some((name, operand.span, false)),
        };
        self.diagnostics.push(Diagnostic {
            span: Some(operand.span),
            message,
            severity: Severity::Error,
        });
        None
    }
    /// Takes the next space from the parser and returns true.
    /// If the parser is not pointing at a space, a diagnostic is created and false is returned.
    fn parse_space(&mut self, parser: &mut Parser) -> bool {
        if parser.expect(' ') {
            true
        } else {
            self.diagnostics.push(Diagnostic {
                span: Some(parser.span_char()),
                message: Message::ExpectedExact(" "),
                severity: Severity::Error,
            });
            false
        }
    }
    /// Takes the next new line character from the parser and returns true.
    /// If the parser is not pointing at a new line, a diagnostic is created and false is returned.
    fn parse_line_end(&mut self, parser: &mut Parser) -> bool {
        if parser.expect('\n') {
            true
        } else {
            self.diagnostics.push(Diagnostic {
                span: Some(parser.span_char()),
                message: Message::ExpectedExact("\n"),
                severity: Severity::Error,
            });
            false
        }
    }
    /// Take the next path from the parser.
    ///
    /// A path is a string literal which resolves to a path relative to the project root.
    fn parse_path(&mut self, parser: &mut Parser) -> Option<(PathBuf, Span)> {
        let Some((path, span)) = parser.string() else {
            self.diagnostics.push(Diagnostic {
                span: Some(parser.span_until(|c| c == ' ' || c == '\n')),
                message: Message::ExpectedPath,
                severity: Severity::Error,
            });
            return None;
        };
        let path_buf;
        if path.starts_with('/') {
            let path = self.evaluate_str(&path[1..], span.file, span.offset + 2);
            path_buf = self.root.join(path);
        } else {
            let path = self.evaluate_str(path, span.file, span.offset + 1);
            path_buf = self.file(parser.file).relative_path(path)
        }
        Some((path_buf, span))
    }
    /// Take the next string from the parser.
    fn parse_string(&mut self, parser: &mut Parser) -> Option<(String, Span)> {
        if parser.as_str().starts_with("'''\n") {
            return self.parse_multiline_string(parser);
        }
        let Some((string, span)) = parser.string() else {
            self.diagnostics.push(Diagnostic {
                span: Some(parser.span_until(|c| c == ' ' || c == '\n')),
                message: Message::ExpectedString,
                severity: Severity::Error,
            });
            return None;
        };
        Some((self.evaluate_str(string, span.file, span.offset + 1), span))
    }
    /// Take a multi-line string, which starts with `'''` at the end of a line, and ends with `'''` at the
    /// indentation of that line.
    ///
    /// Each line between must be indented by 4 spaces more than the line it starts on, which is removed, or be blank.
    /// A comment, with `#` in the first column, is skipped.
    /// The lines are templates, like other strings, and are joined by newlines, without one at the end.
    fn parse_multiline_string(&mut self, parser: &mut Parser) -> Option<(String, Span)> {
        let start = parser.offset;
        let line_start = parser.data[..start].rfind('\n').map_or(0, |i| i + 1);
        let indent = parser.data[line_start..start].len() - parser.data[line_start..start].trim_start_matches(' ').len();
        let content_indent = " ".repeat(indent + 4);
        let closing = format!("{}'''", " ".repeat(indent));
        let span = Span { file: parser.file, offset: start, len: 3 };
        parser.offset += 4;
        let mut lines = Vec::new();
        let mut valid = true;
        loop {
            let rest = parser.as_str();
            let len = rest.find('\n').unwrap_or(rest.len());
            let line = &rest[..len];
            if line.starts_with(&closing) {
                parser.offset += closing.len();
                break;
            }
            // a comment in the first column is not part of the string
            if line.starts_with('#') {
                parser.offset += len + 1;
                continue;
            }
            let spaces = line.len() - line.trim_start_matches(' ').len();
            // a line which is not indented beyond the start is not part of the string, which was not ended
            if rest.is_empty() || !line.trim().is_empty() && spaces <= indent {
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::UnterminatedString,
                    severity: Severity::Error,
                });
                // the end of the previous line, so that skipping the rest of the statement stops at this one
                if !rest.is_empty() {
                    parser.offset -= 1;
                }
                return None;
            }
            if line.trim().is_empty() {
                lines.push(("", parser.offset));
            } else if let Some(content) = line.strip_prefix(&content_indent) {
                lines.push((content, parser.offset + content_indent.len()));
            } else {
                self.diagnostics.push(Diagnostic {
                    span: Some(Span { file: parser.file, offset: parser.offset, len: spaces.max(1) }),
                    message: Message::StringIndentation { expected: indent + 4 },
                    severity: Severity::Error,
                });
                valid = false;
            }
            parser.offset += (len + 1).min(rest.len());
        }
        if !valid {
            return None;
        }
        let lines: Vec<_> = lines.iter().map(|(line, offset)| self.evaluate_str(line, parser.file, *offset)).collect();
        Some((lines.join("\n"), span))
    }
    /// Take the next expression from the parser and evaluate it.
    ///
    /// An expression is an operand, or operands joined by operators.
    /// An operator must have a single space on either side of it.
    fn parse_expression(&mut self, parser: &mut Parser) -> Option<Operand> {
        self.parse_binary(parser, 0)
    }
    /// Take operands joined by operators with a precedence of at least `min_precedence`.
    fn parse_binary(&mut self, parser: &mut Parser, min_precedence: u8) -> Option<Operand> {
        let mut lhs = self.parse_operand(parser)?;
        while let Some(operator) = parser.operator(min_precedence) {
            let rhs = self.parse_binary(parser, operator.precedence() + 1)?;
            lhs = self.evaluate_binary(operator, lhs, rhs)?;
        }
        Some(lhs)
    }
    /// Take the next operand from the parser.
    ///
    /// An operand is a literal, a property name, an expression in `{}` to permit coercion, or an expression in `()`.
    /// It may be preceded by `!` to negate a boolean.
    fn parse_operand(&mut self, parser: &mut Parser) -> Option<Operand> {
        let start = parser.offset;
        match parser.peek() {
            Some('\'') => {
                let (string, span) = self.parse_string(parser)?;
                Some(Operand { data: Data::Str(string), span, coerce: Coercion::Strict, origin: None })
            },
            Some('0'..='9' | '-' | '+') => {
                let span = parser.span_until(|c| c == ' ' || c == '\n');
                let Some((data, span)) = parser.integer() else {
                    self.diagnostics.push(Diagnostic {
                        span: Some(span),
                        message: Message::InvalidInteger,
                        severity: Severity::Error,
                    });
                    return None;
                };
                Some(Operand { data, span, coerce: Coercion::Sign, origin: None })
            },
            // the complement of an integer, or the negation of a boolean, as which `{}` is taken
            Some('!') => {
                parser.expect('!');
                let operand = self.parse_operand(parser)?;
                let data = match operand.data {
                    Data::Unsigned(i) if operand.coerce != Coercion::Any => Data::Unsigned(!i),
                    Data::Signed(i) if operand.coerce != Coercion::Any => Data::Signed(!i),
                    _ => Data::Bool(!self.bool_operand(operand)?),
                };
                Some(Operand { data, span: parser.span_from(start), coerce: Coercion::Strict, origin: None })
            },
            Some('(') => {
                parser.expect('(');
                let inner = self.parse_binary(parser, 0)?;
                if !parser.expect(')') {
                    self.diagnostics.push(Diagnostic {
                        span: Some(parser.span_char()),
                        message: Message::ExpectedExact(")"),
                        severity: Severity::Error,
                    });
                    return None;
                }
                Some(Operand { span: parser.span_from(start), ..inner })
            },
            Some('[') => {
                parser.expect('[');
                let items = self.parse_arguments(parser, ']', |this, parser| this.parse_binary(parser, 0))?;
                let data = Data::List(items.into_iter().map(|item| item.data).collect());
                Some(Operand { data, span: parser.span_from(start), coerce: Coercion::Strict, origin: None })
            },
            Some('{') => {
                parser.expect('{');
                let inner = self.parse_binary(parser, 0)?;
                if !parser.expect('}') {
                    self.diagnostics.push(Diagnostic {
                        span: Some(parser.span_char()),
                        message: Message::ExpectedExact("}"),
                        severity: Severity::Error,
                    });
                    return None;
                }
                // the target of a crate may be computed
                if is_crate_reference(parser) {
                    return self.parse_crate_reference(parser, inner.data.coerce_string(), start);
                }
                Some(Operand { span: parser.span_from(start), coerce: Coercion::Any, ..inner })
            },
            // any word is parsed as an identifier, to report its invalid characters
            Some(c) if c.is_alphabetic() || c == '_' => {
                let (name, span) = self.parse_ident(parser)?;
                if parser.peek() == Some('(') {
                    return self.parse_call(parser, name, span);
                }
                if is_crate_reference(parser) {
                    // the target is a property, such as `host-target`, or the name of a target
                    let target = match self.value(name, span.file) {
                        Some(Value { data: Data::Str(target), .. }) => target.clone(),
                        _ => name.into(),
                    };
                    return self.parse_crate_reference(parser, target, start);
                }
                if is_attribute_access(parser) {
                    return self.parse_named_attribute(parser, name, span, start);
                }
                let (data, origin) = match name {
                    "true" => (Data::Bool(true), None),
                    "false" => (Data::Bool(false), None),
                    _ => self.property(name, span)?,
                };
                Some(Operand { data, span, coerce: Coercion::Strict, origin })
            },
            _ => {
                self.diagnostics.push(Diagnostic {
                    span: Some(parser.span_until(|c| c == ' ' || c == '\n')),
                    message: Message::ExpectedExpression,
                    severity: Severity::Error,
                });
                None
            },
        }
    }
    /// Take the arguments of a function call, and apply the function.
    ///
    /// Arguments are separated by `, ` and are expressions, except for `select`, which takes property names.
    fn parse_call(&mut self, parser: &mut Parser, name: &str, name_span: Span) -> Option<Operand> {
        parser.expect('(');
        let expected = match name {
            "upper" | "lower" | "env" | "path" | "read" | "exists" | "which" | "len" => "1",
            "starts-with" | "ends-with" | "contains" | "join" | "split" => "2",
            "get" => "2 or 3",
            "select" | "run" | "min" | "max" => "at least 1",
            "try" => "2",
            _ => {
                self.diagnostics.push(Diagnostic {
                    span: Some(name_span),
                    message: Message::UnknownFunction(name.into()),
                    severity: Severity::Error,
                });
                return None;
            },
        };
        if name == "try" {
            return self.parse_try(parser, name_span);
        }
        if name == "run" {
            let arguments = self.parse_arguments(parser, ')', Self::parse_run_argument)?;
            let span = parser.span_from(name_span.offset);
            return self.run(arguments, span);
        }
        if name == "select" {
            let names = self.parse_arguments(parser, ')', |this, parser| {
                this.parse_ident(parser).map(|(name, span)| (name.to_string(), span))
            })?;
            let span = parser.span_from(name_span.offset);
            if names.is_empty() {
                self.argument_count(name, expected, 0, span);
                return None;
            }
            return self.select(&names, span);
        }
        let mut arguments = self.parse_arguments(parser, ')', |this, parser| this.parse_binary(parser, 0))?.into_iter();
        let span = parser.span_from(name_span.offset);
        let count = arguments.len().to_string();
        let at_least = expected.strip_prefix("at least ").and_then(|min| min.parse::<usize>().ok());
        if !expected.split(" or ").any(|expected| expected == count) && !at_least.is_some_and(|min| arguments.len() >= min) {
            self.argument_count(name, expected, arguments.len(), span);
            return None;
        }
        let data = match name {
            "upper" => Data::Str(self.str_operand(arguments.next()?)?.to_uppercase()),
            // the number of items in a list, or bytes in a string
            // the item at an index, or the default if there is one when the index is out of range
            "get" => {
                let items = self.list_operand(arguments.next()?)?;
                let index = arguments.next()?;
                let index_span = index.span;
                let index = match self.integer_operand(index)?.data {
                    Data::Unsigned(i) => usize::try_from(i).ok(),
                    Data::Signed(i) => usize::try_from(i).ok(),
                    _ => unreachable!(),
                };
                let default = arguments.next().map(|default| default.data);
                match index.and_then(|index| items.get(index)) {
                    Some(item) => item.clone(),
                    None => match default {
                        Some(default) => default,
                        None => {
                            self.diagnostics.push(Diagnostic {
                                span: Some(index_span),
                                message: Message::IndexOutOfRange { len: items.len() },
                                severity: Severity::Error,
                            });
                            return None;
                        },
                    },
                }
            },
            "min" | "max" => {
                // each item of a list is a value, which may be signed or unsigned like a literal
                let mut values = Vec::new();
                for argument in arguments {
                    match argument.data {
                        Data::List(items) => values.extend(items.into_iter().map(|data| {
                            Operand { data, span: argument.span, coerce: Coercion::Sign, origin: argument.origin }
                        })),
                        _ => values.push(argument),
                    }
                }
                let mut values = values.into_iter();
                let Some(first) = values.next() else {
                    self.diagnostics.push(Diagnostic {
                        span: Some(span),
                        message: Message::NoValues,
                        severity: Severity::Error,
                    });
                    return None;
                };
                let mut result = self.integer_operand(first)?;
                for value in values {
                    let value = self.integer_operand(value)?;
                    let (value_span, value_coerce) = (value.span, value.coerce);
                    let (result_span, result_coerce) = (result.span, result.coerce);
                    let (current, other) = self.unify_sign(result, value, span)?;
                    let ordering = match (&current, &other) {
                        (Data::Unsigned(lhs), Data::Unsigned(rhs)) => lhs.cmp(rhs),
                        (Data::Signed(lhs), Data::Signed(rhs)) => lhs.cmp(rhs),
                        _ => unreachable!(),
                    };
                    let replace = if name == "min" { ordering.is_gt() } else { ordering.is_lt() };
                    // the result keeps the type both are unified to
                    result = if replace {
                        Operand { data: other, span: value_span, coerce: value_coerce, origin: None }
                    } else {
                        Operand { data: current, span: result_span, coerce: result_coerce, origin: None }
                    };
                }
                result.data
            },
            "join" => {
                let items = self.list_operand(arguments.next()?)?;
                let separator = self.str_operand(arguments.next()?)?;
                let items: Vec<_> = items.iter().map(Data::coerce_string).collect();
                Data::Str(items.join(&separator))
            },
            "split" => {
                let string = self.str_operand(arguments.next()?)?;
                let separator = arguments.next()?;
                let separator_span = separator.span;
                let separator = self.str_operand(separator)?;
                if separator.is_empty() {
                    self.diagnostics.push(Diagnostic {
                        span: Some(separator_span),
                        message: Message::EmptySeparator,
                        severity: Severity::Error,
                    });
                    return None;
                }
                Data::List(string.split(&separator).map(|item| Data::Str(item.into())).collect())
            },
            "len" => {
                let operand = arguments.next()?;
                match &operand.data {
                    Data::Str(string) => Data::Unsigned(string.len() as u64),
                    Data::List(items) => Data::Unsigned(items.len() as u64),
                    data => {
                        self.diagnostics.push(Diagnostic {
                            span: Some(operand.span),
                            message: Message::InvalidOperand {
                                ty: data.type_name(),
                                expected: "list or str",
                                coerce: false,
                                origin: operand.origin,
                            },
                            severity: Severity::Error,
                        });
                        return None;
                    },
                }
            },
            "contains" => {
                let items = self.list_operand(arguments.next()?)?;
                let value = arguments.next()?.data;
                // as with `==`, integers are equal regardless of their sign
                Data::Bool(items.iter().any(|item| *item == value || item.convert_sign().is_some_and(|item| item == value)))
            },
            // relative to the directory of the file, where an absolute path stays absolute
            "path" => {
                let path = self.str_operand(arguments.next()?)?;
                Data::Str(normalize(&self.file(name_span.file).relative_path(path)).display().to_string())
            },
            "which" => {
                let argument = arguments.next()?;
                let argument_span = argument.span;
                let name = self.str_operand(argument)?;
                match self.which(&name) {
                    Ok(path) => Data::Str(path.display().to_string()),
                    Err(message) => {
                        self.diagnostics.push(Diagnostic {
                            span: Some(argument_span),
                            message,
                            severity: Severity::Error,
                        });
                        return None;
                    },
                }
            },
            "exists" => {
                let path = self.str_operand(arguments.next()?)?;
                let path = normalize(&self.file(name_span.file).relative_path(path));
                self.depend_on_path(&path);
                Data::Bool(path.exists())
            },
            "read" => {
                let argument = arguments.next()?;
                let argument_span = argument.span;
                let path = self.str_operand(argument)?;
                let path = normalize(&self.file(name_span.file).relative_path(path));
                // depend on the file even if it cannot be read, in case the error is suppressed by `try`
                self.depend_on_path(&path);
                match std::fs::read_to_string(&path) {
                    Ok(contents) => Data::Str(contents.trim_end_matches(['\n', '\r']).into()),
                    Err(error) => {
                        self.diagnostics.push(Diagnostic {
                            span: Some(argument_span),
                            message: Message::ReadFailure(path, error),
                            severity: Severity::Error,
                        });
                        return None;
                    },
                }
            },
            "lower" => Data::Str(self.str_operand(arguments.next()?)?.to_lowercase()),
            "starts-with" | "ends-with" => {
                let string = self.str_operand(arguments.next()?)?;
                let affix = self.str_operand(arguments.next()?)?;
                Data::Bool(if name == "starts-with" { string.starts_with(&affix) } else { string.ends_with(&affix) })
            },
            _ => {
                let variable = arguments.next()?;
                let variable_span = variable.span;
                let variable = self.str_operand(variable)?;
                match std::env::var(&variable) {
                    Ok(value) => Data::Str(value),
                    Err(error) => {
                        self.diagnostics.push(Diagnostic {
                            span: Some(variable_span),
                            message: Message::EnvironmentVariable {
                                name: variable,
                                present: matches!(error, std::env::VarError::NotUnicode(_)),
                            },
                            severity: Severity::Error,
                        });
                        return None;
                    },
                }
            },
        };
        Some(Operand { data, span, coerce: Coercion::Strict, origin: None })
    }
    /// Take the arguments of `try`, returning the value of the first, or the default, the second, if evaluating the
    /// first failed.
    ///
    /// Errors from the first argument are reported as warnings instead. As evaluation stops at the first error, the rest
    /// of the argument is skipped to the separator.
    fn parse_try(&mut self, parser: &mut Parser, name_span: Span) -> Option<Operand> {
        let start = parser.offset;
        let first = self.diagnostics.len();
        let operand = if parser.peek() == Some(')') {
            None
        } else {
            self.parse_binary(parser, 0)
        };
        // without the end of the expression, what follows it cannot be found, so the errors stand
        let end = start + argument_end(&parser.data[start..])?;
        if operand.is_some() && parser.offset != end {
            self.diagnostics.push(Diagnostic {
                span: Some(parser.span_char()),
                message: Message::ExpectedSeparator(')'),
                severity: Severity::Error,
            });
        }
        parser.offset = end;
        let mut operand = operand;
        for diagnostic in &mut self.diagnostics[first..] {
            if matches!(diagnostic.severity, Severity::Error) {
                diagnostic.severity = Severity::Warning;
                operand = None;
            }
        }
        // the default is required, and its errors are not suppressed
        if !parser.as_str().starts_with(", ") {
            parser.expect(')');
            self.argument_count("try", "2", 1, parser.span_from(name_span.offset));
            return None;
        }
        parser.offset += 2;
        let default = self.parse_binary(parser, 0)?;
        if !parser.expect(')') {
            self.diagnostics.push(Diagnostic {
                span: Some(parser.span_char()),
                message: Message::ExpectedExact(")"),
                severity: Severity::Error,
            });
            return None;
        }
        let span = parser.span_from(name_span.offset);
        let data = operand.unwrap_or(default).data;
        Some(Operand { data, span, coerce: Coercion::Strict, origin: None })
    }
    /// Take an argument of `run`, which is an expression, or an environment variable as `name: expression`.
    ///
    /// The name of an environment variable is a name, as taken by [`Self::parse_name`]. As an expression can also be
    /// an identifier or start with `{`, it is only a name if it is followed by `: `.
    fn parse_run_argument(&mut self, parser: &mut Parser) -> Option<RunArgument> {
        let rest = parser.as_str();
        let len = match rest.strip_prefix('{') {
            Some(inner) => expression_end(inner).map_or(0, |end| end + 2),
            None => rest.find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-').unwrap_or(rest.len()),
        };
        if len == 0 || !rest[len..].starts_with(": ") {
            return Some(RunArgument::Positional(self.parse_binary(parser, 0)?));
        }
        let (name, _, _) = self.parse_name(parser)?;
        parser.offset += 2;
        Some(RunArgument::Env(name, self.parse_binary(parser, 0)?))
    }
    /// Run a program, returning its output without trailing newlines.
    ///
    /// The first argument is the program, followed by its arguments and then any environment variables.
    fn run(&mut self, arguments: Vec<RunArgument>, span: Span) -> Option<Operand> {
        let mut command_line = Vec::new();
        let mut env = Vec::new();
        let mut failed = false;
        for argument in arguments {
            match argument {
                RunArgument::Positional(operand) if !env.is_empty() => {
                    self.diagnostics.push(Diagnostic {
                        span: Some(operand.span),
                        message: Message::PositionalAfterEnv,
                        severity: Severity::Error,
                    });
                    failed = true;
                },
                // each item of a list is a separate argument
                RunArgument::Positional(Operand { data: Data::List(items), .. }) => {
                    command_line.extend(Data::List(items).spread());
                },
                RunArgument::Positional(operand) => match self.str_operand(operand) {
                    Some(string) => command_line.push(string),
                    None => failed = true,
                },
                RunArgument::Env(name, operand) => match self.str_operand(operand) {
                    Some(value) => env.push((name, value)),
                    None => failed = true,
                },
            }
        }
        if failed {
            return None;
        }
        let Some((program, args)) = command_line.split_first() else {
            self.argument_count("run", "at least 1", 0, span);
            return None;
        };
        // as in `sh`, a program with a `/` is a path, here relative to the file, otherwise it is searched for in `PATH`
        let program = &if program.contains('/') {
            let mut path = normalize(&self.file(span.file).relative_path(program));
            self.depend_on_path(&path);
            // a path in the same directory must still have a `/`, so that it is not searched for in `PATH`
            if path.is_relative() {
                path = Path::new(".").join(path);
            }
            path.display().to_string()
        } else {
            program.clone()
        };
        let output = std::process::Command::new(program)
            .args(args)
            .envs(env)
            .stdin(std::process::Stdio::null())
            .output();
        let message = match output {
            Err(error) => Message::RunSpawn { program: program.clone(), error: error.to_string() },
            Ok(output) if !output.status.success() => Message::RunFailed {
                program: program.clone(),
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim_end().into(),
            },
            Ok(output) => match String::from_utf8(output.stdout) {
                Ok(stdout) => {
                    let stdout = stdout.trim_end_matches(['\n', '\r']).into();
                    return Some(Operand { data: Data::Str(stdout), span, coerce: Coercion::Strict, origin: None });
                },
                Err(_) => Message::RunOutput { program: program.clone() },
            },
        };
        self.diagnostics.push(Diagnostic {
            span: Some(span),
            message,
            severity: Severity::Error,
        });
        None
    }
    /// Search the directories in `PATH` for an executable file.
    ///
    /// The directories searched before it is found are dependencies, as the program could be added to one of them.
    fn which(&mut self, name: &str) -> Result<PathBuf, Message> {
        use std::os::unix::fs::PermissionsExt;
        if name.is_empty() || name.contains('/') {
            return Err(Message::InvalidProgramName(name.into()));
        }
        let search = std::env::var_os("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&search) {
            // an empty entry means the working directory, which is not a reliable place to find a program
            if dir.as_os_str().is_empty() {
                continue;
            }
            let path = dir.join(name);
            self.depend(dir);
            if path.metadata().is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0) {
                return Ok(path);
            }
        }
        Err(Message::ProgramNotFound(name.into()))
    }
    /// Add a path which the configuration depends on, if it exists.
    ///
    /// A missing dependency is always out of date, which would regenerate the configuration endlessly, so a path which
    /// does not exist is not tracked.
    fn depend_on_path(&mut self, path: &Path) {
        if path.exists() {
            self.depend(path.to_path_buf());
        }
    }
    /// Add a file which the configuration depends on, so that it is regenerated when the file changes.
    fn depend(&mut self, path: PathBuf) {
        if !self.dependencies.contains(&path) {
            self.dependencies.push(path);
        }
    }
    /// Take a list of arguments separated by `, `, after the opening parenthesis or bracket, up to the closing one.
    fn parse_arguments<'a, T>(
        &mut self,
        parser: &mut Parser<'a>,
        close: char,
        mut parse: impl FnMut(&mut Self, &mut Parser<'a>) -> Option<T>,
    ) -> Option<Vec<T>> {
        let mut arguments = Vec::new();
        if parser.expect(close) {
            return Some(arguments);
        }
        loop {
            arguments.push(parse(self, parser)?);
            if parser.expect(close) {
                return Some(arguments);
            }
            if !parser.as_str().starts_with(", ") {
                self.diagnostics.push(Diagnostic {
                    span: Some(parser.span_char()),
                    message: Message::ExpectedSeparator(close),
                    severity: Severity::Error,
                });
                return None;
            }
            parser.offset += 2;
        }
    }
    fn argument_count(&mut self, function: &str, expected: &'static str, found: usize, span: Span) {
        self.diagnostics.push(Diagnostic {
            span: Some(span),
            message: Message::ArgumentCount { function: function.into(), expected, found },
            severity: Severity::Error,
        });
    }
    /// Returns the value of the first property which has one.
    ///
    /// Every property must have been declared, or forward declared by a `set`, so that a misspelling is not ignored.
    fn select(&mut self, names: &[(String, Span)], span: Span) -> Option<Operand> {
        let mut undeclared = false;
        for (name, name_span) in names {
            if !self.properties_idx.contains_key(name) {
                self.diagnostics.push(Diagnostic {
                    span: Some(*name_span),
                    message: Message::UndeclaredProperty(name.clone()),
                    severity: Severity::Error,
                });
                undeclared = true;
            }
        }
        if undeclared {
            return None;
        }
        let selected = names.iter().find_map(|(name, _)| self.value(name, span.file));
        let Some(value) = selected else {
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::NoneSelected(names.iter().map(|(name, _)| name.clone()).collect()),
                severity: Severity::Error,
            });
            return None;
        };
        Some(Operand { data: value.data.clone(), span, coerce: Coercion::Strict, origin: value.origin })
    }
    /// Ensure an operand is a string, converting it if it permits coercion.
    /// Ensure an operand is a list.
    fn list_operand(&mut self, operand: Operand) -> Option<Vec<Data>> {
        match operand.data {
            Data::List(items) => Some(items),
            data => {
                self.diagnostics.push(Diagnostic {
                    span: Some(operand.span),
                    message: Message::InvalidOperand {
                        ty: data.type_name(),
                        expected: "list",
                        coerce: false,
                        origin: operand.origin,
                    },
                    severity: Severity::Error,
                });
                None
            },
        }
    }
    fn str_operand(&mut self, operand: Operand) -> Option<String> {
        match operand.data {
            Data::Str(string) => Some(string),
            data if operand.coerce == Coercion::Any && data != Data::None => Some(data.coerce_string()),
            data => {
                self.diagnostics.push(Diagnostic {
                    span: Some(operand.span),
                    message: Message::InvalidOperand {
                        ty: data.type_name(),
                        expected: "str",
                        coerce: operand.coerce == Coercion::Any,
                        origin: operand.origin,
                    },
                    severity: Severity::Error,
                });
                None
            },
        }
    }
    /// Take the rest of a reference to a crate, `<target>/<name>`, after its target, returning the path of its output.
    ///
    /// An attribute of the crate can be taken instead with `.`, as in `<target>/<name>.crate-type`.
    fn parse_crate_reference(&mut self, parser: &mut Parser, target: String, start: usize) -> Option<Operand> {
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
    fn parse_named_attribute(&mut self, parser: &mut Parser, name: &str, span: Span, start: usize) -> Option<Operand> {
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
    /// Returns the value of a property referenced by an expression, and where the value was set.
    fn property(&mut self, name: &str, span: Span) -> Option<(Data, Option<Span>)> {
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
    /// Apply an operator.
    ///
    /// Operands must have the type required by the operator, and the same type as each other.
    /// An operand is only converted to satisfy this as far as its [`Coercion`] permits.
    fn evaluate_binary(&mut self, operator: Operator, lhs: Operand, rhs: Operand) -> Option<Operand> {
        let span = Span {
            file: lhs.span.file,
            offset: lhs.span.offset,
            len: rhs.span.offset + rhs.span.len - lhs.span.offset,
        };
        let rhs_span = rhs.span;
        let mut coerce = Coercion::Strict;
        let data = match operator {
            Operator::Or | Operator::And => {
                let lhs = self.bool_operand(lhs)?;
                let rhs = self.bool_operand(rhs)?;
                Data::Bool(match operator {
                    Operator::Or => lhs || rhs,
                    _ => lhs && rhs,
                })
            },
            Operator::Equal | Operator::NotEqual => {
                let (lhs, rhs) = self.comparable_operands(lhs, rhs, span)?;
                Data::Bool((lhs == rhs) == matches!(operator, Operator::Equal))
            },
            Operator::Less | Operator::LessEqual | Operator::Greater | Operator::GreaterEqual => {
                let ordering = match self.integer_operands(lhs, rhs, span)? {
                    (Data::Unsigned(lhs), Data::Unsigned(rhs)) => lhs.cmp(&rhs),
                    (Data::Signed(lhs), Data::Signed(rhs)) => lhs.cmp(&rhs),
                    _ => unreachable!(),
                };
                Data::Bool(match operator {
                    Operator::Less => ordering.is_lt(),
                    Operator::LessEqual => ordering.is_le(),
                    Operator::Greater => ordering.is_gt(),
                    _ => ordering.is_ge(),
                })
            },
            // lists are concatenated, but only with other lists
            Operator::Add if matches!(lhs.data, Data::List(_)) || matches!(rhs.data, Data::List(_)) => {
                let (Data::List(mut lhs_items), Data::List(rhs_items)) = (lhs.data.clone(), rhs.data.clone()) else {
                    self.diagnostics.push(Diagnostic {
                        span: Some(span),
                        message: Message::MismatchedOperands {
                            lhs: lhs.data.type_name(),
                            rhs: rhs.data.type_name(),
                            lhs_origin: lhs.origin,
                            rhs_origin: rhs.origin,
                        },
                        severity: Severity::Error,
                    });
                    return None;
                };
                lhs_items.extend(rhs_items);
                Data::List(lhs_items)
            },
            Operator::Add
            | Operator::Subtract
            | Operator::Multiply
            | Operator::Divide
            | Operator::Remainder
            | Operator::BitOr
            | Operator::BitXor
            | Operator::BitAnd
            | Operator::ShiftLeft
            | Operator::ShiftRight => {
                coerce = lhs.coerce.min(rhs.coerce);
                let (lhs, rhs) = self.integer_operands(lhs, rhs, span)?;
                if matches!(operator, Operator::Divide | Operator::Remainder) && matches!(rhs, Data::Unsigned(0) | Data::Signed(0)) {
                    self.diagnostics.push(Diagnostic {
                        span: Some(rhs_span),
                        message: Message::DivisionByZero,
                        severity: Severity::Error,
                    });
                    return None;
                }
                macro_rules! checked {
                    ($lhs:expr, $rhs:expr) => {
                        match operator {
                            Operator::Add => $lhs.checked_add($rhs),
                            Operator::Subtract => $lhs.checked_sub($rhs),
                            Operator::Multiply => $lhs.checked_mul($rhs),
                            Operator::Divide => $lhs.checked_div($rhs),
                            Operator::Remainder => $lhs.checked_rem($rhs),
                            Operator::BitOr => Some($lhs | $rhs),
                            Operator::BitXor => Some($lhs ^ $rhs),
                            Operator::BitAnd => Some($lhs & $rhs),
                            // the amount must be less than the number of bits, while the bits shifted out are lost
                            Operator::ShiftLeft => u32::try_from($rhs).ok().and_then(|rhs| $lhs.checked_shl(rhs)),
                            _ => u32::try_from($rhs).ok().and_then(|rhs| $lhs.checked_shr(rhs)),
                        }
                    };
                }
                let data = match (lhs, rhs) {
                    (Data::Unsigned(lhs), Data::Unsigned(rhs)) => checked!(lhs, rhs).map(Data::Unsigned),
                    (Data::Signed(lhs), Data::Signed(rhs)) => checked!(lhs, rhs).map(Data::Signed),
                    _ => unreachable!(),
                };
                let Some(data) = data else {
                    let shift = matches!(operator, Operator::ShiftLeft | Operator::ShiftRight);
                    self.diagnostics.push(Diagnostic {
                        span: Some(if shift { rhs_span } else { span }),
                        message: if shift { Message::ShiftAmount } else { Message::IntegerOverflow },
                        severity: Severity::Error,
                    });
                    return None;
                };
                data
            },
        };
        Some(Operand { data, span, coerce, origin: None })
    }
    /// Ensure an operand is a boolean, converting it if it permits coercion.
    fn bool_operand(&mut self, operand: Operand) -> Option<bool> {
        let value = match &operand.data {
            Data::Bool(value) => Some(*value),
            data if operand.coerce == Coercion::Any => data.coerce_bool(),
            _ => None,
        };
        if value.is_none() {
            self.diagnostics.push(Diagnostic {
                span: Some(operand.span),
                message: Message::InvalidOperand {
                    ty: operand.data.type_name(),
                    expected: "bool",
                    coerce: operand.coerce == Coercion::Any,
                    origin: operand.origin,
                },
                severity: Severity::Error,
            });
        }
        value
    }
    /// Ensure an operand is an integer, converting it if it permits coercion.
    fn integer_operand(&mut self, operand: Operand) -> Option<Operand> {
        let data = match &operand.data {
            Data::Unsigned(_) | Data::Signed(_) => return Some(operand),
            data if operand.coerce == Coercion::Any => data.coerce_integer(),
            _ => None,
        };
        let Some(data) = data else {
            self.diagnostics.push(Diagnostic {
                span: Some(operand.span),
                message: Message::InvalidOperand {
                    ty: operand.data.type_name(),
                    expected: "integer",
                    coerce: operand.coerce == Coercion::Any,
                    origin: operand.origin,
                },
                severity: Severity::Error,
            });
            return None;
        };
        Some(Operand { data, ..operand })
    }
    /// Ensure both operands are integers of the same type.
    fn integer_operands(&mut self, lhs: Operand, rhs: Operand, span: Span) -> Option<(Data, Data)> {
        let lhs = self.integer_operand(lhs)?;
        let rhs = self.integer_operand(rhs)?;
        self.unify_sign(lhs, rhs, span)
    }
    /// Ensure two integer operands are either both signed or both unsigned.
    fn unify_sign(&mut self, lhs: Operand, rhs: Operand, span: Span) -> Option<(Data, Data)> {
        if lhs.data.type_name() == rhs.data.type_name() {
            return Some((lhs.data, rhs.data));
        }
        if let Some(data) = rhs.data.convert_sign() && rhs.coerce >= Coercion::Sign {
            return Some((lhs.data, data));
        }
        if let Some(data) = lhs.data.convert_sign() && lhs.coerce >= Coercion::Sign {
            return Some((data, rhs.data));
        }
        self.diagnostics.push(Diagnostic {
            span: Some(span),
            message: Message::MismatchedOperands {
                lhs: lhs.data.type_name(),
                rhs: rhs.data.type_name(),
                lhs_origin: lhs.origin,
                rhs_origin: rhs.origin,
            },
            severity: Severity::Error,
        });
        None
    }
    /// Ensure both operands have the same type, of any kind.
    fn comparable_operands(&mut self, lhs: Operand, rhs: Operand, span: Span) -> Option<(Data, Data)> {
        if lhs.data.type_name() == rhs.data.type_name() {
            return Some((lhs.data, rhs.data));
        }
        if lhs.data.is_integer() && rhs.data.is_integer() {
            return self.unify_sign(lhs, rhs, span);
        }
        let (from, to, swap) = if rhs.coerce == Coercion::Any {
            (rhs, lhs, false)
        } else if lhs.coerce == Coercion::Any {
            (lhs, rhs, true)
        } else {
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::MismatchedOperands {
                    lhs: lhs.data.type_name(),
                    rhs: rhs.data.type_name(),
                    lhs_origin: lhs.origin,
                    rhs_origin: rhs.origin,
                },
                severity: Severity::Error,
            });
            return None;
        };
        let Some(data) = from.data.coerce_like(&to.data) else {
            self.diagnostics.push(Diagnostic {
                span: Some(from.span),
                message: Message::InvalidOperand {
                    ty: from.data.type_name(),
                    expected: to.data.type_name(),
                    coerce: true,
                    origin: from.origin,
                },
                severity: Severity::Error,
            });
            return None;
        };
        Some(if swap { (data, to.data) } else { (to.data, data) })
    }
    /// Take an indented attribute, with an optional value.
    ///
    /// The value of an attribute in `names` is a name, as taken by [`Self::parse_name`], and is required. Otherwise it
    /// is an expression.
    fn parse_attribute<'a>(&mut self, parser: &mut Parser<'a>, names: &[&str]) -> Option<Attribute<'a>> {
        let (name, name_span) = self.parse_ident(parser)?;
        let (data, data_span, origin) = if names.contains(&name) {
            if !self.parse_space(parser) {
                return None;
            }
            let (mut value, mut span, _) = self.parse_name(parser)?;
            // a crate can only use crates for the same target, so it is not given, unlike in a reference to a crate
            if is_crate_reference(parser) {
                parser.expect('/');
                let (crate_name, crate_span) = self.parse_ident(parser)?;
                // the target is a property, such as `host-target`, or the name of a target, as in a reference
                let target = match self.value(&value, span.file) {
                    Some(Value { data: Data::Str(target), .. }) => target.clone(),
                    _ => value,
                };
                let declared = self.crates.get(&(crate_name.into(), target)).map(|c| c.name_span);
                value = crate_name.into();
                span = Span { file: span.file, offset: span.offset, len: crate_span.offset + crate_span.len - span.offset };
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::UseWithTarget { declared },
                    severity: Severity::Error,
                });
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::Replacement { description: "use the crate by its name", replacement: value.clone() },
                    severity: Severity::Suggestion,
                });
            }
            (Data::Str(value), span, None)
        } else if parser.expect(' ') {
            let operand = self.parse_expression(parser)?;
            (operand.data, operand.span, operand.origin)
        } else {
            (Data::None, name_span, None)
        };
        if !self.parse_line_end(parser) {
            parser.skip_line();
        }
        Some(Attribute {
            name,
            name_span,
            data,
            data_span,
            origin,
        })
    }
    /// Evaluate string templates.
    ///
    /// An expression in `{}` is replaced with its value, while `{{` and `}}` are replaced with a single brace.
    /// `offset` is the offset of `s` within the file.
    fn evaluate_str(&mut self, s: &str, file: usize, offset: usize) -> String {
        let mut string = String::new();
        let mut rest = s;
        while let Some(i) = rest.find(['{', '}']) {
            string.push_str(&rest[..i]);
            let brace = &rest[i..i + 1];
            let after = &rest[i + 1..];
            if after.starts_with(brace) {
                string.push_str(brace);
                rest = &after[1..];
                continue;
            }
            // the offset of `after` within the file
            let offset = offset + s.len() - after.len();
            if brace == "}" {
                self.diagnostics.push(Diagnostic {
                    span: Some(Span { file, offset: offset - 1, len: 1 }),
                    message: Message::InvalidStringTemplate,
                    severity: Severity::Error,
                });
                rest = after;
                continue;
            }
            let Some(end) = expression_end(after) else {
                self.diagnostics.push(Diagnostic {
                    span: Some(Span { file, offset: offset - 1, len: after.len() + 1 }),
                    message: Message::InvalidStringTemplate,
                    severity: Severity::Error,
                });
                return string;
            };
            let first_diagnostic = self.diagnostics.len();
            let mut parser = Parser::new(file, &after[..end]);
            match self.parse_expression(&mut parser) {
                Some(operand) if parser.is_empty() => string.push_str(&operand.data.coerce_string()),
                Some(_) => self.diagnostics.push(Diagnostic {
                    span: Some(parser.span_until(|_| false)),
                    message: Message::InvalidStringTemplate,
                    severity: Severity::Error,
                }),
                None => (),
            }
            // the parser's spans are relative to the expression
            for diagnostic in &mut self.diagnostics[first_diagnostic..] {
                if let Some(span) = &mut diagnostic.span {
                    span.offset += offset;
                }
            }
            rest = &after[end + 1..];
        }
        string.push_str(rest);
        string
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

/// What remains of a statement after its meta-statements.
enum Meta {
    /// The statement is skipped.
    Skip,
    /// The statement is processed.
    Process,
    /// The statement has been processed already, by `@for`.
    Done,
}

/// An argument of the `run` function.
enum RunArgument {
    Positional(Operand),
    /// An environment variable for the program.
    Env(String, Operand),
}

struct Attribute<'a> {
    name: &'a str,
    name_span: Span,
    data: Data,
    data_span: Span,
    /// Where the value was set, if the data is the value of a property.
    origin: Option<Span>,
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

#[derive(Clone, Copy)]
pub enum Severity {
    Error,
    Warning,
    Note,
    /// A change which fixes a diagnostic before it.
    Suggestion,
}
impl Severity {
    pub const fn accent(&self) -> &'static str {
        match self {
            Self::Error => "\x1b[91m\x1b[1m",
            Self::Warning => "\x1b[93m\x1b[1m",
            Self::Note => "\x1b[96m\x1b[1m",
            Self::Suggestion => "\x1b[92m\x1b[1m",
        }
    }
}
impl core::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
            Severity::Note => write!(f, "note"),
            Severity::Suggestion => write!(f, "suggestion"),
        }
    }
}

pub struct Diagnostic {
    pub span: Option<Span>,
    pub message: Message,
    pub severity: Severity,
}

#[repr(u32)]
pub enum Message {
    User(String),
    UnknownSeverity(String),
    UnknownStatement(String),
    UnknownMetaStatement(String),
    ExpectedStatementAfterMeta,
    UnknownFunction(String),
    /// A separator or the closing character of a list.
    ExpectedSeparator(char),
    ArgumentCount {
        function: String,
        expected: &'static str,
        found: usize,
    },
    NoneSelected(Vec<String>),
    PositionalAfterEnv,
    InvalidProgramName(String),
    ProgramNotFound(String),
    RunSpawn {
        program: String,
        error: String,
    },
    RunFailed {
        program: String,
        status: String,
        stderr: String,
    },
    RunOutput {
        program: String,
    },
    EnvironmentVariable {
        name: String,
        /// Whether the variable is set, but is not valid unicode.
        present: bool,
    },
    ElseWithoutIf(String),
    ThenWithoutStatement,
    ElseNotFirst(String),
    UnsetProperty(String),
    ExpectedIdentifier,
    InvalidIdentifier {
        name: String,
        character: char,
    },
    ExpectedString,
    UnterminatedString,
    StringIndentation {
        expected: usize,
    },
    InvalidStringTemplate,
    InvalidInteger,
    InvalidOperand {
        ty: &'static str,
        expected: &'static str,
        coerce: bool,
        /// Where the value was set, if it is the value of a property.
        origin: Option<Span>,
    },
    MismatchedOperands {
        lhs: &'static str,
        rhs: &'static str,
        lhs_origin: Option<Span>,
        rhs_origin: Option<Span>,
    },
    IntegerOverflow,
    ShiftAmount,
    DivisionByZero,
    InvalidType(String),
    ExpectedPath,
    ExpectedExpression,
    ReadFailure(PathBuf, std::io::Error),
    RepeatedUse {
        path: PathBuf,
        file: usize,
    },
    ExpectedExact(&'static str),
    UndeclaredProperty(String),
    RedeclareProperty {
        id: usize,
        name: String,
        builtin: bool,
    },
    ReadOnlyProperty {
        id: usize,
        name: String,
        /// The value already assigned, if it is not builtin.
        assigned: Option<Span>,
        builtin: bool,
    },
    UnknownDeclarationAttribute {
        property: usize,
        name: String,
    },
    UnknownCrateAttribute {
        name: String,
    },
    UnknownAttribute {
        statement: &'static str,
        name: String,
    },
    RepeatedAttribute(String),
    MissingAttribute(&'static str),
    RedeclareCustom {
        statement: &'static str,
        name: String,
        previous: Span,
    },
    NewlineInArgument,
    TestTarget {
        target: String,
        host: String,
    },
    ReservedCrate {
        name: String,
        target: String,
    },
    BuiltinTargetName(String),
    InvalidCrateType(String),
    EmptySeparator,
    NoValues,
    IndexOutOfRange {
        len: usize,
    },
    ProcMacroTarget {
        target: String,
        host: String,
    },
    UnknownReference(String),
    UnknownAttributeOf {
        kind: &'static str,
        name: String,
        attribute: String,
    },
    AttributeNotSet {
        kind: &'static str,
        name: String,
        attribute: String,
    },
    UseBinary {
        name: String,
        /// The declaration of the binary.
        declared: Span,
    },
    UseWithTarget {
        /// The declaration of the crate for the target, if it has been declared.
        declared: Option<Span>,
    },
    /// A suggestion to replace the span of the diagnostic.
    Replacement {
        description: &'static str,
        replacement: String,
    },
    OverrideAttribute(String),
    RepeatedTarget(String),
    RedeclareCrate {
        name: String,
        target: String,
    },
    UnknownCrate {
        name: String,
        target: String,
    },
    InvalidCrateName {
        name: String,
        character: char,
    },
    UnknownTarget {
        target: String,
        suggestion: Option<String>,
    },
    AvailableTargets(Vec<String>),
    MismatchedType {
        value: usize,
        name: String,
    },
}
impl core::fmt::Display for Message {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User(string) => f.write_str(string),
            Self::UnknownSeverity(name) => write!(f, "unknown severity {name:?}"),
            Self::UnknownStatement(name) => write!(f, "unknown statement {name:?}"),
            Self::UnknownMetaStatement(name) => write!(f, "unknown meta-statement \"@{name}\""),
            Self::UnknownFunction(name) => write!(f, "unknown function {name:?}"),
            Self::ExpectedSeparator(close) => write!(f, "expected \", \" or \"{close}\""),
            Self::ArgumentCount { function, expected, found } => {
                let suffix = if expected.ends_with(" 1") || *expected == "1" { "" } else { "s" };
                let verb = if *found == 1 { "was" } else { "were" };
                write!(f, "function {function:?} takes {expected} argument{suffix}, but {found} {verb} given")
            },
            Self::InvalidProgramName(name) => write!(f, "invalid program name {name:?}, which cannot contain \"/\""),
            Self::ProgramNotFound(name) => write!(f, "program {name:?} not found in PATH"),
            Self::PositionalAfterEnv => write!(f, "program arguments must come before environment variables"),
            Self::RunSpawn { program, error } => write!(f, "failed to run {program:?}: {error}"),
            Self::RunFailed { program, status, .. } => write!(f, "{program:?} failed with {status}"),
            Self::RunOutput { program } => write!(f, "output of {program:?} is not valid unicode"),
            Self::NoneSelected(names) => {
                let names: Vec<_> = names.iter().map(|name| format!("{name:?}")).collect();
                write!(f, "no value for any of the properties {}", names.join(", "))
            },
            Self::EnvironmentVariable { name, present: false } => write!(f, "environment variable {name:?} is not set"),
            Self::EnvironmentVariable { name, present: true } => {
                write!(f, "environment variable {name:?} is not valid unicode")
            },
            Self::ExpectedStatementAfterMeta => write!(f, "expected statement on the line after a meta-statement"),
            Self::ThenWithoutStatement => write!(f, "\"@then\" must be directly after a statement"),
            Self::ElseWithoutIf(name) => write!(f, "\"@{name}\" must follow a conditional statement"),
            Self::ElseNotFirst(name) => write!(f, "\"@{name}\" must be the first meta-statement of a statement"),
            Self::UnsetProperty(name) => write!(f, "no value for property {name:?}"),
            Self::ExpectedIdentifier => write!(f, "expected identifier"),
            Self::InvalidIdentifier { name, character } => write!(f, "invalid character {character:?} in identifier {name:?}"),
            Self::ExpectedString => write!(f, "expected string"),
            Self::UnterminatedString => write!(f, "multi-line string is not terminated"),
            Self::StringIndentation { expected } => {
                write!(f, "lines of a multi-line string must be indented by {expected} spaces, or end it")
            },
            Self::InvalidStringTemplate { .. } => write!(f, "invalid string template"),
            Self::InvalidInteger => write!(f, "invalid integer"),
            Self::InvalidOperand { ty, expected, coerce: false, .. } => write!(f, "{ty} value used where {expected} is required"),
            Self::InvalidOperand { ty, expected, coerce: true, .. } => write!(f, "{ty} value cannot be coerced to {expected}"),
            Self::MismatchedOperands { lhs, rhs, .. } => write!(f, "mismatched operand types {lhs} and {rhs}"),
            Self::IntegerOverflow => write!(f, "integer overflow"),
            Self::ShiftAmount => write!(f, "shift amount must be from 0 to 63"),
            Self::DivisionByZero => write!(f, "division by zero"),
            Self::InvalidType(ty) => write!(f, "invalid data type {ty:?}"),
            Self::ExpectedPath => write!(f, "expected path"),
            Self::ExpectedExpression => write!(f, "expected expression"),
            Self::ExpectedExact(s) => write!(f, "expected {s:?}"),
            Self::RepeatedUse { path, .. } => write!(f, "file {path:?} is already used"),
            Self::ReadFailure(path, error) => write!(f, "failed to read {path:?}: {error}"),
            Self::UndeclaredProperty(name) => write!(f, "property {name:?} is undeclared"),
            Self::RedeclareProperty { name, builtin: false, .. } => write!(f, "redeclaration of property {name:?}"),
            Self::RedeclareProperty { name, builtin: true, .. } => write!(f, "redeclaration of builtin property {name:?}"),
            Self::ReadOnlyProperty { name, builtin: false, .. } => write!(f, "cannot set read-only property {name:?}"),
            Self::ReadOnlyProperty { name, builtin: true, .. } => write!(f, "cannot set read-only builtin property {name:?}"),
            Self::UnknownDeclarationAttribute { name, .. } => write!(f, "unknown declare attribute {name:?}"),
            Self::UnknownCrateAttribute { name, .. } => write!(f, "unknown crate attribute {name:?}"),
            Self::UnknownAttribute { statement, name } => write!(f, "unknown {statement} attribute {name:?}"),
            Self::RepeatedAttribute(name) => write!(f, "attribute {name:?} cannot be repeated"),
            Self::MissingAttribute(name) => write!(f, "missing required attribute {name:?}"),
            Self::RedeclareCustom { statement: "test", name, .. } => write!(f, "redeclaration of test {name:?}"),
            Self::RedeclareCustom { statement: "target", name, .. } => write!(f, "redeclaration of target {name:?}"),
            Self::RedeclareCustom { name, .. } => write!(f, "redeclaration of custom command {name:?}"),
            Self::NewlineInArgument => write!(f, "command arguments cannot contain a newline"),
            Self::EmptySeparator => write!(f, "the separator cannot be empty"),
            Self::NoValues => write!(f, "there are no values to compare"),
            Self::IndexOutOfRange { len } => write!(f, "index out of range for a list of length {len}"),
            Self::InvalidCrateType(name) => {
                write!(f, "invalid crate type {name:?}, which must be \"lib\", \"bin\" or \"proc-macro\"")
            },
            Self::ProcMacroTarget { target, host } => write!(
                f,
                "procedural macros cannot be compiled for target {target:?}, only the host target {host:?}",
            ),
            Self::UseWithTarget { .. } => write!(f, "cannot use a crate of a different target"),
            Self::Replacement { description, .. } => f.write_str(description),
            Self::UnknownReference(name) => write!(f, "no property, custom command, target or test named {name:?}"),
            Self::UnknownAttributeOf { kind, name, attribute } => {
                write!(f, "unknown attribute {attribute:?} of {kind} {name:?}")
            },
            Self::AttributeNotSet { kind, name, attribute } => {
                write!(f, "attribute {attribute:?} of {kind} {name:?} is not set")
            },
            Self::UseBinary { name, .. } => write!(f, "crate {name:?} is a binary, which cannot be used by another crate"),
            Self::BuiltinTargetName(name) => {
                write!(f, "{name:?} is the name of a builtin target, which can be changed with `override`")
            },
            Self::OverrideAttribute(name) => write!(f, "attribute {name:?} cannot be used with `override`"),
            Self::RepeatedTarget(target) => write!(f, "target {target:?} is listed more than once"),
            Self::ReservedCrate { name, target } => {
                write!(f, "crate {name:?} for target {target:?} is reserved for the build system")
            },
            Self::TestTarget { target, host } => write!(
                f,
                "tests for target {target:?} cannot be run, only those for the host target {host:?} or a target with \
                 `test`",
            ),
            Self::UnknownCrate { name, target } => write!(f, "no crate {name:?} for target {target:?}"),
            Self::InvalidCrateName { name, character } => write!(f, "invalid character {character:?} in crate name {name:?}"),
            Self::RedeclareCrate { name, target } => write!(f, "redeclaration of crate {name:?} for target {target:?}"),
            Self::UnknownTarget { target, suggestion: None } => write!(f, "unknown target {target:?}"),
            Self::UnknownTarget { target, suggestion: Some(suggestion) } => {
                write!(f, "unknown target {target:?}, did you mean {suggestion:?}?")
            },
            Self::AvailableTargets(targets) => {
                write!(f, "available targets are:")?;
                for target in targets {
                    write!(f, "\n{target}")?;
                }
                Ok(())
            },
            Self::MismatchedType { name, .. } => write!(f, "invalid data type for property {name:?}"),
        }
    }
}

/// Returns the candidate closest to `s`, if it is close enough to be a likely misspelling.
fn suggest(s: &str, candidates: &[String]) -> Option<String> {
    let (distance, candidate) = candidates.iter()
        .map(|candidate| (edit_distance(s, candidate), candidate))
        .min_by_key(|(distance, _)| *distance)?;
    (distance <= s.chars().count() / 3).then(|| candidate.clone())
}

/// Returns the Levenshtein distance between two strings.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    // the distances from the prefix of `a` processed so far to each prefix of `b`
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitute = diagonal + (ca != cb) as usize;
            diagonal = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(diagonal + 1);
        }
    }
    row[b.len()]
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
