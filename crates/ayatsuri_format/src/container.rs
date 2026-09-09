use crate::error::Error;

pub const MAGIC: [u8; 4] = *b"AOMX";
pub const HEADER_LENGTH: usize = 12;
pub const CONTAINER_VERSION: u16 = 1;
pub const MAX_P_SIZE: u32 = 256 * 1024 * 1024;

const FLAG_DEFLATE: u16 = 1 << 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags(u16);
impl Flags {
    #[must_use]
    pub const fn compressed(self) -> bool {
        self.0 & FLAG_DEFLATE != 0
    }
    #[must_use]
    pub const fn deflate() -> Self {
        Self(FLAG_DEFLATE)
    }
    pub const fn bits(self) -> u16 {
        self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u16,
    pub flags: Flags,
    pub payload_len: u32,
}

impl Header {
    pub fn parse(bytes: &[u8]) -> Result<(Self, &[u8]) , Error> {
        let short = || Error::TooShort{ need: HEADER_LENGTH, got: bytes.len() };
        let head = bytes.first_chunk::<HEADER_LENGTH>().ok_or_else(short)?;
        let payload = bytes.get(HEADER_LENGTH..).ok_or_else(short)?;

        let [m0 ,m1 ,m2 ,m3 ,v0 ,v1 ,f0 ,f1 ,l0 ,l1 ,l2 ,l3] = *head;

        let magic = [m0,m1,m2,m3];
        if magic != MAGIC {
            return Err(Error::BadMagic(magic));
        }

        let version = u16::from_le_bytes([v0,v1]);
        if version > CONTAINER_VERSION{
            return Err(Error::TooNew(version));
        }

        let bits= u16::from_le_bytes([f0,f1]);
        if bits & !FLAG_DEFLATE != 0 {
            return Err(Error::TooNew(version));
        }
        let flags = Flags(bits);

        let payload_len = u32::from_le_bytes([l0,l1,l2,l3]);
        if payload_len > MAX_P_SIZE {
            return Err(Error::PayloadTooLarge { declared: payload_len, ceiling: MAX_P_SIZE });
        }

        if !flags.compressed() {
            let declared = usize::try_from(payload_len).map_err(|_|Error::PayloadTooLarge{ declared: payload_len,ceiling: MAX_P_SIZE })?;
            if declared != payload.len() {
                return Err(Error::LengthMismatch { declared: payload_len, actual: payload.len() });
            }
        }
        Ok((Self {version, flags, payload_len}, payload))
    }
    pub fn to_bytes(self) -> [u8; HEADER_LENGTH] {
        let [m0,m1,m2,m3] = MAGIC;
        let [v0,v1] = self.version.to_le_bytes();
        let [f0,f1] = self.flags.bits().to_le_bytes();
        let [l0,l1,l2,l3] = self.payload_len.to_le_bytes();
        [m0, m1, m2, m3, v0, v1, f0, f1, l0, l1, l2, l3]
    }
}


