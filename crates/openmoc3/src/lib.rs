//! A safe reader for Live2D `.moc3` model files.
//!
//! # Why this exists
//!
//! Cubism Core reads offsets straight out of the file and dereferences them
//! without checking. That is CVE-2023-27566: any `.moc3` is an arbitrary memory
//! write, and VTuber models are routinely downloaded from strangers. This crate
//! treats every file as hostile, returns typed errors, and never panics.
//!
//! # Shape
//!
//! Borrowing, not owning. The file is held once and every section is a typed
//! view over those bytes. Validation happens at load; after that, lookups are
//! arithmetic.
//!
//! ```ignore
//! let bytes = std::fs::read("model.moc3")?;
//! let moc = Moc3::parse(&bytes)?;      // validates everything
//! for name in moc.part_ids()? { ... }  // cannot fail; already checked
//! ```
//!
//! Provenance is in NOTICE. Format documentation is in
//! `ayatsuri2d-research/docs/16-parser-guide.md`.

#![forbid(unsafe_code)]
#![cfg_attr(not(feature = "std"), no_std)]

pub mod counts;
pub mod error;
pub mod sections;
pub mod sot;
pub mod version;

pub use counts::Counts;
pub use error::Error;
pub use sot::{Extent, SlotState};
pub use version::Version;

/// A parsed, validated `.moc3`.
///
/// Holding `&[u8]` rather than a `Vec` is deliberate: one 2.9 MB model is
/// mostly a single float array, and copying it per section is the difference
/// between a fast load and a slow one.
#[derive(Debug, Clone)]
pub struct Moc3<'a> {
    bytes: &'a [u8],
    version: Version,
    counts: Counts,
    // TODO: hold the offset slice and the sorted distinct offsets used by
    // `extent_of`, so section lookup is arithmetic rather than a scan.
}

impl<'a> Moc3<'a> {
    /// Parse and fully validate.
    ///
    /// TODO, in order:
    ///   1. length >= 64, magic == b"MOC3"
    ///   2. version byte -> Version, else UnknownVersion
    ///   3. byte 5: big-endian is legal but unimplemented -> UnsupportedEndian
    ///   4. offset table, rejecting any offset past the end
    ///   5. count table at slot 0, rejecting negative or overflowing counts
    ///   6. every section: count * elem_size must fit its measured extent
    ///
    /// Step 6 is what makes every later accessor infallible.
    pub fn parse(_bytes: &'a [u8]) -> Result<Self, Error> {
        todo!()
    }

    pub fn version(&self) -> Version { self.version }
    pub fn counts(&self) -> &Counts { &self.counts }

    /// TODO: the 64-byte ID records at `slot`, trimmed at the first NUL.
    ///
    /// Bound the NUL search to 64 bytes. A hostile file will omit it.
    pub fn ids(&self, _slot: usize) -> Result<impl Iterator<Item = &'a str>, Error> {
        // placeholder so the signature type-checks while unimplemented
        Err::<core::iter::Empty<&'a str>, _>(Error::UnknownVersion(0))
    }

    /// TODO: read a section as `i32`. Index fields are SIGNED and -1 means
    /// "none"; reading them unsigned turns that into 4294967295.
    pub fn i32s(&self, _slot: usize) -> Result<&'a [u8], Error> { todo!() }

    /// TODO: read a section as `f32`.
    pub fn f32s(&self, _slot: usize) -> Result<&'a [u8], Error> { todo!() }
}

/// The recurring shape in this format: a begin index and a length, both indexed
/// by the owning object, pointing into a shared pool.
///
/// TODO: validate `begin + len <= pool_len` once, here, so nothing downstream
/// can go out of range. A begin of -1 means none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub begin: u32,
    pub len: u32,
}

impl Span {
    pub fn new(_begin: i32, _len: i32, _pool_len: usize) -> Result<Option<Self>, Error> {
        todo!()
    }
}
