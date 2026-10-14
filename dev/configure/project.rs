use std::{collections::{HashMap, hash_map::Entry}, path::{Path, PathBuf}};

pub mod rust;

pub struct Project {
    root: PathBuf,
    pub files: Vec<File>,
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
}
impl Project {
    pub fn new(root: PathBuf, targets: Vec<String>) -> Self {
        Self {
            root,
            files: Vec::new(),
            files_idx: HashMap::new(),
            properties: Vec::new(),
            properties_idx: HashMap::new(),
            values: Vec::new(),
            values_idx: HashMap::new(),
            diagnostics: Vec::new(),
            targets,
            targets_listed: false,
            crates: HashMap::new(),
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
    pub fn get(&self, name: &str, file: usize) -> Option<&Data> {
        Some(&self.value(name, file)?.data)
    }
    fn value(&self, name: &str, file: usize) -> Option<&Value> {
        let mut idx = self.files[file].locals.get(name);
        if idx.is_none() {
            idx = self.values_idx.get(name);
        }
        Some(&self.values[*idx?])
    }
    /// Check for various conditions that cannot be evaluated as errors until reading has completed.
    /// - Missing declarations for `set` statements.
    /// - Missing crates for `use` attributes.
    pub fn check(&mut self) {
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
        for c in self.crates.values() {
            for (name, span) in &c.uses {
                if !self.crates.contains_key(&(name.clone(), c.target.clone())) {
                    self.diagnostics.push(Diagnostic {
                        span: Some(*span),
                        message: Message::UnknownCrate {
                            name: name.clone(),
                            target: c.target.clone(),
                        },
                        severity: Severity::Error,
                    });
                }
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

        writeln!(w, "{style_accent}{severity}{style_white}: {}{style_reset}", diagnostic.message)?;
        if let Some(span) = diagnostic.span.as_ref() {
            self.write_location(w, span, style_accent, style_reset)?;
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
            if !self.parse_meta(&mut parser, &mut chain) {
                parser.skip_statement();
                continue;
            }
            let start = parser.offset;
            let Some((keyword, span)) = self.parse_ident(&mut parser) else {
                parser.skip_statement();
                continue;
            };
            match keyword {
                "declare" => self.parse_declare(&mut parser),
                "use" => self.parse_use(&mut parser),
                "set" => self.parse_set(&mut parser),
                "log" => self.parse_log(&mut parser),
                "crate" => self.parse_crate(&mut parser),
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
        self.files[idx].data = data;
    }
    /// Take the meta-statements which precede a statement, returning whether the statement should be processed.
    ///
    /// Each meta-statement is a line starting with `@`, and must be followed directly by another meta-statement or by
    /// the statement it applies to. Once the statement is known to be skipped, later conditions are not evaluated.
    ///
    /// `chain` is whether a statement has been processed in the current chain of conditional statements, which `@else`
    /// continues, or [`None`] if the previous statement was unconditional.
    fn parse_meta(&mut self, parser: &mut Parser, chain: &mut Option<bool>) -> bool {
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
                return false;
            }
        }
        if failed {
            *chain = Some(true);
            return false;
        }
        *chain = match continues {
            Some(processed) => Some(processed || active),
            None if conditional => Some(active),
            None => None,
        };
        active
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
        let Some((string, span)) = self.parse_string(parser) else {
            parser.skip_statement();
            return;
        };
        if !self.parse_line_end(parser) {
            parser.skip_statement();
        }
        self.diagnostics.push(Diagnostic {
            span: Some(span),
            message: Message::User(string),
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
        let mut target = None;
        let mut uses = Vec::new();
        while let Some(_) = parser.indent() {
            let Some(attribute) = self.parse_attribute(parser, &["use"]) else {
                parser.skip_line();
                continue;
            };
            match attribute.name {
                "builtin" => builtin = true,
                "target" | "use" => match attribute.data {
                    Data::Str(string) if attribute.name == "target" => target = Some((string, attribute.data_span)),
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
        // without an attribute, the crate is compiled for the `target` property in effect at its declaration
        let (target, target_span) = target.unwrap_or_else(|| {
            (self.property("target", span).map_or_default(|(data, _)| data.coerce_string()), span)
        });
        self.check_target(&target, target_span);
        match self.crates.entry((name.clone(), target.clone())) {
            Entry::Occupied(_) => self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::RedeclareCrate {
                    name,
                    target,
                },
                severity: Severity::Error,
            }),
            Entry::Vacant(entry) => {
                entry.insert(rust::Crate {
                    name,
                    name_span: span,
                    path,
                    path_span,
                    target,
                    builtin,
                    uses,
                });
            },
        }
    }
    /// Report a target which the compiler does not support, listing the supported targets the first time.
    fn check_target(&mut self, target: &str, span: Span) {
        if self.targets.iter().any(|t| t == target) {
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
            Some('!') => {
                parser.expect('!');
                let operand = self.parse_operand(parser)?;
                let value = self.bool_operand(operand)?;
                Some(Operand { data: Data::Bool(!value), span: parser.span_from(start), coerce: Coercion::Strict, origin: None })
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
                Some(Operand { span: parser.span_from(start), coerce: Coercion::Any, ..inner })
            },
            // any word is parsed as an identifier, to report its invalid characters
            Some(c) if c.is_alphabetic() || c == '_' => {
                let (name, span) = self.parse_ident(parser)?;
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
            Operator::Add | Operator::Subtract | Operator::Multiply | Operator::Divide | Operator::Remainder => {
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
                            _ => $lhs.checked_rem($rhs),
                        }
                    };
                }
                let data = match (lhs, rhs) {
                    (Data::Unsigned(lhs), Data::Unsigned(rhs)) => checked!(lhs, rhs).map(Data::Unsigned),
                    (Data::Signed(lhs), Data::Signed(rhs)) => checked!(lhs, rhs).map(Data::Signed),
                    _ => unreachable!(),
                };
                let Some(data) = data else {
                    self.diagnostics.push(Diagnostic {
                        span: Some(span),
                        message: Message::IntegerOverflow,
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
            let (value, span, _) = self.parse_name(parser)?;
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
            // the expression may itself contain `{}` to coerce a property
            let mut depth = 0usize;
            let end = after.find(|c| {
                match c {
                    '{' => depth += 1,
                    '}' if depth == 0 => return true,
                    '}' => depth -= 1,
                    _ => (),
                }
                false
            });
            let Some(end) = end else {
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

/// Returns whether a character, at index `i`, cannot be part of an identifier.
fn invalid_ident_char(&(i, c): &(usize, char)) -> bool {
    match c {
        'a'..='z' | '-' | '_' => false,
        // a leading digit would be an integer
        '0'..='9' => i == 0,
        _ => true,
    }
}

/// An evaluated part of an expression.
struct Operand {
    data: Data,
    span: Span,
    coerce: Coercion,
    /// Where the value was set, if the operand is the value of a property.
    origin: Option<Span>,
}

/// How far an operand may be converted to the type an operator requires.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Coercion {
    /// The operand must already have the required type.
    Strict,
    /// An integer literal, which may change between signed and unsigned.
    Sign,
    /// An expression in `{}`, which may be converted to any type.
    Any,
}

#[derive(Clone, Copy)]
enum Operator {
    Or,
    And,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
}
impl Operator {
    const fn precedence(self) -> u8 {
        match self {
            Self::Or => 0,
            Self::And => 1,
            Self::Equal | Self::NotEqual | Self::Less | Self::LessEqual | Self::Greater | Self::GreaterEqual => 2,
            Self::Add | Self::Subtract => 3,
            Self::Multiply | Self::Divide | Self::Remainder => 4,
        }
    }
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
pub struct Span {
    pub file: usize,
    pub offset: usize,
    pub len: usize,
}

#[derive(Clone, Copy)]
pub enum Severity {
    Error,
    Warning,
    Note,
}
impl Severity {
    pub const fn accent(&self) -> &'static str {
        match self {
            Self::Error => "\x1b[91m\x1b[1m",
            Self::Warning => "\x1b[93m\x1b[1m",
            Self::Note => "\x1b[96m\x1b[1m",
        }
    }
}
impl core::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
            Severity::Note => write!(f, "note"),
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
    ElseWithoutIf(String),
    ElseNotFirst(String),
    UnsetProperty(String),
    ExpectedIdentifier,
    InvalidIdentifier {
        name: String,
        character: char,
    },
    ExpectedString,
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
            Self::ExpectedStatementAfterMeta => write!(f, "expected statement on the line after a meta-statement"),
            Self::ElseWithoutIf(name) => write!(f, "\"@{name}\" must follow a conditional statement"),
            Self::ElseNotFirst(name) => write!(f, "\"@{name}\" must be the first meta-statement of a statement"),
            Self::UnsetProperty(name) => write!(f, "no value for property {name:?}"),
            Self::ExpectedIdentifier => write!(f, "expected identifier"),
            Self::InvalidIdentifier { name, character } => write!(f, "invalid character {character:?} in identifier {name:?}"),
            Self::ExpectedString => write!(f, "expected string"),
            Self::InvalidStringTemplate { .. } => write!(f, "invalid string template"),
            Self::InvalidInteger => write!(f, "invalid integer"),
            Self::InvalidOperand { ty, expected, coerce: false, .. } => write!(f, "{ty} value used where {expected} is required"),
            Self::InvalidOperand { ty, expected, coerce: true, .. } => write!(f, "{ty} value cannot be coerced to {expected}"),
            Self::MismatchedOperands { lhs, rhs, .. } => write!(f, "mismatched operand types {lhs} and {rhs}"),
            Self::IntegerOverflow => write!(f, "integer overflow"),
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
                    write!(f, "\n    {target}")?;
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

pub struct Property {
    span: Option<Span>,
    name: String,
    ty: Type,
    /// Whether `set` is forbidden, leaving the property with its default or builtin value.
    read_only: bool,
    /// A description of the property, included in diagnostics about it.
    info: Option<String>,
}
impl Property {
    pub fn new(name: String, ty: Type) -> Self {
        Self {
            span: None,
            name,
            ty,
            read_only: false,
            info: None,
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn info(self, info: &str) -> Self {
        Self { info: Some(info.into()), ..self }
    }
    pub fn read_only(self) -> Self {
        Self { read_only: true, ..self }
    }
}

pub enum Type {
    Any,
    String,
    Bool,
    Unsigned,
    Signed,
}
impl Type {
    /// Returns the data if it has this type, or returns it as an error.
    ///
    /// An unsigned integer is converted for a signed type if it is in range, as a literal without a sign is unsigned.
    fn accept(&self, data: Data) -> Result<Data, Data> {
        match (self, data) {
            (Type::Any, data)
            | (Type::String, data @ Data::Str(_))
            | (Type::Bool, data @ Data::Bool(_))
            | (Type::Unsigned, data @ Data::Unsigned(_))
            | (Type::Signed, data @ Data::Signed(_)) => Ok(data),
            (Type::Signed, Data::Unsigned(i)) => i64::try_from(i).map(Data::Signed).map_err(|_| Data::Unsigned(i)),
            (_, data) => Err(data),
        }
    }
}

pub struct Value {
    property: usize,
    span: Option<Span>,
    data: Data,
    data_span: Option<Span>,
    /// Where the data was written, which is the origin of a referenced property rather than the reference itself.
    origin: Option<Span>,
    local: bool,
}
impl Value {
    pub fn new(property: usize, data: Data) -> Self {
        Self {
            property,
            span: None,
            data,
            data_span: None,
            origin: None,
            local: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    None,
    Str(String),
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
}
impl Data {
    /// Returns the name of the type as used by `declare`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Str(_) => "str",
            Self::Bool(_) => "bool",
            Self::Unsigned(_) => "unsigned",
            Self::Signed(_) => "signed",
        }
    }
    pub fn is_integer(&self) -> bool {
        matches!(self, Self::Unsigned(_) | Self::Signed(_))
    }
    /// Convert to an integer: a `bool` is `0` or `1` and a `str` is parsed.
    pub fn coerce_integer(&self) -> Option<Self> {
        match self {
            Self::None => None,
            Self::Str(string) => {
                let mut parser = Parser::new(0, string);
                match parser.integer() {
                    Some((data, _)) if parser.is_empty() => Some(data),
                    _ => None,
                }
            },
            Self::Bool(value) => Some(Self::Unsigned(*value as u64)),
            Self::Unsigned(_) | Self::Signed(_) => Some(self.clone()),
        }
    }
    /// Convert to a boolean: an integer is true if it is not zero and a `str` must be `true` or `false`.
    pub fn coerce_bool(&self) -> Option<bool> {
        match self {
            Self::None => None,
            Self::Str(string) => match string.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            Self::Bool(value) => Some(*value),
            Self::Unsigned(i) => Some(*i != 0),
            Self::Signed(i) => Some(*i != 0),
        }
    }
    /// Convert to the type of `other`.
    pub fn coerce_like(&self, other: &Self) -> Option<Self> {
        match other {
            Self::None => None,
            Self::Str(_) => Some(Self::Str(self.coerce_string())),
            Self::Bool(_) => self.coerce_bool().map(Self::Bool),
            Self::Unsigned(_) | Self::Signed(_) => {
                let integer = self.coerce_integer()?;
                if integer.type_name() == other.type_name() {
                    Some(integer)
                } else {
                    integer.convert_sign()
                }
            },
        }
    }
    /// Convert between a signed and unsigned integer, if the value can be represented.
    pub fn convert_sign(&self) -> Option<Self> {
        match self {
            Self::Unsigned(i) => i64::try_from(*i).ok().map(Self::Signed),
            Self::Signed(i) => u64::try_from(*i).ok().map(Self::Unsigned),
            _ => None,
        }
    }
    pub fn coerce_string(&self) -> String {
        match self {
            Self::None => String::new(),
            Self::Str(s) => s.clone(),
            Self::Bool(b) => b.to_string(),
            Self::Unsigned(i) => i.to_string(),
            Self::Signed(i) => i.to_string(),
        }
    }
}


struct Parser<'a> {
    file: usize,
    data: &'a str,
    offset: usize,
}

impl <'a> Parser<'a> {
    pub fn new(file: usize, data: &'a str) -> Self {
        Self {
            file,
            data,
            offset: 0,
        }
    }
    pub fn as_str(&self) -> &'a str {
        &self.data[self.offset..]
    }
    pub fn as_bytes(&self) -> &'a [u8] {
        self.data[self.offset..].as_bytes()
    }
    pub fn is_empty(&self) -> bool {
        self.offset >= self.data.len()
    }
    pub fn peek(&mut self) -> Option<char> {
        self.as_str().chars().next()
    }
    pub fn expect(&mut self, c: char) -> bool {
        match self.as_str().chars().next() {
            None => false,
            Some(other) if other != c => false,
            Some(other) => {
                self.offset += other.len_utf8();
                true
            },
        }
    }
    pub fn skip_empty(&mut self) {
        while let Some(b) = self.as_bytes().get(0) {
            match b {
                b'#' => {
                    self.skip_statement();
                },
                b'\n' => self.offset += 1,
                _ => return,
            }
        }
    }
    pub fn skip_line(&mut self) -> Span {
        let start = self.offset;
        match self.as_str().find('\n') {
            Some(end) => self.offset += end + 1,
            None => self.offset = self.data.len(),
        }
        Span {
            file: self.file,
            offset: start,
            len: self.offset - start,
        }
    }
    pub fn skip_statement(&mut self) -> Span {
        let offset = self.offset;
        while self.offset < self.data.len() {
            // continue from the next non-indented line
            if let Some(end) = self.as_str().find('\n') {
                self.offset += end + 1;
            } else {
                self.offset = self.data.len();
            }
            match self.as_bytes().get(0) {
                Some(b' ') => (),
                _ => break,
            }
        }
        Span {
            file: self.file,
            offset,
            len: self.offset - offset,
        }
    }
    pub fn indent(&mut self) -> Option<Span> {
        if self.as_str().starts_with("    ") {
            let offset = self.offset;
            self.offset += 4;
            Some(Span {
                file: self.file,
                offset,
                len: 4,
            })
        } else {
            None
        }
    }
    pub fn ident(&mut self) -> Option<(&'a str, Span)> {
        let data = self.data.as_bytes();
        let start = self.offset;
        let mut i = self.offset;
        while i < data.len() {
            match data[i] {
                b'a'..=b'z' | b'-' | b'_' => i += 1,
                // a leading digit would be an integer
                b'0'..=b'9' if i > start => i += 1,
                _ => break,
            }
        }
        if i == self.offset {
            return None;
        }
        let ident = &self.data[self.offset..i];
        self.offset = i;
        Some((ident, Span { file: self.file, offset: start, len: ident.len() }))
    }
    /// An operator with a single space on either side.
    ///
    /// The operator is only taken if its precedence is at least `min_precedence`.
    pub fn operator(&mut self, min_precedence: u8) -> Option<Operator> {
        let rest = self.as_str().strip_prefix(' ')?;
        let end = rest.find(' ')?;
        let operator = match &rest[..end] {
            "||" => Operator::Or,
            "&&" => Operator::And,
            "==" => Operator::Equal,
            "!=" => Operator::NotEqual,
            "<" => Operator::Less,
            "<=" => Operator::LessEqual,
            ">" => Operator::Greater,
            ">=" => Operator::GreaterEqual,
            "+" => Operator::Add,
            "-" => Operator::Subtract,
            "*" => Operator::Multiply,
            "/" => Operator::Divide,
            "%" => Operator::Remainder,
            _ => return None,
        };
        if operator.precedence() < min_precedence {
            return None;
        }
        self.offset += end + 2;
        Some(operator)
    }
    /// An integer with an optional `-` or `+` sign.
    ///
    /// The prefixes `0x`, `0o` and `0b` select hexadecimal, octal and binary, otherwise it is decimal.
    /// Digits may be separated with `_`, but the first character must be a digit.
    /// Returns [`Data::Signed`] if the sign is `-` and [`Data::Unsigned`] otherwise.
    pub fn integer(&mut self) -> Option<(Data, Span)> {
        let s = self.as_str();
        let (negative, unsigned) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        let (radix, digits) = if let Some(rest) = unsigned.strip_prefix("0x") {
            (16, rest)
        } else if let Some(rest) = unsigned.strip_prefix("0o") {
            (8, rest)
        } else if let Some(rest) = unsigned.strip_prefix("0b") {
            (2, rest)
        } else {
            (10, unsigned)
        };
        let end = digits.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(digits.len());
        if digits.starts_with('_') {
            return None;
        }
        let magnitude = u64::from_str_radix(&digits[..end].replace('_', ""), radix).ok()?;
        let data = if negative {
            Data::Signed(0i64.checked_sub_unsigned(magnitude)?)
        } else {
            Data::Unsigned(magnitude)
        };
        let start = self.offset;
        self.offset += s.len() - digits.len() + end;
        Some((data, self.span_from(start)))
    }
    /// A `'` quoted string on a single line.
    pub fn string(&mut self) -> Option<(&'a str, Span)> {
        let data = self.as_str();
        let rest = data.strip_prefix('\'')?;
        let end = rest.find(['\'', '\n'])?;
        if !rest[end..].starts_with('\'') {
            return None;
        }
        let start = self.offset;
        self.offset += end + 2;
        Some((&rest[..end], self.span_from(start)))
    }
    pub fn span_char(&self) -> Span {
        Span {
            file: self.file,
            offset: self.offset,
            len: self.as_str().chars().next().map_or_default(|c| c.len_utf8()),
        }
    }
    /// Returns the span from `start` to the current offset.
    pub fn span_from(&self, start: usize) -> Span {
        Span {
            file: self.file,
            offset: start,
            len: self.offset - start,
        }
    }
    pub fn span_until(&self, pat: impl Fn(char) -> bool) -> Span {
        let len = self.as_str().find(pat).unwrap_or(self.data.len() - self.offset);
        Span {
            file: self.file,
            offset: self.offset,
            len,
        }
    }
}
