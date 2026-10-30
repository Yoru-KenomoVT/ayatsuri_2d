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
//! ```text
//! let bytes = std::fs::read("model.moc3")?;
//! let moc = Moc3::parse(&bytes)?;          // validates everything
//! for name in moc.ids(slot::PART_IDS)? { ... }
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
pub use sections::slot;
pub use sot::{Extent, SlotState, Sot};
pub use version::Version;

/// Header length: magic, version, endianness and reserved bytes.
pub const HEADER_LEN: usize = 0x40;

/// A little-endian u32 at `at`, if the buffer holds one there.
pub(crate) fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    let arr: [u8; 4] = s.try_into().ok()?;
    Some(u32::from_le_bytes(arr))
}

/// A little-endian i32 at `at`. Index fields are SIGNED and -1 means "none";
/// reading them unsigned turns that into 4294967295.
pub(crate) fn read_i32(b: &[u8], at: usize) -> Option<i32> {
    let s = b.get(at..at.checked_add(4)?)?;
    let arr: [u8; 4] = s.try_into().ok()?;
    Some(i32::from_le_bytes(arr))
}

/// Slots above this are version-gated additions (v3.3, v4.2, v5, v6): absent
/// in older files even when their counts are non-zero, so a missing section
/// is an error only for the core v3 layout.
const V3_CORE_MAX_SLOT: usize = 100;

/// The ID tables, validated at parse so iteration cannot fail.
const ID_SLOTS: [usize; 5] =
    [slot::PART_IDS, slot::DEFORMER_IDS, slot::ART_MESH_IDS, slot::PARAMETER_IDS, slot::GLUE_IDS];

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
    sot: Sot<'a>,
}

impl<'a> Moc3<'a> {
    /// Parse and fully validate, in the order the format guide gives:
    /// header, offset table, count table, then `count * elem_size` against
    /// every section's measured extent, then the ID tables. Everything a
    /// later accessor relies on is checked here.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let have = bytes.len();
        if have < HEADER_LEN + 4 {
            return Err(Error::Truncated { need: HEADER_LEN + 4, have });
        }
        let magic: [u8; 4] = bytes
            .get(0..4)
            .and_then(|s| s.try_into().ok())
            .ok_or(Error::Truncated { need: 4, have })?;
        if &magic != b"MOC3" {
            return Err(Error::BadMagic(magic));
        }
        let vbyte = bytes.get(4).copied().ok_or(Error::Truncated { need: 5, have })?;
        let version = Version::from_byte(vbyte).ok_or(Error::UnknownVersion(vbyte))?;
        if bytes.get(5).copied().unwrap_or(0) != 0 {
            return Err(Error::UnsupportedEndian);
        }

        let sot = Sot::parse(bytes, version)?;
        let cto = sot.get(0).unwrap_or(0) as usize;
        let ct_need = cto.saturating_add(version.count_table_bytes());
        let ct = bytes.get(cto..ct_need).ok_or(Error::Truncated { need: ct_need, have })?;
        let counts = Counts::parse(ct)?;
        if !counts.is_self_consistent() {
            return Err(Error::CountsInconsistent {
                deformers: counts.deformers,
                warp: counts.warp_deformers,
                rotation: counts.rotation_deformers,
            });
        }

