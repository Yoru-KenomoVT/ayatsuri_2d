#![forbid(unsafe_code)]

extern crate alloc;

pub mod buffer;
pub mod container;
pub mod error;
pub mod model;
pub mod payload;
pub mod read;
pub mod writer;

pub use model::Model;
pub use payload::{
    Animation, AomPayload, Curve, CurveTarget, DisplayInfo, Event, Expression, ExpressionBlend,
    ExpressionParam, GuideLine, Interp, Key, Meta, SceneKind, Track,
};
pub use read::read;
pub use writer::write;

pub use container::{CONTAINER_VERSION, Flags, HEADER_LENGTH, Header, MAGIC, MAX_P_SIZE};
pub use error::Error;
