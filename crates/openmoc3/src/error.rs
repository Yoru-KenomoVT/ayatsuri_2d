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
    /// `count * element_size` exceeds the space the section actually has.
    SectionTooSmall { slot: usize, need: usize, have: usize },
    /// An index field points past the end of the table it indexes.
    IndexOutOfRange { slot: usize, at: usize, value: i32, limit: i32 },
    /// A 64-byte ID record with no NUL terminator, or invalid UTF-8.
    BadId { slot: usize, at: usize },
    /// A count is negative, or large enough that `count * size` would overflow.
    ImplausibleCount { index: usize, value: i32 },
}

impl fmt::Display for Error {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO: one clear sentence per variant. Say what was expected, what was
        // found, and where. "slot 47 declares 900 entries but has room for 12"
        // beats "invalid section".
        todo!()
    }
}
