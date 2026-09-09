#![forbid(unsafe_code)]

extern crate alloc;

pub mod buffer;
pub mod container;
pub mod error;
pub mod payload;
pub mod read;
pub mod writer;

pub use container::{Flags, Header, CONTAINER_VERSION, HEADER_LENGTH, MAGIC, MAX_P_SIZE};
pub use error::Error;

