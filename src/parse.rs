use std::{
    collections::{HashSet, HashMap, VecDeque},
    cmp::{PartialEq, Eq},
    fmt::{self, Display},
    fs::File,
    hash::{Hash, Hasher},
    io::{self, Read},
    ops::{Deref, Sub},
    path::Path,
};

macro_rules! consume {
    ($pos:expr, $string:expr => $length:expr) => {
        {
            let span = Span(*$pos, $length);
            *$pos += $length;
            let new_ref = &$string[..$length];
            *$string = &$string[$length..];
            Spanning(span, new_ref.to_owned())
        }
    };
}

fn parse_symbol(symbol: &str, from: &mut &str, pos: &mut usize) -> Result<impl Spans, ParseError> {
    if let Some(stripped) = from.strip_prefix(symbol) {
        let dif = from.len() - stripped.len();
        Ok(consume!(pos, from => dif))
    } else {
        Err(ParseError::ExpectedSymbol(symbol.into(), *pos))
    }
}
fn strip_whitespace(from: &mut &str, pos: &mut usize) {
    let len = from.len();
    *from = from.trim_start();
    *pos += len - from.len();
}

#[derive(Debug, Clone)]
struct Identifier(Spanning<String>);
impl Identifier {
    fn named(name: &str) -> Self {
        Self(Spanning(Span(0,0), name.into()))
    }
}
impl Deref for Identifier {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        (self.0).1.as_str()
    }
}
impl Identifier {
    fn parse<'a>(from: &mut &'a str, pos: &mut usize) -> Result<Self, ParseError> {
        let mut chars = from.chars();
        if let Some(c) = chars.next() {
            let mut len = 1;
            match c {
                'a'..='z' | 'A'..='Z' | '_' => (),
                _ => return Err(ParseError::ExpectedIdentifier(*pos))
            }
            for c in chars {
                match c {
                    'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => len+=1,
                    _ => break
                }
            }
            Ok(Self(consume!(pos, from => len)))
        } else {
            Err(ParseError::ExpectedIdentifier(*pos))
        }
    }
}
impl Spans for Identifier {
    fn span(&self) -> Span {
        self.0.span()
    }
}

pub enum Global {
    Function,
    External
}

pub struct Module {
    pub functions: HashMap<String, Function>,
    pub structures: HashMap<String, Structure>
}
impl Module {
    /// Parse a module into a tree structure, returning other required modules as well as 
    pub fn parse(from: &mut &str, unit: &mut Unit) -> Result<(Vec<String>, Self), ParseError> {
        let mut this = Self::default();
        let mut modules = vec![];
        let mut pos = 0;
        let mut qualifiers = HashMap::new();
        while from.len() > 0 {
            strip_whitespace(from, &mut pos);
            let qualifier = Qualifier::parse(from, &mut pos)?;
            let qualifier_name = qualifier.name.to_string();
            if let Some(other_qualifier) = qualifiers.insert(qualifier_name.clone(), qualifier) {
                return Err(ParseError::DuplicateQualifier(qualifiers.remove(&qualifier_name).unwrap(), other_qualifier))
            }
            match qualifier_name.as_str() {
                "fn" => {
                    let function = Function::parse(qualifiers, from, &mut pos)?;
                    let function_name = function.name.to_string();
                    let function_span = function.span();
                    if let Some(old_function) = this.functions.insert(function_name.clone(), function) {
                        return Err(ParseError::DuplicateFunction(function_name, function_span, old_function.span()))
                    }
                    qualifiers = HashMap::new();
                }
                "struct" => {
                    let structure = Structure::parse(qualifiers, from, &mut pos)?;
                    let structure_name = structure.name.to_string();
                    let structure_span = structure.span();
                    if let Some(old_structure) = this.structures.insert(structure_name.clone(), structure) {
                        return Err(ParseError::DuplicateStructure(structure_name, structure_span, old_structure.span()))
                    }
                    qualifiers = HashMap::new();
                }
                _ => ()
            }
        }
        if !qualifiers.is_empty() {
            Err(ParseError::UnexpectedEOF)
        } else {
            Ok((modules, this))
        }
    }
}
impl Default for Module {
    fn default() -> Self {
        Self {
            functions: Default::default(),
            structures: Default::default()
        }
    }
}