        let moc = Self { bytes, version, counts, sot };
        moc.check_sections()?;
        moc.check_ids()?;
        Ok(moc)
    }

    /// Every described section must have room for `count * elem_size`. A core
    /// slot that is missing while its count is non-zero is an error; a
    /// version-gated slot (101 and up) is only checked when present.
    fn check_sections(&self) -> Result<(), Error> {
        let file_len = self.bytes.len() as u32;
        for desc in sections::SECTIONS {
            if desc.slot > self.version.highest_slot() {
                continue;
            }
            let count = (desc.count_of)(&self.counts).max(0) as usize;
            let need = count.saturating_mul(desc.elem_size);
            match sot::classify(&self.sot, desc.slot, file_len) {
                SlotState::Present(e) => {
                    if need > e.size as usize {
                        return Err(Error::SectionTooSmall {
                            slot: desc.slot,
                            need,
                            have: e.size as usize,
                        });
                    }
                }
                SlotState::Reserved | SlotState::Empty => {
                    if need > 0 && desc.slot <= V3_CORE_MAX_SLOT {
                        return Err(Error::SectionTooSmall { slot: desc.slot, need, have: 0 });
                    }
                }
            }
        }
        Ok(())
    }

    /// Every 64-byte ID record must be NUL-terminated ASCII. A hostile file
    /// will omit the terminator; bounding the search to the record is what
    /// keeps `ids` infallible afterwards.
    fn check_ids(&self) -> Result<(), Error> {
        for id_slot in ID_SLOTS {
            let Some(desc) = sections::describe(id_slot) else { continue };
            let count = (desc.count_of)(&self.counts).max(0) as usize;
            let Some(data) = self.section_bytes(id_slot) else { continue };
            for at in 0..count {
                let start = at.saturating_mul(sections::elem::ID);
                let rec = data.get(start..start.saturating_add(sections::elem::ID));
                let ok = rec.is_some_and(|r| {
                    r.iter()
                        .position(|&b| b == 0)
                        .is_some_and(|n| r.get(..n).is_some_and(|s| s.is_ascii()))
                });
                if !ok {
                    return Err(Error::BadId { slot: id_slot, at });
                }
            }
        }
        Ok(())
    }

    pub fn version(&self) -> Version {
        self.version
    }

    pub fn counts(&self) -> &Counts {
        &self.counts
    }

    /// A slot's classification: reserved, empty, or present with an extent.
    pub fn slot_state(&self, slot: usize) -> SlotState {
        sot::classify(&self.sot, slot, self.bytes.len() as u32)
    }

    /// A present section's raw bytes, padding included.
    pub fn section_bytes(&self, slot: usize) -> Option<&'a [u8]> {
        match self.slot_state(slot) {
            SlotState::Present(e) => {
                let start = e.offset as usize;
                self.bytes.get(start..start.saturating_add(e.size as usize))
            }
            _ => None,
        }
    }

    /// A section's entries as 64-byte ID records, trimmed at the first NUL.
    /// Validated at parse, so iteration cannot fail.
    pub fn ids(&self, id_slot: usize) -> Result<Ids<'a>, Error> {
        let desc = sections::describe(id_slot).ok_or(Error::NoSuchSection { slot: id_slot })?;
        if desc.elem_size != sections::elem::ID {
            return Err(Error::NoSuchSection { slot: id_slot });
        }
        let count = (desc.count_of)(&self.counts).max(0) as usize;
        Ok(Ids { bytes: self.section_bytes(id_slot).unwrap_or(&[]), count, next: 0 })
    }

    /// A section's entries as signed 32-bit integers (-1 means "none").
    pub fn i32s(&self, slot: usize) -> Result<I32s<'a>, Error> {
        Ok(I32s { bytes: self.typed_section(slot, 4)? })
    }

    /// A section's entries as 32-bit floats.
    pub fn f32s(&self, slot: usize) -> Result<F32s<'a>, Error> {
        Ok(F32s { bytes: self.typed_section(slot, 4)? })
    }

    /// A section's entries as signed 16-bit indices (position indices, glue).
    pub fn i16s(&self, slot: usize) -> Result<I16s<'a>, Error> {
        Ok(I16s { bytes: self.typed_section(slot, 2)? })
    }

    /// A section's entries as raw bytes (the one-byte drawable flags).
    pub fn u8s(&self, slot: usize) -> Result<&'a [u8], Error> {
        self.typed_section(slot, 1)
    }

    /// Canvas info (slot 1): five floats and a flag word in a 64-byte record.
    /// Field order verified by the research hexpat.
    pub fn canvas_info(&self) -> Result<CanvasInfo, Error> {
        let data = self
            .section_bytes(slot::CANVAS_INFO)
            .ok_or(Error::NoSuchSection { slot: slot::CANVAS_INFO })?;
        let f = |at: usize| read_u32(data, at).map(f32::from_bits);
        match (f(0), f(4), f(8), f(12), f(16), read_u32(data, 20)) {
            (
                Some(pixels_per_unit),
                Some(origin_x),
                Some(origin_y),
                Some(width),
                Some(height),
                Some(flags),
            ) => Ok(CanvasInfo { pixels_per_unit, origin_x, origin_y, width, height, flags }),
            _ => {
                Err(Error::SectionTooSmall { slot: slot::CANVAS_INFO, need: 24, have: data.len() })
            }
        }
    }

    /// The section's byte view cut to exactly `count * elem` (padding off),
    /// for a slot whose elements are `elem` bytes wide.
    fn typed_section(&self, slot: usize, elem: usize) -> Result<&'a [u8], Error> {
        let desc = sections::describe(slot).ok_or(Error::NoSuchSection { slot })?;
        if desc.elem_size != elem {
            return Err(Error::NoSuchSection { slot });
        }
        let count = (desc.count_of)(&self.counts).max(0) as usize;
        let need = count.saturating_mul(elem);
        let data = self.section_bytes(slot).unwrap_or(&[]);
        // parse() proved need <= data.len() for every present section, and a
        // missing section only ever pairs with a zero count
        data.get(..need).ok_or(Error::SectionTooSmall { slot, need, have: data.len() })
    }
}

