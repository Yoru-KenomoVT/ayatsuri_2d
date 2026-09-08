//! The section offset table, and how to measure a section.
//!
//! At 0x40. Each entry is the absolute file offset of one section. Sections
//! carry no length: a section runs to the next DISTINCT offset in the table,
//! and the leftover is alignment padding, always under 64 bytes.

use crate::{Error, Version};

/// Where the table starts.
pub const SOT_OFFSET: usize = 0x40;

/// A section's position and measured extent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub offset: u32,
    pub size: u32,
}

/// What an entry means. Three states, and conflating them is a real bug source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    /// Offset 0. The slot does not exist in this version.
    Reserved,
    /// Offset equals the file length. The slot exists and holds zero entries.
    Empty,
    /// A real section.
    Present(Extent),
}

/// TODO: read `version.sot_entries()` little-endian u32 from SOT_OFFSET.
///
/// Reject any non-zero offset greater than the file length before returning:
/// every later bounds check assumes this one already passed.
pub fn parse(_bytes: &[u8], _version: Version) -> Result<&[u32], Error> {
    todo!()
}

/// TODO: classify one entry.
///
///     0            -> Reserved
///     file_len     -> Empty
///     otherwise    -> Present, with size measured by `extent_of`
pub fn classify(_offsets: &[u32], _slot: usize, _file_len: u32) -> SlotState {
    todo!()
}

/// TODO: a section's size is the distance to the next DISTINCT offset.
///
/// Distinct matters: several slots can share one offset, in which case all but
/// the last are zero-length. Sorting the distinct non-zero offsets once at load
/// and binary-searching is cheaper than scanning per lookup, and load time is
/// where this belongs (see ayatsuri2d/docs/03-north-star.md: no allocation and no
/// unbounded work once the frame loop starts).
pub fn extent_of(_offsets: &[u32], _slot: usize, _file_len: u32) -> Option<Extent> {
    todo!()
}
