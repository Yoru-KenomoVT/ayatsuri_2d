//! The section offset table, and how to measure a section.
//!
//! At 0x40. Each entry is the absolute file offset of one section. Sections
//! carry no length: a section runs to the next DISTINCT offset in the table,
//! and the leftover is alignment padding, always under 64 bytes.

use crate::{read_u32, Error, Version};

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

/// The validated table: a view over the entry bytes, length derived from
/// slot 0 the way the format defines it.
#[derive(Debug, Clone, Copy)]
pub struct Sot<'a> {
    bytes: &'a [u8],
    entries: usize,
}

impl<'a> Sot<'a> {
    /// Read the table. Slot 0 is the count table offset, which also marks
    /// where this table ends: `entries = (count_table_offset - 0x40) / 4`,
    /// 480 in v3 through v5 and 1440 in v6. Every non-zero offset is checked
    /// against the file length here, so every later bounds check can assume
    /// this one passed.
    pub fn parse(file: &'a [u8], version: Version) -> Result<Self, Error> {
        let file_len = file.len();
        let cto = read_u32(file, SOT_OFFSET)
            .ok_or(Error::Truncated { need: SOT_OFFSET + 4, have: file_len })?;
        let cto_us = cto as usize;
        let rel = cto_us.checked_sub(SOT_OFFSET).filter(|r| *r >= 4 && r.is_multiple_of(4));
        let Some(rel) = rel else {
            return Err(Error::OffsetOutOfBounds { slot: 0, offset: cto });
        };
        if cto_us > file_len {
            return Err(Error::OffsetOutOfBounds { slot: 0, offset: cto });
        }
        let entries = rel / 4;
        if entries != version.sot_entries() {
            return Err(Error::BadSotLength { entries, expected: version.sot_entries() });
        }
        let bytes = file
            .get(SOT_OFFSET..cto_us)
            .ok_or(Error::Truncated { need: cto_us, have: file_len })?;
        let sot = Self { bytes, entries };
        for slot in 0..entries {
            let offset = sot.get(slot).unwrap_or(0);
            if offset as usize > file_len {
                return Err(Error::OffsetOutOfBounds { slot, offset });
            }
        }
        Ok(sot)
    }

    pub fn len(&self) -> usize {
        self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries == 0
    }

    /// The raw offset of a slot, if the slot exists in the table.
    pub fn get(&self, slot: usize) -> Option<u32> {
        read_u32(self.bytes, slot.checked_mul(4)?)
    }
}

/// Classify one entry.
///
/// ```text
/// 0            -> Reserved
/// file_len     -> Empty
/// otherwise    -> Present, with size measured by extent_of
/// ```
pub fn classify(sot: &Sot<'_>, slot: usize, file_len: u32) -> SlotState {
    match sot.get(slot) {
        None | Some(0) => SlotState::Reserved,
        Some(offset) if offset == file_len => SlotState::Empty,
        Some(_) => match extent_of(sot, slot, file_len) {
            Some(e) => SlotState::Present(e),
            None => SlotState::Reserved,
        },
    }
}

/// A section's size is the distance to the next DISTINCT offset.
///
/// Distinct matters: several slots can share one offset, in which case all but
/// the last are zero-length. A linear scan over at most 1440 entries happens
/// only at load and in accessors, never on the frame path, so no sorted copy
/// is kept (ayatsuri2d/docs/goals.md: the budget guards the frame loop, not
/// the loader).
pub fn extent_of(sot: &Sot<'_>, slot: usize, file_len: u32) -> Option<Extent> {
    let offset = sot.get(slot)?;
    if offset == 0 || offset >= file_len {
        return None;
    }
    let mut end = file_len;
    for i in 0..sot.len() {
        if let Some(o) = sot.get(i) {
            if o > offset && o < end {
                end = o;
            }
        }
    }
    Some(Extent { offset, size: end.saturating_sub(offset) })
}