/// The stage the model is placed on, from slot 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasInfo {
    pub pixels_per_unit: f32,
    pub origin_x: f32,
    pub origin_y: f32,
    pub width: f32,
    pub height: f32,
    pub flags: u32,
}

/// Iterator over 64-byte ID records, yielded as `&str` up to the first NUL.
#[derive(Debug, Clone)]
pub struct Ids<'a> {
    bytes: &'a [u8],
    count: usize,
    next: usize,
}

impl<'a> Iterator for Ids<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if self.next >= self.count {
            return None;
        }
        let at = self.next;
        self.next = self.next.saturating_add(1);
        let start = at.saturating_mul(sections::elem::ID);
        let rec = self.bytes.get(start..start.saturating_add(sections::elem::ID))?;
        let n = rec.iter().position(|&b| b == 0)?;
        core::str::from_utf8(rec.get(..n)?).ok()
    }
}

/// A section viewed as little-endian i32 entries.
#[derive(Debug, Clone, Copy)]
pub struct I32s<'a> {
    bytes: &'a [u8],
}

impl<'a> I32s<'a> {
    pub fn len(&self) -> usize {
        self.bytes.len() / 4
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<i32> {
        read_i32(self.bytes, i.checked_mul(4)?)
    }

    pub fn iter(&self) -> impl Iterator<Item = i32> + 'a {
        let bytes = self.bytes;
        (0..self.len()).filter_map(move |i| read_i32(bytes, i.saturating_mul(4)))
    }
}

/// A section viewed as little-endian f32 entries.
#[derive(Debug, Clone, Copy)]
pub struct F32s<'a> {
    bytes: &'a [u8],
}

impl<'a> F32s<'a> {
    pub fn len(&self) -> usize {
        self.bytes.len() / 4
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<f32> {
        Some(f32::from_bits(read_u32(self.bytes, i.checked_mul(4)?)?))
    }

    pub fn iter(&self) -> impl Iterator<Item = f32> + 'a {
        let bytes = self.bytes;
        (0..self.len())
            .filter_map(move |i| read_u32(bytes, i.saturating_mul(4)).map(f32::from_bits))
    }
}

/// A section viewed as little-endian i16 entries.
#[derive(Debug, Clone, Copy)]
pub struct I16s<'a> {
    bytes: &'a [u8],
}

impl<'a> I16s<'a> {
    pub fn len(&self) -> usize {
        self.bytes.len() / 2
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<i16> {
        let s = self.bytes.get(i.checked_mul(2)?..i.checked_mul(2)?.checked_add(2)?)?;
        let arr: [u8; 2] = s.try_into().ok()?;
        Some(i16::from_le_bytes(arr))
    }
}

/// The recurring shape in this format: a begin index and a length, both indexed
/// by the owning object, pointing into a shared pool. Validated once, at
/// construction, so nothing downstream can go out of range. A begin of -1
/// means none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub begin: u32,
    pub len: u32,
}

impl Span {
    pub fn new(begin: i32, len: i32, pool_len: usize) -> Result<Option<Self>, Error> {
        if begin == -1 {
            return Ok(None);
        }
        if begin < 0 || len < 0 {
            return Err(Error::BadSpan { begin, len, pool: pool_len });
        }
        let end = (begin as usize).checked_add(len as usize);
        match end {
            Some(end) if end <= pool_len => Ok(Some(Self { begin: begin as u32, len: len as u32 })),
            _ => Err(Error::BadSpan { begin, len, pool: pool_len }),
        }
    }
}

#[cfg(all(test, feature = "std"))]
#[allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
mod tests {
    use super::*;

    const SOT_ENTRIES: usize = 480; // v4.02
    const CTO: usize = HEADER_LEN + SOT_ENTRIES * 4; // 0x7C0
    const CT_LEN: usize = 128;

    struct Builder {
        bytes: Vec<u8>,
    }

    impl Builder {
        /// A minimal valid v4.02 file: header, offset table, zeroed counts.
        fn v402() -> Self {
            let mut bytes = vec![0u8; CTO + CT_LEN];
            bytes[0..4].copy_from_slice(b"MOC3");
            bytes[4] = 4;
            bytes[5] = 0;
            bytes[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&(CTO as u32).to_le_bytes());
            Self { bytes }
        }

        fn set_count(&mut self, index: usize, value: i32) -> &mut Self {
            let at = CTO + index * 4;
            self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            self
        }

