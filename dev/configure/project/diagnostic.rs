//! Diagnostics, and how they are written for a person to read.

use std::path::PathBuf;

use super::{File, Project, Property, Span};

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
pub(super) fn suggest(s: &str, candidates: &[String]) -> Option<String> {
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

impl Project {
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
                    write_replacement(w, &self.files[span.file], span, replacement, style_accent, style_reset)?;
                },
                _ => write_location(w, &self.files[span.file], span, style_accent, style_reset)?,
            }
        }
        match &diagnostic.message {
            Message::RedeclareProperty { id, .. } => {
                if let Some(span) = &self.properties[*id].as_ref().unwrap().span {
                    writeln!(w, "{style_note}note{style_white}: previously declared here{style_reset}")?;
                    write_location(w, &self.files[span.file], span, style_note, style_reset)?;
                }
            },
            Message::ReadOnlyProperty { id, assigned, .. } => {
                if let Some(span) = assigned {
                    writeln!(w, "{style_note}note{style_white}: value assigned here{style_reset}")?;
                    write_location(w, &self.files[span.file], span, style_note, style_reset)?;
                }
                if let Some(span) = &self.properties[*id].as_ref().unwrap().span {
                    writeln!(w, "{style_note}note{style_white}: declared read-only here{style_reset}")?;
                    write_location(w, &self.files[span.file], span, style_note, style_reset)?;
                }
            },
            Message::UseBinary { declared, .. } | Message::UseWithTarget { declared: Some(declared) } => {
                writeln!(w, "{style_note}note{style_white}: declared here{style_reset}")?;
                write_location(w, &self.files[declared.file], declared, style_note, style_reset)?;
            },
            Message::RedeclareCustom { previous, .. } => {
                writeln!(w, "{style_note}note{style_white}: previously declared here{style_reset}")?;
                write_location(w, &self.files[previous.file], previous, style_note, style_reset)?;
            },
            Message::RepeatedUse { file, .. } => {
                if let Some(span) = &self.files[*file].used {
                    writeln!(w, "{style_note}note{style_white}: first used here{style_reset}")?;
                    write_location(w, &self.files[span.file], span, style_note, style_reset)?;
                }
            },
            Message::RedeclareCrate { name, target } => {
                let span = &self.crates[&(name.clone(), target.clone())].name_span;
                writeln!(w, "{style_note}note{style_white}: previously declared here{style_reset}")?;
                write_location(w, &self.files[span.file], span, style_note, style_reset)?;
            },
            Message::UnknownDeclarationAttribute { property, .. } => {
                let property = self.properties[*property].as_ref().unwrap();
                if let Some(span) = &property.span {
                    writeln!(w, "{style_note}note{style_white}: attribute specified for declaration of {:?}{style_reset}", property.name)?;
                    write_location(w, &self.files[span.file], span, style_note, style_reset)?;
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
                write_location(w, &self.files[span.file], span, style_note, style_reset)?;
            },
            Message::MismatchedOperands { lhs, rhs, lhs_origin, rhs_origin } => {
                for (ty, origin) in [(lhs, lhs_origin), (rhs, rhs_origin)] {
                    if let Some(span) = origin {
                        writeln!(w, "{style_note}note{style_white}: {ty} value set here{style_reset}")?;
                        write_location(w, &self.files[span.file], span, style_note, style_reset)?;
                    }
                }
            },
            Message::MismatchedType { value, .. } => {
                let value = &self.values[*value];
                // the value of a referenced property was set elsewhere
                if let Some(origin) = value.origin && value.data_span.is_some_and(|span| span.offset != origin.offset || span.file != origin.file) {
                    writeln!(w, "{style_note}note{style_white}: {} value set here{style_reset}", value.data.type_name())?;
                    write_location(w, &self.files[origin.file], &origin, style_note, style_reset)?;
                }
                if let Some(span) = &self.properties[value.property].as_ref().unwrap().span {
                    writeln!(w, "{style_note}note{style_white}: declared here{style_reset}")?;
                    write_location(w, &self.files[span.file], span, style_note, style_reset)?;
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
}

/// Write the location of a span followed by its first line with the span replaced, and the replacement underlined.
fn write_replacement(
    w: &mut dyn std::io::Write,
    file: &File,
    span: &Span,
    replacement: &str,
    style_accent: &str,
    style_reset: &str,
) -> std::io::Result<()> {
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

/// Write the location of a span followed by its first line with the span underlined.
fn write_location(
    w: &mut dyn std::io::Write,
    file: &File,
    span: &Span,
    style_accent: &str,
    style_reset: &str,
) -> std::io::Result<()> {
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
