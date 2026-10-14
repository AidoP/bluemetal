//! The flycheck database, which records the diagnostics to replay for each crate's flycheck label.
//!
//! It is written when configuring, so that a flycheck does not evaluate the configuration again, which may be impure.
//!
//! The format is the magic bytes, a format version, then the number of entries followed by each entry. An entry is a
//! label followed by the number of paths and each path. Numbers are little-endian `u32`, and strings are their length
//! followed by their UTF-8 bytes.

use std::io::Write;

pub const PATH: &str = "target/flycheck.db";
const MAGIC: &[u8; 4] = b"BMFC";
const VERSION: u32 = 1;

/// The diagnostics for a label, borrowed from the bytes of the database.
#[derive(Debug, PartialEq)]
pub struct Entry<'a> {
    pub label: &'a str,
    /// The files of JSON diagnostics to replay, in order.
    pub diagnostics: Vec<&'a str>,
}

pub fn write(w: &mut dyn Write, entries: &[Entry]) -> std::io::Result<()> {
    w.write_all(MAGIC)?;
    write_u32(w, VERSION)?;
    write_u32(w, entries.len())?;
    for entry in entries {
        write_str(w, entry.label)?;
        write_u32(w, entry.diagnostics.len())?;
        for path in &entry.diagnostics {
            write_str(w, path)?;
        }
    }
    Ok(())
}

/// Parse a database, failing with [`std::io::ErrorKind::InvalidData`] if it is not a database of this version.
pub fn read(bytes: &[u8]) -> std::io::Result<Vec<Entry<'_>>> {
    let mut r = Reader(bytes);
    if r.take(MAGIC.len())? != MAGIC || r.u32()? != VERSION {
        return Err(invalid());
    }
    let count = r.u32()?;
    let mut entries = Vec::new();
    for _ in 0..count {
        let label = r.str()?;
        let len = r.u32()?;
        let mut diagnostics = Vec::new();
        for _ in 0..len {
            diagnostics.push(r.str()?);
        }
        entries.push(Entry { label, diagnostics });
    }
    Ok(entries)
}

fn invalid() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, "not a flycheck database of this version")
}

fn write_u32(w: &mut dyn Write, value: impl TryInto<u32>) -> std::io::Result<()> {
    let value: u32 = value.try_into().map_err(|_| std::io::Error::other("value too large for the database"))?;
    w.write_all(&value.to_le_bytes())
}

fn write_str(w: &mut dyn Write, s: &str) -> std::io::Result<()> {
    write_u32(w, s.len())?;
    w.write_all(s.as_bytes())
}

/// The remaining bytes of a database being parsed.
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> std::io::Result<&'a [u8]> {
        let Some((taken, rest)) = self.0.split_at_checked(len) else {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        };
        self.0 = rest;
        Ok(taken)
    }
    fn u32(&mut self) -> std::io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn str(&mut self) -> std::io::Result<&'a str> {
        let len = self.u32()? as usize;
        std::str::from_utf8(self.take(len)?).map_err(|_| invalid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let entries = vec![
            Entry { label: "app@x86_64", diagnostics: vec!["a.json", "b é.json"] },
            Entry { label: "empty", diagnostics: vec![] },
        ];
        let mut bytes = Vec::new();
        write(&mut bytes, &entries).unwrap();
        assert_eq!(read(&bytes).unwrap(), entries);
    }

    #[test]
    fn rejects_other_files() {
        let mut bytes = Vec::new();
        write(&mut bytes, &[]).unwrap();
        bytes[4] += 1;
        assert_eq!(read(&bytes).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(read(b"{\"crates\"").unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        // truncated
        let mut bytes = Vec::new();
        write(&mut bytes, &[Entry { label: "x", diagnostics: vec![] }]).unwrap();
        assert_eq!(read(&bytes[..bytes.len() - 1]).unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
