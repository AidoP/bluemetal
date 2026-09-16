use std::path::Path;
use std::path::PathBuf;

const WS: &[char] = &[' ', '\n'];

// struct Error {
//     UnexpectedEof,
//     UnexpectedToken,
//     InvalidStatement,
// }

struct Spanned<T> {
    offset: usize,
    len: usize,
    value: T,
}

#[derive(Debug)]
enum Msg {
    Eof,
    InvalidStatement,
}

#[derive(Debug)]
struct Diagnostic {
    /// The ID of the file that emitted the diagnostic.
    id: usize,
    offset: usize,
    len: usize,
    msg: Msg,
}

enum Value {
    String(String),
    Path(PathBuf),
    Int(i64),
}

enum Expression {
    Literal(Value),
}

enum Scope {
    Local,
    Global,
}

pub enum Statement {
    Use(PathBuf),
    Set {
        scope: Scope,
        value: Expression,
    },
}
impl Statement {
    fn parse(ctx: &mut Context, def: &BuildDef, s: &str, offset: usize) -> Option<Spanned<Self>> {
        let Some(end) = s.find(WS) else {
            ctx.diagnostics.push(Diagnostic {
                id: def.id,
                offset,
                len: s.len(),
                msg: Msg::Eof,
            });
            return None;
        };
        match &s[..end] {
            _ => {
                ctx.diagnostics.push(Diagnostic {
                    id: def.id,
                    offset,
                    len: end,
                    msg: Msg::InvalidStatement,
                });
                return None;
            },
        }
    }
}

/// # Build Definition File
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
            let statement = Statement::parse(ctx, self, s, offset);
        }
    }
}

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
        self.definitions.push(BuildDef::new(id, path.to_owned(), data));
        let definition = &mut self.definitions[id];
        definition.parse(self);
        println!("diag: {:?}", self.diagnostics);
        Ok(definition)
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