pub struct Unit {
    modules: HashMap<String, Module>,
    //globals: HashMap<String, Global>
}
impl Unit {
    pub fn parse<P: AsRef<Path>>(root: P, initial_module: &str) -> Result<Self, ParseError> {
        let mut this = Self::default();
        let mut to_parse: VecDeque<String> = VecDeque::default();
        to_parse.push_back(initial_module.into());

        while let Some(module_name) = to_parse.pop_front() {
            if !this.modules.contains_key(&module_name) {
                let mut path = root.as_ref().to_path_buf();
    
                let mut module_path = module_name.replace("::", "/");
                if module_path.find('.').is_some() { return Err(ParseError::ModuleName(module_name)) }
                module_path.push_str(".bm");
                path.push(module_path);
                let mut file = File::open(path)?;
                let mut contents = String::new();
                file.read_to_string(&mut contents)?;
    
                let (import_modules, module) = Module::parse(&mut contents.as_str(), &mut this).unwrap();
                to_parse.extend(import_modules);
                this.modules.insert(module_name, module);
            }
        }


        Ok(this)
    }
    pub fn register_function() {}
    pub fn register_type() {}
}
impl Default for Unit {
    fn default() -> Self {
        Self {
            modules: Default::default()
        }
    }
}
impl std::ops::Index<&str> for Unit {
    type Output = Module;
    fn index(&self, index: &str) -> &Self::Output {
        &self.modules[index]
    }
}