        /// Append a section and point `slot` at it.
        fn add_section(&mut self, slot: usize, data: &[u8]) -> &mut Self {
            let at = self.bytes.len() as u32;
            let entry = HEADER_LEN + slot * 4;
            self.bytes[entry..entry + 4].copy_from_slice(&at.to_le_bytes());
            self.bytes.extend_from_slice(data);
            self
        }

        fn build(&self) -> Vec<u8> {
            self.bytes.clone()
        }
    }

    fn id_record(name: &str) -> [u8; 64] {
        let mut rec = [0u8; 64];
        rec[..name.len()].copy_from_slice(name.as_bytes());
        rec
    }

    #[test]
    fn minimal_file_parses() {
        let f = Builder::v402().build();
        let moc = Moc3::parse(&f).unwrap();
        assert_eq!(moc.version(), Version::V4_02);
        assert_eq!(moc.counts().parts, 0);
    }

    #[test]
    fn one_part_round_trips_its_id() {
        let mut b = Builder::v402();
        b.set_count(0, 1); // parts
        b.add_section(2, &[0u8; 8]); // runtime space
        b.add_section(3, &id_record("Part01")); // IDs
        for slot in 4..=9 {
            b.add_section(slot, &(-1i32).to_le_bytes());
        }
        let f = b.build();
        let moc = Moc3::parse(&f).unwrap();
        let names: Vec<&str> = moc.ids(slot::PART_IDS).unwrap().collect();
        assert_eq!(names, ["Part01"]);
        let parents = moc.i32s(9).unwrap();
        assert_eq!(parents.get(0), Some(-1));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut f = Builder::v402().build();
        f[0] = b'X';
        assert!(matches!(Moc3::parse(&f), Err(Error::BadMagic(_))));
    }

    #[test]
    fn rejects_truncated() {
        assert!(matches!(Moc3::parse(&[0u8; 10]), Err(Error::Truncated { .. })));
    }

    #[test]
    fn rejects_unknown_version() {
        let mut f = Builder::v402().build();
        f[4] = 9;
        assert!(matches!(Moc3::parse(&f), Err(Error::UnknownVersion(9))));
    }

    #[test]
    fn rejects_big_endian() {
        let mut f = Builder::v402().build();
        f[5] = 1;
        assert!(matches!(Moc3::parse(&f), Err(Error::UnsupportedEndian)));
    }

    #[test]
    fn rejects_offset_out_of_bounds() {
        let mut f = Builder::v402().build();
        let entry = HEADER_LEN + 3 * 4;
        let huge = (f.len() as u32 + 100).to_le_bytes();
        f[entry..entry + 4].copy_from_slice(&huge);
        assert!(matches!(Moc3::parse(&f), Err(Error::OffsetOutOfBounds { slot: 3, .. })));
    }

    #[test]
    fn rejects_count_without_section() {
        let f = Builder::v402().set_count(0, 1).build();
        assert!(matches!(Moc3::parse(&f), Err(Error::SectionTooSmall { .. })));
    }

    #[test]
    fn rejects_negative_count() {
        let f = Builder::v402().set_count(0, -5).build();
        assert!(matches!(Moc3::parse(&f), Err(Error::ImplausibleCount { index: 0, value: -5 })));
    }

    #[test]
    fn rejects_inconsistent_deformers() {
        let f = Builder::v402().set_count(1, 3).build();
        assert!(matches!(Moc3::parse(&f), Err(Error::CountsInconsistent { .. })));
    }

    #[test]
    fn rejects_id_without_terminator() {
        let mut b = Builder::v402();
        b.set_count(0, 1);
        b.add_section(2, &[0u8; 8]);
        b.add_section(3, &[0x41u8; 64]); // 64 x 'A', no NUL
        for slot in 4..=9 {
            b.add_section(slot, &(-1i32).to_le_bytes());
        }
        let f = b.build();
        assert!(matches!(Moc3::parse(&f), Err(Error::BadId { slot: 3, at: 0 })));
    }

    #[test]
    fn span_validates_once() {
        assert_eq!(Span::new(-1, 0, 10), Ok(None));
        assert_eq!(Span::new(2, 3, 10), Ok(Some(Span { begin: 2, len: 3 })));
        assert!(Span::new(8, 3, 10).is_err());
        assert!(Span::new(-2, 1, 10).is_err());
    }

    /// The deliberately malformed model from CVE-2023-27566. A correct parser
    /// rejects it cleanly, without panicking. Skipped when the research
    /// checkout is not beside this repo.
    #[test]
    fn exploit_moc3_is_rejected_not_crashed() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../ayatsuri2d-research/corpus/moc3ingbird/exploit.moc3"
        );
        let Ok(bytes) = std::fs::read(path) else { return };
        assert!(Moc3::parse(&bytes).is_err());
    }
}
