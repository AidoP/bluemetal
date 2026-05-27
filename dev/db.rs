use std::{collections::HashMap, io::BufReader, num::NonZeroUsize, path::Path};

use crate::Config;

/// # Compilation Database Error
pub enum Error {
    Open(std::io::Error),
    Read(std::io::Error),
    InvalidHeader,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open(error) => write!(f, "while opening, {error}"),
            Self::Read(error) => write!(f, "while reading, {error}"),
            Self::InvalidHeader => write!(f, "invalid header"),
        }
    }
}

/// # Compilation Database
///
/// Binary file format that records the state of compilation.
pub struct Db {
    targets: HashMap<&'static str, ()>,
}
impl Db {
    pub fn new(config: &Config) -> Result<Self, Error> {
        let mut db = Self {
            targets: HashMap::new(),
        };
        let path = config.source_root.join("out/build.db");
        match std::fs::File::open(&path) {
            Ok(file) => db.load(&mut BufReader::new(file))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(Error::Open(error)),
        }
        Ok(db)
    }
    pub fn load(&mut self, stream: &mut dyn std::io::Read) -> Result<(), Error> {
        Ok(())
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Header {
    magic: u64,
    reserved: [u32; 5],
}
impl Header {
    pub const MAGIC: u64 = u64::from_ne_bytes(*b"BMCOMPDB");
    pub fn new() -> Self {
        Self {
            magic: Self::MAGIC,
            reserved: [0u32; _],
        }
    }
    pub fn read<T: std::io::Read>(mut f: T) -> Result<Self, Error> {
        let mut buffer = [0; size_of::<Self>()];
        match f.read_exact(&mut buffer) {
            Err(error) => Err(Error::Read(error)),
            Ok(_) => {
                let header = unsafe { *buffer.as_mut_ptr().cast::<Self>() };
                if header.magic != Self::MAGIC {
                    return Err(Error::InvalidHeader);
                }
                Ok(header)
            },
        }
    }
}