/// A marker indicating the start and length of a section of text, in unicode characters
#[derive(Debug, Copy, Clone)]
pub struct Span(usize, usize);
impl Span {
    fn end(&self) -> usize {
        self.0 + self.1
    }
}
impl Sub for Span {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(rhs.0, self.end() - rhs.0)
    }
}
pub trait Spans {
    fn span(&self) -> Span;
}
#[derive(Debug, Clone)]
pub struct Spanning<T>(Span, T);
impl<T> Spans for Spanning<T> {
    fn span(&self) -> Span {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct Qualifier {
    name: Identifier,
    value: Option<Spanning<String>>,
}
impl Qualifier {
    fn named(name: &str) -> Self {
        Self {
            name: Identifier::named(name),
            value: None
        }
    }
    fn parse(from: &mut &str, pos: &mut usize) -> Result<Self, ParseError> {
        strip_whitespace(from, pos);
        let name = Identifier::parse(from, pos)?;
        strip_whitespace(from, pos);
        let value_pos = *pos;
        let value = if let Ok(_) = parse_symbol("|", from, pos) {
            if let Some((left, right)) = from.split_once('|') {
                *pos += left.len();
                *from = right;
                Some(Spanning(Span(value_pos, *pos - value_pos), left.trim_start().trim_end().into()))
            } else {
                return Err(ParseError::ExpectedSymbol("|".to_owned(), *pos))
            }
        } else { None };
        Ok(Qualifier {
            name,
            value
        })
    }
}
impl PartialEq<Self> for Qualifier {
    fn eq(&self, other: &Self) -> bool {
        *self.name == *other.name
    }
}
impl Eq for Qualifier {}
impl Hash for Qualifier {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}
impl Spans for Qualifier {
    fn span(&self) -> Span {
        self.name.span()
    }
}

#[derive(Debug)]
pub enum Keyword {
    Fn,
    Struct,
}

#[derive(Debug, Clone)]
pub enum DataType {
    Integer8(bool),
    Integer16(bool),
    Integer32(bool),
    Integer64(bool),
    Integer(bool),
    Pointer(Box<DataType>),
    Reference(Box<DataType>),
    DynamicArray(Box<DataType>),
    StaticArray(Box<DataType>),

    Structure(Identifier),
}
impl DataType {
    fn parse(from: &mut &str, pos: &mut usize) -> Result<Self, ParseError> {
        macro_rules! return_match {
            ($string:expr => $ret:expr) => {
                if from[..$string.len()] == *$string {
                    *from = &from[$string.len()..];
                    *pos += $string.len();
                    return Ok($ret)
                }
            };
        }
        strip_whitespace(from, pos);
        return_match!{"*" => Self::Pointer(Box::new(Self::parse(from, pos)?))}
        return_match!{"&" => Self::Reference(Box::new(Self::parse(from, pos)?))}
        return_match!{"[&]" => Self::DynamicArray(Box::new(Self::parse(from, pos)?))}
        return_match!{"[]" => Self::StaticArray(Box::new(Self::parse(from, pos)?))}
        return_match!{"struct" => {
            strip_whitespace(from, pos);
            Self::Structure(Identifier::parse(from, pos)?)
        }}

        match Identifier::parse(from, pos)?.deref() {
            "uint8" => Ok(Self::Integer8(true)),
            "int8" => Ok(Self::Integer8(false)),
            "uint16" => Ok(Self::Integer16(true)),
            "int16" => Ok(Self::Integer16(false)),
            "uint32" => Ok(Self::Integer32(true)),
            "int32" => Ok(Self::Integer32(false)),
            "uint64" => Ok(Self::Integer64(true)),
            "int64" => Ok(Self::Integer64(false)),
            "uint" => Ok(Self::Integer(true)),
            "int" => Ok(Self::Integer(false)),
            _ => Err(ParseError::UnknownType)
        }
    }
}
impl Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Integer(true) => write!(f, "uint"),
            Self::Integer(false) => write!(f, "int"),
            Self::Integer8(true) => write!(f, "uint8"),
            Self::Integer8(false) => write!(f, "int8"),
            Self::Integer16(true) => write!(f, "uint16"),
            Self::Integer16(false) => write!(f, "int16"),
            Self::Integer32(true) => write!(f, "uint32"),
            Self::Integer32(false) => write!(f, "int32"),
            Self::Integer64(true) => write!(f, "uint64"),
            Self::Integer64(false) => write!(f, "int64"),
            Self::Pointer(inner) => write!(f, "*{}", inner),
            Self::Reference(inner) => write!(f, "&{}", inner),
            Self::DynamicArray(inner) => write!(f, "[&]{}", inner),
            Self::StaticArray(inner) => write!(f, "[]{}", inner),
            Self::Structure(identifier) => write!(f, "[]{}", identifier.deref()),
        }
    }
}

pub trait Expression: std::fmt::Debug {
    fn data_type(&self) -> DataType;
}

#[derive(Debug)]
pub struct Block {
    open_brace: Span,
    expressions: Vec<Box<dyn Expression>>,
    close_brace: Span
}
impl Block {
    fn parse(from: &mut &str, pos: &mut usize) -> Result<Self, ParseError> {
        strip_whitespace(from, pos);
        let open_brace = parse_symbol("{", from, pos)?.span();
        strip_whitespace(from, pos);
        let expressions = vec![];
        strip_whitespace(from, pos);
        let close_brace = parse_symbol("}", from, pos)?.span();
        Ok(Self {
            open_brace,
            expressions,
            close_brace
        })
    }
}
impl Spans for Block {
    fn span(&self) -> Span {
        self.close_brace - self.open_brace
    }
}

