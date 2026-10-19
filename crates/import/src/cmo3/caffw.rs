// Writing the CAFF container, the mirror of caff.rs: the fixed header with
// a key, the table of entries with its integers XOR'd by the key and its
// bytes by the key's low byte, then every entry's bytes XOR'd the same
// way. `main.xml` goes in as a one-entry streamed zip, deflated, with the
// sizes in a trailing data descriptor as the Editor writes it.

use crate::ImportError;

/// One file to store: its name, its bytes and whether it is the project
/// XML (which gets the `main_xml` kind and the zip compression code).
pub struct File {
    pub name: String,
    pub bytes: Vec<u8>,
    pub main: bool,
}

const HEADER_LEN: usize = 0x36;
/// Two bytes every project ends with.
const TRAILER: [u8; 2] = [0x62, 0x63];

struct Out {
    b: Vec<u8>,
    key: u32,
    kb: u8,
}

impl Out {
    fn u32(&mut self, v: u32) {
        self.b.extend_from_slice(&(v ^ self.key).to_be_bytes());
    }

    fn u8(&mut self, v: u8) {
        self.b.push(v ^ self.kb);
    }

    fn text(&mut self, s: &str) -> Result<(), ImportError> {
        let bytes = s.as_bytes();
        let n = u8::try_from(bytes.len())
            .map_err(|_| ImportError::Export(format!("cmo3: entry name too long: {s}")))?;
        self.u8(n);
        for &c in bytes {
            self.u8(c);
        }
        Ok(())
    }
}

/// The archive, entries in the order given.
pub fn write(files: &[File], key: u32) -> Result<Vec<u8>, ImportError> {
    let kb = (key & 0xff) as u8;
    // the table's size depends only on the names, so lay it out first
    let table_len: usize = 4 + files
        .iter()
        .map(|f| {
            let kind = if f.main { "main_xml".len() } else { 0 };
            1 + f.name.len() + 1 + kind + 4 + 4 + 4 + 1 + 1 + 8
        })
        .sum::<usize>();
    let total: usize = files.iter().map(|f| f.bytes.len()).sum();
    let mut o = Out { b: Vec::with_capacity(HEADER_LEN + table_len + total + 2), key, kb };
    o.b.extend_from_slice(b"CAFF");
    o.b.extend_from_slice(&[0, 0, 0, 0x2d, 0x2d, 0x2d, 0x2d, 0, 0, 0]);
    o.b.extend_from_slice(&key.to_be_bytes());
    o.b.extend_from_slice(&[0; 8]);
    o.b.extend_from_slice(&[0x7f, 0x7f]);
    o.b.resize(HEADER_LEN, 0);
    let count = u32::try_from(files.len())
        .map_err(|_| ImportError::Export("cmo3: too many entries".into()))?;
    o.u32(count);
    let mut start = HEADER_LEN + table_len;
    for f in files {
        o.text(&f.name)?;
        o.text(if f.main { "main_xml" } else { "" })?;
        o.u32(0);
        let (s, n) = (u32::try_from(start), u32::try_from(f.bytes.len()));
        let (Ok(s), Ok(n)) = (s, n) else {
            return Err(ImportError::Export("cmo3: archive over 4 GB".into()));
        };
        o.u32(s);
        o.u32(n);
        o.u8(1);
        o.u8(if f.main { 33 } else { 16 });
        o.b.extend_from_slice(&[0; 8]);
        start += f.bytes.len();
    }
    if o.b.len() != HEADER_LEN + table_len {
        return Err(ImportError::Export("cmo3: table size mismatch".into()));
    }
    for f in files {
        o.b.extend(f.bytes.iter().map(|c| c ^ kb));
    }
    o.b.extend_from_slice(&TRAILER);
    Ok(o.b)
}

/// A one-entry streamed zip named `contents`: local header with the
/// sizes left for a data descriptor, the deflate stream, the descriptor.
pub fn zip_one(data: &[u8]) -> Result<Vec<u8>, ImportError> {
    let deflated = miniz_oxide::deflate::compress_to_vec(data, 6);
    let (csize, usize_) = (u32::try_from(deflated.len()), u32::try_from(data.len()));
    let (Ok(csize), Ok(usize_)) = (csize, usize_) else {
        return Err(ImportError::Export("cmo3: main.xml over 4 GB".into()));
    };
    let crc = crc32(data);
    let mut z = Vec::with_capacity(deflated.len() + 64);
    z.extend_from_slice(b"PK\x03\x04");
    z.extend_from_slice(&0x0014u16.to_le_bytes());
    z.extend_from_slice(&0x0808u16.to_le_bytes());
    z.extend_from_slice(&8u16.to_le_bytes());
    // modification time and date: noon, 2026-01-01
    z.extend_from_slice(&0x6000u16.to_le_bytes());
    z.extend_from_slice(&0x5c21u16.to_le_bytes());
    z.extend_from_slice(&[0; 12]);
    z.extend_from_slice(&8u16.to_le_bytes());
    z.extend_from_slice(&0u16.to_le_bytes());
    z.extend_from_slice(b"contents");
    z.extend_from_slice(&deflated);
    z.extend_from_slice(b"PK\x07\x08");
    z.extend_from_slice(&crc.to_le_bytes());
    z.extend_from_slice(&csize.to_le_bytes());
    z.extend_from_slice(&usize_.to_le_bytes());
    Ok(z)
}

/// Plain CRC-32 (IEEE), as zip wants it.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn what_we_write_our_reader_opens() {
        let xml = b"<root/>".repeat(100);
        let files = vec![
            File { name: "imageFileBuf.png".into(), bytes: vec![1, 2, 3], main: false },
            File { name: "main.xml".into(), bytes: zip_one(&xml).unwrap(), main: true },
        ];
        let bytes = write(&files, 0x19cb_35f1).unwrap();
        let caff = super::super::caff::Caff::open(&bytes).unwrap();
        assert_eq!(caff.entries.len(), 2);
        assert_eq!(caff.read("imageFileBuf.png").unwrap(), vec![1, 2, 3]);
        assert_eq!(caff.main_xml().unwrap(), xml);
    }
}
