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
    /// Map the raw header byte. 1..=6 are defined; anything else is an error.
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            1 => Some(Version::V3_00),
            2 => Some(Version::V3_03),
            3 => Some(Version::V4_00),
            4 => Some(Version::V4_02),
            5 => Some(Version::V5_00),
            6 => Some(Version::V6),
            _ => None,
        }
    }

    /// How many `u32` entries the section offset table holds.
    ///
    /// **This is the one people get wrong.** Public references model 160
    /// entries; measured files say 480 in v3 through v5 and 1440 in v6
    /// (GIST-moc3-v6.md section 1). The table runs from 0x40 to the count
    /// table offset held in slot 0, so the length is
    /// `(count_table_offset - 0x40) / 4`, and a v6 file populates entries
    /// 160 through 166, which a reader hardcoded to 160 silently loses.
    pub fn sot_entries(self) -> usize {
        match self {
            Version::V6 => 1440,
            _ => 480,
        }
    }

    /// Size of the count info table in bytes. v5 doubled it to make room for
    /// the blend-shape counts.
    pub fn count_table_bytes(self) -> usize {
        if self >= Version::V5_00 {
            256
        } else {
            128
        }
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
