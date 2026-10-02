use std::path::Path;
use std::path::PathBuf;

const WS: &[char] = &[' ', '\n'];

// struct Error {
//     UnexpectedEof,
//     UnexpectedToken,
//     InvalidStatement,
// }

enum Severity {
    Warning,
    Error,
}
impl core::fmt::Display for Severity {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
        }
    }
}

#[derive(Debug)]
enum Msg {
    Eof,
    InvalidStatement,
}
impl Msg {
    pub fn severity(&self) -> Severity {
        match self {
            Self::Eof => Severity::Error,
            Self::InvalidStatement => Severity::Error,
        }
    }
}
impl core::fmt::Display for Msg {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::Eof => write!(f, "unexpected end of file"),
            Self::InvalidStatement => write!(f, "invalid statement"),
        }
    }
}

#[derive(Debug)]
pub struct Diagnostic {
    /// The ID of the file that emitted the diagnostic.
    id: usize,
    offset: usize,
    len: usize,
    msg: Msg,
    location: &'static std::panic::Location<'static>,
}

#[derive(Debug)]
enum Value {
    String(String),
    Path(PathBuf),
    Int(i64),
}

#[derive(Debug)]
enum Expression {
    Literal(Value),
}

#[derive(Debug)]
enum Scope {
    Local,
    Global,
}

#[derive(Debug)]
pub enum Statement {
    Use(PathBuf),
    Set {
        scope: Scope,
        value: Expression,
    },
}
impl Statement {
    fn parse(ctx: &mut Context, def: &BuildDef, s: &mut &str, offset: usize) -> Option<Self> {
        let Some(end) = s.find(WS) else {
            ctx.diagnostics.push(Diagnostic {
                id: def.id,
                offset,
                len: s.len(),
                msg: Msg::Eof,
                location: std::panic::Location::caller(),
            });
            return None;
        };
        match &s[..end] {
            "include" => {
                *s = &s[end..];
                return Some(Statement::Use("a".into()));
            },
            _ => {
                ctx.diagnostics.push(Diagnostic {
                    id: def.id,
                    offset,
                    len: end,
                    msg: Msg::InvalidStatement,
                    location: std::panic::Location::caller(),
                });
                return None;
            },
        }
    }
}

/// # Build Definition File
#[derive(Debug)]
pub struct BuildDef {
    id: usize,
    path: PathBuf,
    data: String,
    statements: Vec<Statement>,
}
impl BuildDef {
    pub fn new(id: usize, path: PathBuf, data: String) -> Self {
        Self {
            id,
            path,
            data,
            statements: Vec::new(),
        }
    }
    pub fn parse(&mut self, ctx: &mut Context) {
        let mut s = &self.data[..];
        while !s.is_empty() {
            s = s.trim_ascii_start();
            // safety: `s` is a substring of `data`
            let offset = unsafe { s.as_ptr().offset_from(self.data.as_ptr()) } as usize;
            let Some(statement) = Statement::parse(ctx, self, &mut s, offset) else {
                break;
            };
        }
    }
    pub fn position(&self, offset: usize) -> Option<(usize, usize)> {
        // find line
        let mut pos = 0;
        let mut line = 1;
        while pos < offset {
            if let Some(end) = self.data[pos..].find('\n') {
                pos += end + 1;
            } else {
                // end of file
                break;
            }
            line += 1;
        }
        println!("{offset}, {pos}");
        Some((line, offset + 1 - pos))
    }
}

#[derive(Debug)]
pub struct Context {
    definitions: Vec<BuildDef>,
    diagnostics: Vec<Diagnostic>,
}
impl Context {
    pub fn new() -> Self {
        Self {
            definitions: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
    pub fn load(&mut self, path: &Path) -> Result<&BuildDef, std::io::Error> {
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        let mut data = String::new();
        file.read_to_string(&mut data)?;
        println!("loaded {path:?}:\n{data}");
        let id = self.definitions.len();
        let mut definition = BuildDef::new(id, path.to_owned(), data);
        definition.parse(self);
        self.definitions.push(definition);
        let definition = &mut self.definitions[id];
        Ok(definition)
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    pub fn write_diagnostic(&self, w: &mut dyn std::io::Write, diagnostic: &Diagnostic) -> std::io::Result<()> {
        let build_def = &self.definitions[diagnostic.id];
        let (line, col) = build_def.position(diagnostic.offset).unwrap();
        writeln!(w, "{}: {}\nfrom {}\n --> {}:{line}:{col}", diagnostic.msg.severity(), diagnostic.msg, diagnostic.location, build_def.path.display())
    }
}

struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Parser<'a> {
    pub fn new(data: &'a str) -> Self {
        Self {
            data: data.as_bytes(),
            pos: 0,
        }
    }

    fn next_ident(&self) -> Option<&'a str> {
        let mut i = self.pos;
        while i < self.data.len() {
            // safety: bounds check above
            let c = unsafe { self.data.get_unchecked(i) };
            match c {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' => (),
                _ => break,
            }
            i += 1;
        }
        if i == 0 {
            return None;
        }
        // safety: original sequence is valid UTF-8 and this cuts on an ASCII boundary
        Some(unsafe { str::from_utf8_unchecked(self.data.get_unchecked(self.pos..i)) })
    }
}

#[cfg(test)]
mod test {
    use super::Parser;
    #[test]
    fn test_parser() {
        let p = Parser::new("hello world!");
        assert_eq!(p.next_ident(), Some("hello"));
        assert_eq!(p.next_ident(), Some("world"));
    }
}
