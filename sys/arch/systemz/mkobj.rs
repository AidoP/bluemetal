use std::{
    process::{
        ExitCode,
        Command,
    },
};

fn main() -> ExitCode {
    let mut args = std::env::args();
    let program = args.next();
    let program = program.as_deref().unwrap_or("mkobj");
    let Some(section_name) = args.next() else {
        eprintln!("invalid usage: missing section symbol name");
        help(program, &mut std::io::stderr());
        return ExitCode::FAILURE;
    };

    let section_name = match section_name.as_str() {
        "-" => None,
        name => Some(name),
    };

    let Some(input_path) = args.next() else {
        eprintln!("invalid usage: missing input file path");
        help(program, &mut std::io::stderr());
        return ExitCode::FAILURE;
    };

    let Some(output_path) = args.next() else {
        eprintln!("invalid usage: missing output file path");
        help(program, &mut std::io::stderr());
        return ExitCode::FAILURE;
    };

    let mut command = Command::new("llvm-objcopy");
    command.args([
            "-S",
            "-O",
            "binary",
            &input_path,
            "-",
        ]);
    let input = match command.output() {
        Ok(output) => {
            output.stdout
        },
        Err(error) => {
            eprintln!("failed to run `llvm-objcopy` with {input_path:?}: {error}");
            return ExitCode::FAILURE;
        }
    };

    let stream = match std::fs::File::create(&output_path) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("failed to create {output_path:?}: {error}");
            return ExitCode::FAILURE;
        },
    };
    let mut stream = std::io::BufWriter::new(stream);

    let section_id = 1;
    if let Some(section_name) = section_name {
        let symbol = match Symbol::new(&section_name, SymbolType::Section, 0, input.len().try_into().expect("input file length under 16MB")) {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("invalid section symbol {section_name:?}: {error}");
                return ExitCode::FAILURE;
            },
        };
        if let Err(error) = write_esd(&mut stream, section_id, &[symbol]) {
            eprintln!("failed to write to {output_path:?}: {error}");
            return ExitCode::FAILURE;
        }
    }
    let mut offset = 0;
    for txt in input.chunks(MAX_TEXT_CHUNK_LEN) {
        if let Err(error) = write_txt(&mut stream, offset, section_id, txt) {
            eprintln!("failed to write to {output_path:?}: {error}");
            return ExitCode::FAILURE;
        }
        offset += txt.len() as u32;
    }

    if let Err(error) = write_end(&mut stream, 0, section_id) {
        eprintln!("failed to write to {output_path:?}: {error}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn help(name: &str, out: &mut dyn std::io::Write) {
    write!(out, "\
Usage: {name} <section> <input> <output>

Runs objcopy to create a static binary then wraps it inside an MVS 3.8
compatible object file.

Arguments:

    section     Section symbol to create. If `-` only TXT records are written.

    input       Path of the input object or executable to be processed by
                objcopy.

    output      Path to write an MVS 3.8 compatible object file.
").unwrap();
}

const BLANK: u8 = 0x40;
const PADDING: [u8; 80] = [BLANK; 80];

#[derive(Debug)]
pub enum SymbolError {
    /// Symbol longer than 8 bytes.
    NameTooLong,
    /// Symbol contains illegal characters.
    NameInvalid,
    /// Out of bounds address.
    AddressInvalid,
    /// Out of bounds length.
    LengthInvalid,
}
impl core::fmt::Display for SymbolError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::NameTooLong => write!(f, "symbol name is too long"),
            Self::NameInvalid => write!(f, "symbol name contains invalid characters"),
            Self::AddressInvalid => write!(f, "symbol address overflows 24 bits"),
            Self::LengthInvalid => write!(f, "symbol length overflows 24 bits"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SymbolType {
    Section = 0x00,
    Label = 0x01,
    External = 0x02,
    Private = 0x04,

    Blank = 0x40,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Symbol {
    pub name: [u8; 8],
    pub ty: SymbolType,
    /// Big-endian 24-bit address.
    pub addr: [u8; 3],
    /// "Blank" (0x40)
    pub _reserved: u8,
    /// Big-endian length or section ID.
    pub info: [u8; 3],
}
impl Symbol {
    pub const BLANK: Self = Self {
        name: [BLANK; 8],
        ty: SymbolType::Blank,
        addr: [BLANK; 3],
        _reserved: BLANK,
        info: [BLANK; 3],
    };
    pub const fn is_blank(&self) -> bool {
        match self.ty {
            SymbolType::Blank => true,
            _ => false,
        }
    }
    pub const fn is_label(&self) -> bool {
        match self.ty {
            SymbolType::Label => true,
            _ => false,
        }
    }
    pub fn new(name: &str, ty: SymbolType, mut addr: u32, mut info: u32) -> Result<Symbol, SymbolError> {
        if name.len() > 8 {
            return Err(SymbolError::NameTooLong);
        }
        if  ty == SymbolType::External {
            if info != 0 {
                return Err(SymbolError::LengthInvalid);
            }
            if addr != 0 {
                return Err(SymbolError::AddressInvalid);
            }
            addr = 0x404040;
            info = 0x404040;
        }
        let addr = addr.to_be_bytes();
        if addr[0] != 0 {
            return Err(SymbolError::AddressInvalid);
        }
        let info = info.to_be_bytes();
        if info[0] != 0 {
            return Err(SymbolError::LengthInvalid);
        }
        let mut symbol = Symbol {
            name: [BLANK; 8],
            ty,
            addr: [addr[1], addr[2], addr[3]],
            info: [info[1], info[2], info[3]],
            _reserved: BLANK,
        };
        for (i, c) in name.bytes().enumerate() {
            let c = CP_TO_EBCDIC[c as usize];
            if c == 0 {
                return Err(SymbolError::NameInvalid);
            }
            symbol.name[i] = c;
        }
        Ok(symbol)
    }
}

fn write_esd(writer: &mut dyn std::io::Write, first_id: u16, symbols: &[Symbol]) -> std::io::Result<()> {
    let esd_id = first_id.to_be_bytes();
    let symbols = unsafe { core::slice::from_raw_parts(symbols.as_ptr().cast::<u8>(), size_of_val(&symbols)) };
    let size = (symbols.len() as u16).to_be_bytes();
    let buffer = [
        0x02,
        0xC5, 0xE2, 0xC4, // ESD
        BLANK, BLANK, BLANK, BLANK, BLANK, BLANK,
        size[0], size[1],
        BLANK, BLANK,
        esd_id[0], esd_id[1],
    ];
    writer.write_all(&buffer)?;
    writer.write_all(symbols)?;
    writer.write_all(&PADDING[..PADDING.len() - buffer.len() - symbols.len()])?;
    Ok(())
}

const MAX_TEXT_CHUNK_LEN: usize = 72 - 16;
fn write_txt(writer: &mut dyn std::io::Write, offset: u32, section: u16, data: &[u8]) -> std::io::Result<()> {
    assert!(data.len() <= MAX_TEXT_CHUNK_LEN);
    let len = u16::try_from(data.len()).unwrap().to_be_bytes();
    let section = section.to_be_bytes();
    let addr = offset.to_be_bytes();
    if addr[0] != 0 {
        panic!("overflowed maximum object size");
    }
    let buffer = [
        0x02,
        0xE3, 0xE7, 0xE3, // TXT
        BLANK,
        addr[1], addr[2], addr[3],
        BLANK, BLANK,
        len[0], len[1],
        BLANK, BLANK,
        section[0], section[1],
    ];
    writer.write_all(&buffer)?;
    writer.write_all(&data)?;
    writer.write_all(&PADDING[..80 - buffer.len() - data.len()])?;
    Ok(())
}

fn write_end(writer: &mut dyn std::io::Write, offset: u32, section: u16) -> std::io::Result<()> {
    let addr = offset.to_be_bytes();
    if addr[0] != 0 {
        panic!("overflowed maximum object size");
    }
    let section = section.to_be_bytes();
    let buffer = [
        0x02,
        0xC5, 0xD5, 0xC4, // END
        BLANK,
        addr[1], addr[2], addr[3],
        BLANK, BLANK, BLANK, BLANK, BLANK, BLANK,
        section[0], section[1],
    ];
    writer.write_all(&buffer)?;
    writer.write_all(&PADDING[..PADDING.len() - buffer.len()])?;
    Ok(())
}

/// Converts a codepoint to "safe" EBCDIC.
/// Invalid characters are `0`.
///
/// Valid: `A-Z`, `0-9`, `$#@`
const CP_TO_EBCDIC: [u8; 256] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x40, 0x00, 0x00, 0x7B, 0x5B, 0x6C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x7C, 0xC1, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xD1, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6,
    0xD7, 0xD8, 0xD9, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
