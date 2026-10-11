//! Foreign formats in, [`ayatsuri_format::Model`] out.
//!
//! This crate is the only bridge between the hostile-input parsers and the
//! runtime: `ayatsuri-core` never sees a `.moc3` (docs/architecture.md). Every
//! cross-pool index is validated here, so the model handed out cannot point
//! out of range.

#![forbid(unsafe_code)]

mod kra;
mod layers;
mod moc3;
mod moc3_export;
mod physics3;
mod psd_doc;

pub use layers::Imported;

/// A Photoshop document as a fresh, rig-ready model.
pub fn from_psd(bytes: &[u8]) -> Result<Imported, ImportError> {
    let doc = psd_doc::read_psd(bytes)?;
    layers::build(&doc)
}

/// A Krita document as a fresh, rig-ready model. Layers Krita stores in
/// other colour models are skipped and counted.
pub fn from_kra(bytes: &[u8]) -> Result<Imported, ImportError> {
    let (doc, skipped) = kra::read_kra(bytes)?;
    let mut out = layers::build(&doc)?;
    out.skipped += skipped;
    Ok(out)
}

pub use moc3::from_moc3;
pub use moc3_export::{ExportVersion, export_warnings, to_moc3};
pub use physics3::{from_physics3, to_motion3, to_physics3};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("moc3: {0}")]
    Moc3(#[from] openmoc3::Error),
    #[error("{what} {at}: index {value} is outside -1..{limit}")]
    BadIndex { what: &'static str, at: usize, value: i64, limit: usize },
    #[error("{what} {at}: span of {len} at {begin} does not fit a pool of {pool}")]
    BadSpan { what: &'static str, at: usize, begin: i64, len: i64, pool: usize },
    #[error("{what} {at}: {got} keyforms stored but the binding calls for {expected}")]
    KeyformMismatch { what: &'static str, at: usize, expected: usize, got: usize },
    #[error("parameter binding {0} is not owned by any parameter")]
    UnownedBinding(usize),
    #[error(
        "deformer {child} appears before its parent {parent}; the hierarchy must be topological"
    )]
    NotTopological { child: usize, parent: usize },
    #[error("deformer {at}: unknown type {value}")]
    UnknownDeformerType { at: usize, value: i32 },
    #[error("physics3.json: {0}")]
    Physics(String),
    #[error("export: {0}")]
    Export(String),
}