#[derive(Debug)]
pub struct Function {
    fn_qualifier: Qualifier,
    name: Identifier,
    args: Vec<TypedBinding>,
    return_type: Option<Identifier>,
    block: Block
}
impl Function {
    fn parse(mut qualifiers: HashMap<String, Qualifier>, from: &mut &str, pos: &mut usize) -> Result<Self, ParseError> {
        let fn_qualifier = qualifiers.remove("fn").unwrap();
        strip_whitespace(from, pos);
        let name = Identifier::parse(from, pos)?;
        strip_whitespace(from, pos);
        parse_symbol("(", from, pos)?;
        let mut args = vec![];
        while let Ok(arg) = TypedBinding::parse(from, pos) {
            args.push(arg);
            strip_whitespace(from, pos);
            if let Err(_) = parse_symbol(",", from, pos) {
                break
            }
            strip_whitespace(from, pos);
        }
        strip_whitespace(from, pos);
        parse_symbol(")", from, pos)?;
        strip_whitespace(from, pos);
        let return_type = if from.chars().next() == Some(':') {
            parse_symbol(":", from, pos)?;
            strip_whitespace(from, pos);
            let to_ret = Identifier::parse(from, pos)?;
            strip_whitespace(from, pos);
            Some(to_ret)
        } else { None };
        let block = Block::parse(from, pos)?;
        Ok(Self {
            fn_qualifier,
            name,
            args,
            return_type,
            block
        })
    }
}
impl Spans for Function {
    fn span(&self) -> Span {
        self.block.span() - self.fn_qualifier.span()
    }
}

#[derive(Debug)]
pub struct Structure {
    struct_qualifier: Qualifier,
    name: Identifier,
    fields: Vec<TypedBinding>,
    end_brace: Span
}
impl Structure {
    fn parse(mut qualifiers: HashMap<String, Qualifier>, from: &mut &str, pos: &mut usize) -> Result<Self, ParseError> {
        let struct_qualifier =  qualifiers.remove("struct").unwrap();
        strip_whitespace(from, pos);
        let name = Identifier::parse(from, pos)?;
        strip_whitespace(from, pos);
        strip_whitespace(from, pos);
        parse_symbol("{", from, pos)?;
        let mut fields = vec![];
        while let Ok(field) = TypedBinding::parse(from, pos) {
            fields.push(field);
            strip_whitespace(from, pos);
            if let Err(_) = parse_symbol(",", from, pos) {
                break
            }
            strip_whitespace(from, pos);
        }
        strip_whitespace(from, pos);
        let end_brace = parse_symbol("}", from, pos)?.span();
        strip_whitespace(from, pos);
        Ok(Self {
            struct_qualifier,
            name,
            fields,
            end_brace
        })
    }
}
impl Spans for Structure {
    fn span(&self) -> Span {
        self.end_brace - self.struct_qualifier.span()
    }
}
impl Display for Structure {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "struct{} {} {{", if let Some(layout) = &self.struct_qualifier.value { format!("|{}|", layout.1) } else { "".to_string() }, &*self.name)?;
        for field in &self.fields {
            write!(f, "{}, ", field)?;
        }
        write!(f, "}}")
    }
}

#[derive(Debug, Clone)]
struct TypedBinding {
    name: Identifier,
    datatype: DataType
}
impl TypedBinding {
    fn parse(from: &mut &str, pos: &mut usize) -> Result<Self, ParseError> {
        strip_whitespace(from, pos);
        let name = Identifier::parse(from, pos)?;
        strip_whitespace(from, pos);
        parse_symbol(":", from, pos)?;
        strip_whitespace(from, pos);
        let datatype = DataType::parse(from, pos)?;
        strip_whitespace(from, pos);
        Ok(Self {
            name,
            datatype
        })
    }
}
impl Display for TypedBinding {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", &*self.name, self.datatype)
    }
}

#[derive(Debug)]
pub enum ParseError {
    Io(io::Error),
    ModuleName(String),
    ExpectedIdentifier(usize),
    ExpectedSymbol(String, usize),
    DuplicateQualifier(Qualifier, Qualifier),
    DuplicateFunction(String, Span, Span),
    DuplicateStructure(String, Span, Span),
    UnexpectedEOF,
    UnknownType
}
impl From<io::Error> for ParseError {
    fn from(from: io::Error) -> Self {
        Self::Io(from)
    }
}