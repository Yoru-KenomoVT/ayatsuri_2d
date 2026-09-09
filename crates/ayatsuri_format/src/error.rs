use alloc::{fmt, string::String};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    //Container errors
    TooShort { need: usize,got:usize },
    BadMagic([u8; 4]),
    TooNew(u16),
    PayloadTooLarge { declared: u32, ceiling: u32 },
    LengthMismatch { declared: u32, actual: usize },
    //Deflation
    Inflate(String),
    InflateShort { declared: u32, produced: usize },
    //Blobs
    BlobOutRange { off: u32, len: u32, image: usize },
    BlobMisaligned { off : u32 },
    BlobNotWholeElements { len : u32, element: usize},
    //mapping
    Decode(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { need, got } => write!(f,"too short. need : {need}|got : {got}"),
            Self::BadMagic(got) => write!(f, "bad magic expected \"AOMX\" but got {got:02x?}"),
            Self::TooNew(v) => write!(f, "Version {v} is newer than what the current build reads"),
            Self::PayloadTooLarge{ declared, ceiling} =>{ write!(f, "declared payload exceeds the ceiling. payload: {declared}|ceiling: {ceiling}")}
            Self::LengthMismatch { declared,actual}=> {write!(f, "uncompressed paayload declared {declared}, found {actual}")}
            Self::Inflate(why)=> write!(f, "inflate failed read error: {why}"),
            Self::InflateShort{ declared,produced } =>{ write!(f, "Inflate produced unexpected bytes. result: {produced}|expected: {declared} ")}
            Self::BlobOutRange{off,len,image} =>{ write!(f, "Blob art {off} len {len} runs past the {image}-bytes")}
            Self::BlobMisaligned{off} =>{write!(f, "blob offset {off} is not a multiple of 16")}
            Self::BlobNotWholeElements{ len, element}=> {write!(f, "blob length {len} is not a whole number of {element}-byte")}
            Self::Decode(why) => write!(f, "could not decode map read error for more info: {why}"),
        }
    }
}



