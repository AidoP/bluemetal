use std::{collections::{HashMap, hash_map::Entry}, path::{Path, PathBuf}};

mod rust;

pub struct Project {
    root: PathBuf,
    files: Vec<File>,
    files_idx: HashMap<PathBuf, usize>,
    properties: Vec<Option<Property>>,
    properties_idx: HashMap<String, usize>,
    values: Vec<Value>,
    values_idx: HashMap<String, usize>,
    pub diagnostics: Vec<Diagnostic>,

    pub crates: HashMap<String, rust::Crate>,
}
impl Project {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            files: Vec::new(),
            files_idx: HashMap::new(),
            properties: Vec::new(),
            properties_idx: HashMap::new(),
            values: Vec::new(),
            values_idx: HashMap::new(),
            diagnostics: Vec::new(),
            crates: HashMap::new(),
        }
    }
    pub fn file(&self, idx: usize) -> &File {
        &self.files[idx]
    }
    pub fn parse(&mut self, file: File) {
        let idx = self.files.len();
        self.files_idx.insert(file.path.clone(), idx);
        self.files.push(file);
        self.parse_file(idx);
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
                    Some(_) => self.diagnostics.push(Diagnostic {
                        span: Some(property_ref.span),
                        message: Message::RedeclareProperty { id },
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
        let file = &mut self.files[value.span.file];
        if value.local {
            file.locals.insert(name.clone(), id);
        } else {
            let _ = file.locals.remove(&name);
        }
        self.values.push(value);
        self.values_idx.insert(name, id);
        id
    }
    pub fn get(&self, name: &str, file: usize) -> Option<&Data> {
        let mut idx = self.files[file].locals.get(name);
        if idx.is_none() {
            idx = self.values_idx.get(name);
        }
        Some(&self.values[*idx?].data)
    }
    /// Check for various conditions that cannot be evaluated as errors until reading has completed.
    /// - Missing declarations for `set` statements.
    /// - Invalid data types for `set` statements.
    pub fn check(&mut self) {
        for (name, &idx) in &self.values_idx {
            let value = &self.values[idx];
            let Some(property) = &self.properties[value.property] else {
                self.diagnostics.push(Diagnostic {
                    span: Some(value.span),
                    message: Message::UndeclaredProperty(name.clone()),
                    severity: Severity::Error,
                });
                continue;
            };
            match (&property.ty, &value.data) {
                (Type::Any, _) => (),
                (Type::Bool, Data::Bool(_)) => (),
                (Type::String, Data::Str(_)) => (),
                (Type::Unsigned, Data::Unsigned(_)) => (),
                (Type::Signed, Data::Signed(_)) => (),
                _ => self.diagnostics.push(Diagnostic {
                    span: Some(value.data_span),
                    message: Message::MismatchedType { value: idx, name: name.clone() },
                    severity: Severity::Error,
                }),
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
            Message::RedeclareProperty { id } => {
                writeln!(w, "{style_note}note{style_white}: previously declared here{style_reset}")?;
                self.write_location(w, &self.properties[*id].as_ref().unwrap().span, style_note, style_reset)?;
            },
            Message::UnknownDeclarationAttribute { property, .. } => {
                let property = self.properties[*property].as_ref().unwrap();
                writeln!(w, "{style_note}note{style_white}: attribute specified for declaration of {:?}{style_reset}", property.name)?;
                self.write_location(w, &property.span, style_note, style_reset)?;
            },
            Message::MismatchedType { value, .. } => {
                let value = &self.values[*value];
                writeln!(w, "{style_note}note{style_white}: previously declared here{style_reset}")?;
                self.write_location(w, &self.properties[value.property].as_ref().unwrap().span, style_note, style_reset)?;
            },
            _ => (),
        }
        Ok(())
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
        loop {
            parser.skip_empty();
            if parser.is_empty() {
                break;
            }
            let start = parser.offset;
            let Some((keyword, span)) = parser.ident() else {
                let span = parser.skip_statement();
                self.diagnostics.push(Diagnostic {
                    span: Some(span),
                    message: Message::ExpectedIdentifier,
                    severity: Severity::Error,
                });
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
            Ok(file) => {
                self.parse(file);
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
        let Some((ident, span)) = parser.ident() else {
            let span = parser.skip_statement();
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::ExpectedIdentifier,
                severity: Severity::Error,
            });
            return;
        };
        let idx = self.declare(ident.into(), None);
        if !self.parse_line_end(parser) {
            parser.skip_statement();
        }
        let mut ty = Type::Any;
        while let Some(_) = parser.indent() {
            let Some(attribute) = self.parse_attribute(parser) else {
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
                "default" => {
                    self.set(ident.into(), Value {
                        property: idx,
                        span: attribute.name_span,
                        data: attribute.data,
                        data_span: attribute.data_span,
                        local: false,
                    });
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
        let property = &mut self.properties[idx];
        if property.is_some() {
            return;
        }
        *property = Some(Property {
            span,
            name: ident.into(),
            ty,
        });
    }
    fn parse_set(&mut self, parser: &mut Parser) {
        if !self.parse_space(parser) {
            parser.skip_statement();
            return;
        }
        let Some((ident, span)) = parser.ident() else {
            let span = parser.skip_statement();
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::ExpectedIdentifier,
                severity: Severity::Error,
            });
            return;
        };
        if !self.parse_space(parser) {
            parser.skip_statement();
            return;
        }
        let Some((data, data_span)) = self.parse_expression(parser) else {
            parser.skip_statement();
            return;
        };
        if !self.parse_line_end(parser) {
            parser.skip_statement();
        }
        let property = self.declare(ident.into(), None);
        let value = Value {
            property,
            span,
            data,
            data_span,
            local: false,
        };
        self.set(ident.into(), value);
    }
    fn parse_log(&mut self, parser: &mut Parser) {
        if !self.parse_space(parser) {
            parser.skip_statement();
            return;
        }
        let Some((severity, span)) = parser.ident() else {
            let span = parser.skip_statement();
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::ExpectedIdentifier,
                severity: Severity::Error,
            });
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
        let Some((name, span)) = parser.ident() else {
            let span = parser.skip_statement();
            self.diagnostics.push(Diagnostic {
                span: Some(span),
                message: Message::ExpectedIdentifier,
                severity: Severity::Error,
            });
            return;
        };
        let path;
        let path_span;
        if parser.peek() == Some(' ') && self.parse_space(parser) && let Some((p, s)) = self.parse_path(parser) {
            path = p;
            path_span = s;
        } else {
            let mut rel_path = self.files[parser.file].relative_path(name);
            rel_path.push("lib.rs");
            path = rel_path;
            path_span = span;
        }
        if !self.parse_line_end(parser) {
            parser.skip_statement();
        }
        let mut builtin = false;
        let mut sysroot = false;
        while let Some(_) = parser.indent() {
            let Some(attribute) = self.parse_attribute(parser) else {
                parser.skip_line();
                continue;
            };
            match attribute.name {
                "builtin" => builtin = true,
                "sysroot" => sysroot = true,
                name => self.diagnostics.push(Diagnostic {
                    span: Some(attribute.name_span),
                    message: Message::UnknownCrateAttribute {
                        name: name.into(),
                    },
                    severity: Severity::Error,
                }),
            }
        }
        self.crates.insert(name.into(), rust::Crate {
            name: name.into(),
            name_span: span,
            path,
            path_span,
            sysroot,
            builtin,
        });
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
            let path = self.evaluate_str(&path[1..], span);
            path_buf = self.root.join(path);
        } else {
            let path = self.evaluate_str(path, span);
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
        Some((self.evaluate_str(string, span), span))
    }
    fn parse_expression(&mut self, parser: &mut Parser) -> Option<(Data, Span)> {
        match parser.peek() {
            Some('\'') => if let Some((string , span)) = parser.string() {
                let string = self.evaluate_str(string, span);
                Some((Data::Str(string), span))
            } else {
                None
            },
            Some('t' | 'f') => if let Some((value , span)) = parser.bool() {
                Some((Data::Bool(value), span))
            } else {
                None
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
    fn parse_attribute<'a>(&mut self, parser: &mut Parser<'a>) -> Option<Attribute<'a>> {
        let Some((name, name_span)) = parser.ident() else {
            self.diagnostics.push(Diagnostic {
                span: Some(parser.span_until(|c| c == ' ' || c == '\n')),
                message: Message::ExpectedIdentifier,
                severity: Severity::Error,
            });
            return None;
        };
        let (data, data_span) = if parser.expect(' ') {
            let Some((data, data_span)) = self.parse_expression(parser) else {
                return None;
            };
            (data, data_span)
        } else {
            (Data::None, name_span)
        };
        if !self.parse_line_end(parser) {
            parser.skip_line();
        }
        Some(Attribute {
            name,
            name_span,
            data,
            data_span,
        })
    }
    /// Evaluate string templates
    fn evaluate_str(&mut self, s: &str, span: Span) -> String {
        let mut string = String::new();
        let mut chars = s.char_indices().peekable();
        let mut name_start = None;
        while let Some((i, c)) = chars.next() {
            match c {
                '{' => {
                    match chars.next() {
                        Some((_, '{')) => string.push(c),
                        _ => {
                            name_start = Some(i+1);
                        },
                    }
                },
                '}' => {
                    match chars.peek() {
                        Some((_, '}')) => {
                            chars.next();
                            string.push(c);
                        },
                        _ => {
                            let Some(start) = name_start else {
                                self.diagnostics.push(Diagnostic {
                                    span: Some(Span { file: span.file, offset: span.offset + i, len: 1 }),
                                    message: Message::InvalidStringTemplate,
                                    severity: Severity::Error,
                                });
                                continue;
                            };
                            let name = &s[start..i];
                            name_start = None;
                            let Some(value) = self.get(name, span.file) else {
                                self.diagnostics.push(Diagnostic {
                                    span: Some(Span { file: span.file, offset: span.offset + start + 1, len: i - start }),
                                    message: Message::UnsetProperty(name.into()),
                                    severity: Severity::Error,
                                });
                                continue;
                            };
                            let value = value.coerce_string();
                            string.push_str(&value);
                        },
                    }
                },
                'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' if name_start.is_some() => (),
                _ if name_start.is_some() => {
                    let start = name_start.unwrap();
                    self.diagnostics.push(Diagnostic {
                        span: Some(Span { file: span.file, offset: span.offset + start + 1, len: i + 1 - start }),
                        message: Message::InvalidStringTemplate,
                        severity: Severity::Error,
                    });
                    // consume the template literal otherwise it will error a second time
                    while let Some((_, c)) = chars.next() {
                        if c == '}' {
                            break;
                        }
                    }
                    name_start = None;
                },
                _ => string.push(c),
            }
        }
        string
    }
}

struct Attribute<'a> {
    name: &'a str,
    name_span: Span,
    data: Data,
    data_span: Span,
}

pub struct File {
    path: PathBuf,
    data: String,
    locals: HashMap<String, usize>,
}
impl File {
    pub fn read(path: std::path::PathBuf) -> std::io::Result<Self> {
        Ok(Self {
            data: std::fs::read_to_string(&path)?,
            path,
            locals: HashMap::new(),
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
    UnsetProperty(String),
    ExpectedIdentifier,
    ExpectedString,
    InvalidStringTemplate,
    InvalidType(String),
    ExpectedPath,
    ExpectedExpression,
    ReadFailure(PathBuf, std::io::Error),
    ExpectedExact(&'static str),
    UndeclaredProperty(String),
    RedeclareProperty {
        id: usize,
    },
    UnknownDeclarationAttribute {
        property: usize,
        name: String,
    },
    UnknownCrateAttribute {
        name: String,
    },
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
            Self::UnsetProperty(name) => write!(f, "no value for property {name:?}"),
            Self::ExpectedIdentifier => write!(f, "expected identifier"),
            Self::ExpectedString => write!(f, "expected string"),
            Self::InvalidStringTemplate { .. } => write!(f, "invalid string template"),
            Self::InvalidType(ty) => write!(f, "invalid data type {ty:?}"),
            Self::ExpectedPath => write!(f, "expected path"),
            Self::ExpectedExpression => write!(f, "expected expression"),
            Self::ExpectedExact(s) => write!(f, "expected {s:?}"),
            Self::ReadFailure(path, error) => write!(f, "failed to read {path:?}: {error}"),
            Self::UndeclaredProperty(name) => write!(f, "property {name:?} is undeclared"),
            Self::RedeclareProperty { .. } => write!(f, "redeclaration of property"),
            Self::UnknownDeclarationAttribute { name, .. } => write!(f, "unknown declare attribute {name:?}"),
            Self::UnknownCrateAttribute { name, .. } => write!(f, "unknown crate attribute {name:?}"),
            Self::MismatchedType { name, .. } => write!(f, "invalid data type for property {name:?}"),
        }
    }
}

pub struct Property {
    span: Span,
    name: String,
    ty: Type,
}

pub enum Type {
    Any,
    String,
    Bool,
    Unsigned,
    Signed,
}

pub struct Value {
    property: usize,
    span: Span,
    data: Data,
    data_span: Span,
    local: bool,
}

#[derive(Debug)]
pub enum Data {
    None,
    Str(String),
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
}
impl Data {
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
                b'a'..=b'z' | b'-' => i += 1,

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
    /// Either `true` or `false`.
    pub fn bool(&mut self) -> Option<(bool, Span)> {
        let s = self.as_str();
        const TRUE: &str = "true";
        const FALSE: &str = "false";
        if s.starts_with(TRUE) {
            let span = Span { file: self.file, offset: self.offset, len: TRUE.len() };
            self.offset += span.len;
            return Some((true, span));
        }
        if s.starts_with(FALSE) {
            let span = Span { file: self.file, offset: self.offset, len: FALSE.len() };
            self.offset += span.len;
            return Some((false, span));
        }
        None
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
