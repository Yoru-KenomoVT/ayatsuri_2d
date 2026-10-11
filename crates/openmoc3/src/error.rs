//! Every way a `.moc3` file can be wrong.
//!
//! All of these are recoverable and carry enough detail to say what failed and
//! where. None of them panics. That is the entire point of this crate:
//! Cubism Core performs none of these checks, which is CVE-2023-27566, and the
//! consequence is that any downloaded model is an arbitrary memory write.

use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Fewer than 64 bytes, or a section that runs past the end.
    Truncated { need: usize, have: usize },
    /// First four bytes are not `MOC3`.
    BadMagic([u8; 4]),
    /// Version byte outside 1..=6.
    UnknownVersion(u8),
    /// Big-endian file. Byte 5 is set. Legal, never seen in the wild.
    UnsupportedEndian,
    /// A section offset points outside the file.
    OffsetOutOfBounds { slot: usize, offset: u32 },
    /// The offset table length derived from slot 0 does not match what this
    /// version measures (480 entries in v3 to v5, 1440 in v6).
    BadSotLength { entries: usize, expected: usize },
    /// `count * element_size` exceeds the space the section actually has.
    SectionTooSmall { slot: usize, need: usize, have: usize },
    /// An index field points past the end of the table it indexes.
    IndexOutOfRange { slot: usize, at: usize, value: i32, limit: i32 },
    /// A 64-byte ID record with no NUL terminator, or non-ASCII bytes.
    BadId { slot: usize, at: usize },
    /// A count is negative, or large enough that `count * size` would overflow.
    ImplausibleCount { index: usize, value: i32 },
    /// `deformers != warp_deformers + rotation_deformers`, the one invariant
    /// that holds in every model seen across all three versions.
    CountsInconsistent { deformers: i32, warp: i32, rotation: i32 },
    /// A begin/count pair that reaches outside its pool.
    BadSpan { begin: i32, len: i32, pool: usize },
    /// A slot number this crate has no description for.
    NoSuchSection { slot: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Error::Truncated { need, have } => {
                write!(f, "file truncated: need {need} bytes, have {have}")
            }
            Error::BadMagic(m) => {
                write!(
                    f,
                    "bad magic: expected \"MOC3\", found {:02x} {:02x} {:02x} {:02x}",
                    m[0], m[1], m[2], m[3]
                )
            }
            Error::UnknownVersion(b) => {
                write!(f, "unknown version byte {b}: 1 through 6 are defined")
            }
            Error::UnsupportedEndian => {
                write!(f, "big-endian file (header byte 5 set): legal but unsupported")
            }
            Error::OffsetOutOfBounds { slot, offset } => {
                write!(f, "slot {slot}: offset {offset} points outside the file")
            }
            Error::BadSotLength { entries, expected } => {
                write!(
                    f,
                    "offset table has {entries} entries, expected {expected} for this version"
                )
            }
            Error::SectionTooSmall { slot, need, have } => {
                write!(f, "slot {slot} declares {need} bytes of entries but has room for {have}")
            }
            Error::IndexOutOfRange { slot, at, value, limit } => {
                write!(f, "slot {slot}, entry {at}: index {value} is outside -1..{limit}")
            }
            Error::BadId { slot, at } => {
                write!(
                    f,
                    "slot {slot}, record {at}: ID is not NUL-terminated ASCII within 64 bytes"
                )
            }
            Error::ImplausibleCount { index, value } => {
                write!(f, "count {index} is {value}: negative or implausibly large")
            }
            Error::CountsInconsistent { deformers, warp, rotation } => {
                write!(
                    f,
                    "deformers is {deformers} but warp {warp} + rotation {rotation} = {}",
                    warp.saturating_add(rotation)
                )
            }
            Error::BadSpan { begin, len, pool } => {
                write!(f, "span begins at {begin} for {len} entries but the pool holds {pool}")
            }
            Error::NoSuchSection { slot } => {
                write!(f, "slot {slot} has no known description")
            }
        }
    }
}

impl core::error::Error for Error {}
