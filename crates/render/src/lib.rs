//! The renderer, behind a trait so backends are swappable.
//!
//! `ayatsuri-core` produces vertex positions and a draw list; this crate turns
//! them into pixels. Pipelines are built at load, never mid-frame, and nothing
//! in the frame path reads back from the GPU (docs/goals.md).

#![forbid(unsafe_code)]

pub mod gpu;

pub use gpu::{BlendMode, Cmd, GpuRenderer, MaskMode, Packet, TexData, Vertex};
pub use wgpu;
