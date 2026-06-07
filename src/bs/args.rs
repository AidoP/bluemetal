use std::borrow::Cow;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::ops::ControlFlow;
use std::process::ExitCode;
use crate::fmt;
use crate::error;
use crate::warning;

const fn str_hash(s: &str) -> usize {
    let s = s.as_bytes();
    let mut hash = 0usize;
    let mut i = 0;
    while i < s.len() {
        hash = hash.wrapping_add(s[i] as usize);
        hash = hash.wrapping_add(hash << 10);
        hash ^= hash >> 6;
        i += 1;
    }

    hash = hash.wrapping_add(hash << 3);
    hash ^= hash >> 11;
    hash = hash.wrapping_add(hash << 15);
    hash
}

const fn str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[derive(Debug)]
struct Opt {
    name: &'static str,
    positional: bool,
    /// Only applies when processing this subcommand.
    subcommand: Option<&'static str>,
    help: &'static str,
    required: bool,
    value: bool,
    /// For positional arguments, this captures all following positional arguments.
    repeat: bool,
    f: fn(&mut Config, Option<Cow<OsStr>>) -> ControlFlow<ExitCode>,
    children: [usize; Self::TRIE_SIZE],
}
impl Opt {
    pub const fn new(
        name: &'static str,
        help: &'static str,
        f: fn(&mut Config, value: Option<Cow<OsStr>>) -> ControlFlow<ExitCode>,
    ) -> Self {
        Self {
            name,
            positional: false,
            subcommand: None,
            help,
            required: false,
            value: false,
            repeat: false,
            f,
            children: [0; _],
        }
    }
    pub const fn positional(mut self) -> Self {
        self.positional = true;
        self
    }
    pub const fn value(mut self) -> Self {
        self.value = true;
        self
    }
    pub fn process(&self, config: &mut Config, value: Option<Cow<OsStr>>) -> ControlFlow<ExitCode> {
        (self.f)(config, value)
    }
}

impl Opt {
    const TRIE_SIZE: usize = 4;
    const TRIE_FACTOR: usize = Self::TRIE_SIZE.ilog2() as usize;

    /// Returns the index of the item with the given name in the graph.
    ///
    /// If `Err`, the index of the last node found, and the index of the child node
    /// it would have continued searching through.
    const fn lookup(trie: &[Self], key: &str) -> Result<usize, (usize, usize)> {
        let mut index = 0;
        let mut hash = str_hash(key);
        loop {
            let node = &trie[index];
            if str_eq(node.name, key) {
                return Ok(index);
            }
            let q = hash & (Self::TRIE_SIZE - 1);
            let next_index = node.children[q];
            if next_index == 0 {
                return Err((index, q));
            }
            index = next_index;

            hash = hash >> Self::TRIE_FACTOR;
        }
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct Opts<T: AsRef<[Opt]> + ?Sized = [Opt]>(T);
impl<const N: usize> Opts<[Opt; N]> {
    /// Convert an array into a hash trie.
    ///
    /// # Panics
    ///
    /// Panics if there are elements with duplicated keys in the array.
    pub const fn new(mut nodes: [Opt; N]) -> Self {
        // skip inserting the root node
        let mut i = 1;
        while i < nodes.len() {
            let node = &nodes[i];
            match Opt::lookup(&nodes, node.name) {
                Ok(_) => ::core::panic!("duplicate item"),
                Err((end, q)) => {
                    nodes[end].children[q] = i;
                }
            }
            i += 1;
        }
        Self(nodes)
    }
    pub const fn find<'a>(&'a self, key: &str) -> Option<&'a Opt> {
        let trie = &self.0;
        match Opt::lookup(trie, key) {
            Ok(i) => Some(&trie[i]),
            Err(_) => None,
        }
    }
}
impl Opts<[Opt]> {
    pub const fn get<'a>(&'a self, p: usize) -> Option<&'a Opt> {
        if p > self.0.len() {
            None
        } else {
            Some(&self.0[p])
        }
    }
    pub const fn position(&self, key: &str) -> Option<usize> {
        let trie = &self.0;
        match Opt::lookup(trie, key) {
            Ok(i) => Some(i),
            Err(_) => None,
        }
    }
    pub const fn find<'a>(&'a self, key: &str) -> Option<&'a Opt> {
        let trie = &self.0;
        match Opt::lookup(trie, key) {
            Ok(i) => Some(&trie[i]),
            Err(_) => None,
        }
    }
    pub const fn as_slice(&self) -> &[Opt] {
        &self.0
    }
}

