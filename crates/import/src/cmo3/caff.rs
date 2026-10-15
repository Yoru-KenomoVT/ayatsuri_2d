// The CAFF container a Cubism Editor project (.cmo3, .can3) is stored in,
// as worked out from the files: a fixed header with a 32-bit key, then a
// file table whose integers are XOR'd with the key and whose bytes with
// its low byte, then the entries themselves. Entries marked obfuscated are
// XOR'd with the low byte too. `main.xml` is a one-entry streamed zip.

use crate::ImportError;

fn err(msg: impl Into<String>) -> ImportError {
    ImportError::Export(format!("cmo3: {}", msg.into()))
}

/// One entry of the table.
pub struct Entry {
    pub name: String,
    pub start: usize,
    pub size: usize,
    pub obfuscated: bool,
}

pub struct Caff<'a> {
    bytes: &'a [u8],
    key_byte: u8,
    pub entries: Vec<Entry>,
}

struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
    key: u32,
    kb: u8,
}

impl Cursor<'_> {
    fn u8(&mut self) -> Result<u8, ImportError> {
        let v = *self.b.get(self.pos).ok_or_else(|| err("table ends early"))?;
        self.pos += 1;
        Ok(v ^ self.kb)
    }

    fn u32(&mut self) -> Result<u32, ImportError> {
        let s = self.b.get(self.pos..self.pos + 4).ok_or_else(|| err("table ends early"))?;
        let arr: [u8; 4] = s.try_into().map_err(|_| err("table"))?;
        self.pos += 4;
        Ok(u32::from_be_bytes(arr) ^ self.key)
    }

    fn text(&mut self) -> Result<String, ImportError> {
        let n = self.u8()? as usize;
        let s = self.b.get(self.pos..self.pos + n).ok_or_else(|| err("table ends early"))?;
        self.pos += n;
        let raw: Vec<u8> = s.iter().map(|c| c ^ self.kb).collect();
        Ok(String::from_utf8_lossy(&raw).into_owned())
    }
}

impl<'a> Caff<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self, ImportError> {
        if bytes.get(..4) != Some(b"CAFF") {
            return Err(err("not a Cubism project archive"));
        }
        let k = bytes.get(0x0e..0x12).ok_or_else(|| err("short header"))?;
        let arr: [u8; 4] = k.try_into().map_err(|_| err("short header"))?;
        let key = u32::from_be_bytes(arr);
        let kb = (key & 0xff) as u8;
        let mut c = Cursor { b: bytes, pos: 0x36, key, kb };
        let count = c.u32()? as usize;
        if count > 1 << 20 {
            return Err(err("table too long"));
        }
        let mut entries = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let name = c.text()?;
            let _kind = c.text()?;
            let tags = c.u32()? as usize;
            if tags > 1024 {
                return Err(err("tag list too long"));
            }
            for _ in 0..tags {
                c.text()?;
            }
            let start = c.u32()? as usize;
            let size = c.u32()? as usize;
            let obfuscated = c.u8()? != 0;
            let _compressed = c.u8()?;
            c.pos += 8;
            if start.checked_add(size).is_none_or(|e| e > bytes.len()) {
                return Err(err(format!("{name}: outside the file")));
            }
            entries.push(Entry { name, start, size, obfuscated });
        }
        Ok(Caff { bytes, key_byte: kb, entries })
    }

    /// An entry's bytes, de-obfuscated.
    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        let e = self.entries.iter().find(|e| e.name == name)?;
        let raw = self.bytes.get(e.start..e.start + e.size)?;
        Some(if e.obfuscated {
            raw.iter().map(|c| c ^ self.key_byte).collect()
        } else {
            raw.to_vec()
        })
    }

    /// `main.xml`, unpacked from its zip.
    pub fn main_xml(&self) -> Result<Vec<u8>, ImportError> {
        let zip = self.read("main.xml").ok_or_else(|| err("no main.xml"))?;
        unzip_one(&zip)
    }
}

/// The single entry of a streamed zip (local header, no central directory
/// needed): stored or deflated.
pub fn unzip_one(blob: &[u8]) -> Result<Vec<u8>, ImportError> {
    if blob.get(..4) != Some(b"PK\x03\x04") {
        return Err(err("main.xml is not a zip"));
    }
    let le16 = |at: usize| -> Option<u16> {
        Some(u16::from_le_bytes([*blob.get(at)?, *blob.get(at + 1)?]))
    };
    let method = le16(8).ok_or_else(|| err("zip header"))?;
    let name_len = le16(26).ok_or_else(|| err("zip header"))? as usize;
    let extra_len = le16(28).ok_or_else(|| err("zip header"))? as usize;
    let data = blob.get(30 + name_len + extra_len..).ok_or_else(|| err("zip header"))?;
    // the project XML is a few megabytes; anything near this is not one
    const LIMIT: usize = 1 << 30;
    match method {
        0 => Ok(data.to_vec()),
        8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, LIMIT)
            .map_err(|_| err("main.xml will not inflate")),
        m => Err(err(format!("zip method {m}"))),
    }
}
