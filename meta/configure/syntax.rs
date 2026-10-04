use std::{collections::{HashMap, hash_map::Entry}, path::{Path, PathBuf}};

pub struct Project {
    files: Vec<File>,
    files_idx: HashMap<PathBuf, usize>,
    decls: Vec<Decl>,
    decls_idx: HashMap<String, usize>,
    pub diagnostics: Vec<Diagnostic>,
}
impl Project {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            files_idx: HashMap::new(),
            decls: Vec::new(),
            decls_idx: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }
    pub fn parse(&mut self, file: File) {
        let idx = self.files.len();
        self.files_idx.insert(file.path.clone(), idx);
        self.files.push(file);
        self.parse_file(idx);
    }
    /// Declare a property.
    pub fn declare(&mut self, name: String, decl: Decl) {
        let idx = self.decls.len();
        match self.decls_idx.entry(name) {
            Entry::Occupied(entry) => {
                let existing = *entry.get();
                self.diagnostics.push(Diagnostic {
                    span: decl.span,
                    message: Message::DeclExists { existing },
                });
                return;
            },
            Entry::Vacant(entry) => {
                entry.insert(idx);
            },
        }
        self.decls.push(decl);
    }
}

impl Project {
    fn parse_file(&mut self, idx: usize) {
        let file = &mut self.files[idx];
        let mut parser = Parser::new(idx, &file.data);
        while !parser.is_empty() {
            parser.skip_empty();
            let Some(keyword) = parser.keyword() else {
                let span = parser.skip_statement();
                self.diagnostics.push(Diagnostic {
                    span,
                    message: Message::ExpectedKeyword,
                });
                continue;
            };
            match keyword {
                "use" => {

            println!("keyword: {keyword}");
                }
                _ => {
                    let span = parser.skip_statement();
                    self.diagnostics.push(Diagnostic {
                        span,
                        message: Message::InvalidStatement,
                    });
                },
            }
        }
    }
}

pub struct File {
    path: PathBuf,
    data: String,
}
impl File {
    pub fn read(path: std::path::PathBuf) -> std::io::Result<Self> {
        Ok(Self {
            data: std::fs::read_to_string(&path)?,
            path,
        })
    }
    /// Returns the path relative to the file.
    pub fn relative_path(&self, path: &Path) -> PathBuf {
        self.path.parent().unwrap().join(path)
    }
}

pub struct Span {
    pub file: usize,
    pub offset: usize,
    pub len: usize,
}

pub struct Diagnostic {
    pub span: Span,
    pub message: Message,
}

#[repr(u32)]
pub enum Message {
    InvalidStatement,
    ExpectedKeyword,
    UnexpectedEof,
    DeclExists {
        existing: usize,
    },
}
impl core::fmt::Display for Message {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidStatement => write!(f, "invalid statement"),
            Self::ExpectedKeyword => write!(f, "expected keyword"),
            Self::UnexpectedEof => write!(f, "unexpected end of file"),
            Self::DeclExists { .. } => write!(f, "property has already been defined"),
        }
    }
}

/// A variable declaration.
pub struct Decl {
    pub span: Span,
    /// The ID of the current value.
    pub current: Option<u32>,
}

struct Var {
    id: u32,
    decl: u32,
    /// The previous value of the declaration.
    prev: Option<u32>,
    value: Value,
}


enum Statement {
    /// Set a variable
    Local {
        id: u32,
        value: Value,
    },
    Use(),
}

enum Value {
    Str(String),
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
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
    pub fn skip_empty(&mut self) {
        while let Some(b) = self.as_bytes().get(0) {
            match b {
                b'#' => {
                    let data = self.as_str();
                    let end = data.find('\n').unwrap_or(data.len());
                    self.offset += end;
                },
                _ => return,
            }

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
            match self.as_bytes().get(self.offset) {
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
    pub fn keyword(&mut self) -> Option<&'a str> {
        let data = self.data.as_bytes();
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
        let keyword = &self.data[self.offset..i];
        self.offset = i;
        Some(keyword)
    }
    pub fn span_eof(&self) -> Span {
        Span {
            file: self.file,
            offset: self.offset,
            len: self.data.len() - self.offset,
        }
    }
}