impl Opts {
    pub fn parse(mut args: std::env::ArgsOs, config: &mut Config) -> ControlFlow<ExitCode> {
        let error = |e| {
            error!("{}", e);
            ControlFlow::Break(ExitCode::FAILURE)
        };

        // For positional arguments, the position in OPTIONS.
        let mut opts_index = 0;

        'args: while let Some(arg) = args.next() {
            let arg_bytes = arg.as_encoded_bytes();
            if let Some(b'-') = arg_bytes.get(0) {
                let mut key = &arg_bytes[1..];
                let mut value = None;
                let mut p = 1;
                while p < arg_bytes.len() {
                    if arg_bytes[p] == b'=' {
                        key = &arg_bytes[1..p];
                        value = Some(&arg_bytes[p+1..]);
                        break;
                    }
                    p += 1;
                }
                // Safety: the string has only been split on ASCII boundaries
                let key = unsafe { OsStr::from_encoded_bytes_unchecked(key) };
                let value = match value {
                    None => None,
                    Some(value) => Some(unsafe { OsStr::from_encoded_bytes_unchecked(value) }),
                };
                let Some(key) = key.to_str() else {
                    // never a valid flag if not UTF-8
                    let mut arg = OsString::from("-");
                    arg.push(key);
                    return error(ArgError::InvalidFlag(arg));
                };
                if key.starts_with('-') {
                    let Some(opt) = OPTIONS.find(&key[1..]) else {
                        return error(ArgError::InvalidFlag(arg));
                    };
                    if opt.positional {
                        return error(ArgError::InvalidFlag(arg));
                    }
                    let value = match value {
                        Some(value) if opt.value => Some(Cow::Borrowed(value)),
                        Some(value) => {
                            return error(ArgError::UnexpectedValue(opt.name, None, value.into()));
                        },
                        None if opt.value => {
                            let Some(value) = args.next() else {
                                return error(ArgError::MissingValue(opt.name, None));
                            };
                            Some(Cow::Owned(value))
                        },
                        None => None,
                    };
                    opt.process(config, value)?;
                } else {
                    'short_flags: for c in key.chars() {
                        for &(alias, p) in ALIASES {
                            if alias == c {
                                let opt = OPTIONS.get(p).unwrap();
                                let value = match value {
                                    Some(value) if opt.value => Some(Cow::Borrowed(value)),
                                    Some(value) => {
                                        return error(ArgError::UnexpectedValue(opt.name, Some(c), value.into()));
                                    },
                                    None if opt.value => {
                                        let Some(value) = args.next() else {
                                            return error(ArgError::MissingValue(opt.name, Some(c)));
                                        };
                                        Some(Cow::Owned(value))
                                    },
                                    None => None,
                                };
                                opt.process(config, value)?;
                                continue 'short_flags;
                            }
                        }
                        let mut arg = String::from("-");
                        arg.push(c);
                        return error(ArgError::InvalidFlag(arg.into()));
                    }
                }
                continue 'args;
            } else {
                while opts_index < OPTIONS.as_slice().len() {
                    let opt = &OPTIONS.as_slice()[opts_index];
                    opts_index += 1;
                    if !opt.positional {
                        continue;
                    }
                    opt.process(config, Some(Cow::Owned(arg)))?;
                    continue 'args;
                }
                return error(ArgError::Unexpected(arg));
            }
        }
        ControlFlow::Continue(())
    }
}

const fn index_aliases<const N: usize>(options: &Opts, values: [(char, &str); N]) -> [(char, usize); N] {
    let mut index = [('\0', 0); N];
    let mut i = 0;
    while i < values.len() {
        let (alias, key) = values[i];
        let p = options.position(key).unwrap();
        index[i] = (alias, p);
        i += 1;
    }
    index
}

enum ArgError {
    InvalidFlag(OsString),
    UnexpectedValue(&'static str, Option<char>, OsString),
    MissingValue(&'static str, Option<char>),
    Unexpected(OsString),
}
impl core::fmt::Display for ArgError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        let hl = fmt::bright_cyan();
        let reset = fmt::reset();
        match self {
            Self::InvalidFlag(arg) => write!(f, "invalid flag {hl}{arg:?}{reset}"),
            Self::UnexpectedValue(key, None, value) => write!(f, "unexpected value for {hl}--{key}={value:?}{reset}"),
            Self::MissingValue(key, None) => write!(f, "argument {hl}--{key}{reset} requires a value"),
            Self::UnexpectedValue(key, Some(alias), value) => write!(f, "unexpected value for {hl}--{key}={value:?}{reset} ({hl}-{alias}{reset})"),
            Self::MissingValue(key, Some(alias)) => write!(f, "argument {hl}--{key}{reset} ({hl}-{alias}{reset}) requires a value"),
            Self::Unexpected(arg) => write!(f, "unexpected argument {hl}{arg:?}{reset}"),
        }
    }
}
