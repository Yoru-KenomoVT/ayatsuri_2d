//! Format versions, and the two things that change between them.
//!
//! Everything else about the layout is additive: v4.02 is a byte-exact prefix
//! of v5.00, which is a prefix of v6. Verified against five models. So a reader
//! branches on exactly the two values below and nothing else.

/// The `.moc3` format revision, from byte 4 of the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Version {
    V3_00,
    V3_03,
    V4_00,
    V4_02,
    V5_00,
    /// Cubism Editor 5.3 and later. Adds offscreen surfaces and per-drawable
    /// blend modes.
    V6,
}

impl Version {
    /// TODO: map the raw byte. 1..=6 are defined; anything else is an error.
    pub fn from_byte(_b: u8) -> Option<Self> {
        todo!()
    }

    /// How many `u32` entries the section offset table holds.
    ///
    /// **This is the one people get wrong.** It is not implied by where the
    /// count table begins: the space between the end of the table and the
    /// count table is reserved padding, and reading it as more entries is a
    /// mistake (it is all zeroes, so nothing catches you).
    pub fn sot_entries(self) -> usize {
        match self {
            Version::V6 => 480,
            _ => 160,
        }
    }

    /// Size of the count info table in bytes. v5 doubled it to make room for
    /// the blend-shape counts.
    pub fn count_table_bytes(self) -> usize {
        if self >= Version::V5_00 { 256 } else { 128 }
    }

    /// Highest slot index that carries meaning in this version. Slots above it
    /// are reserved and always zero.
    pub fn highest_slot(self) -> usize {
        match self {
            Version::V6 => 166,
            Version::V5_00 => 151,
            _ => 136,
        }
    }
}
