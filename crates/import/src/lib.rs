//! Foreign formats in, [`ayatsuri_format::Model`] out.
//!
//! This crate is the only bridge between the hostile-input parsers and the
//! runtime: `ayatsuri-core` never sees a `.moc3` (docs/architecture.md). Every
//! cross-pool index is validated here, so the model handed out cannot point
//! out of range.

#![forbid(unsafe_code)]

#[cfg(feature = "documents")]
mod clip;
#[cfg(feature = "documents")]
mod cmo3;
mod exp3;
#[cfg(feature = "documents")]
mod kra;
#[cfg(feature = "documents")]
mod layers;
mod moc3;
mod moc3_export;
mod motion3;
mod physics3;
#[cfg(feature = "documents")]
mod psd_doc;
#[cfg(feature = "documents")]
mod sqlite;

#[cfg(feature = "documents")]
pub use layers::{AlphaBlend, Blend, ColorBlend, Document, Imported, Layer};

/// A paint document's layers by file extension (`psd`, `kra` or `clip`),
/// before they are turned into a model, with how many layers were skipped
/// while reading. Anything else is read as a PSD.
#[cfg(feature = "documents")]
pub fn read_document(bytes: &[u8], ext: &str) -> Result<(Document, usize), ImportError> {
    let read = match ext {
        "kra" => kra::read_kra(bytes),
        "clip" => clip::read_clip(bytes),
        _ => psd_doc::read_psd(bytes).map(|d| (d, 0)),
    };
    match &read {
        Ok((doc, skipped)) => log::debug!(
            "{ext}: {} x {} document, {} layer(s), {skipped} skipped",
            doc.width,
            doc.height,
            doc.layers.len()
        ),
        Err(e) => log::debug!("{ext}: could not read the document: {e}"),
    }
    read
}

/// Whether a file extension names a paint document this crate opens.
#[cfg(feature = "documents")]
pub fn is_paint_ext(ext: &str) -> bool {
    matches!(ext, "psd" | "kra" | "clip")
}

/// A paint document, by extension, as a fresh rig-ready model.
#[cfg(feature = "documents")]
pub fn from_paint(bytes: &[u8], ext: &str) -> Result<Imported, ImportError> {
    let (doc, skipped) = read_document(bytes, ext)?;
    let mut out = layers::build(&doc)?;
    out.skipped += skipped;
    Ok(out)
}

/// Any layered document (already read from its file) as a fresh model.
#[cfg(feature = "documents")]
pub fn from_layers(doc: &Document) -> Result<Imported, ImportError> {
    layers::build(doc)
}

/// A Photoshop document as a fresh, rig-ready model.
#[cfg(feature = "documents")]
pub fn from_psd(bytes: &[u8]) -> Result<Imported, ImportError> {
    let doc = psd_doc::read_psd(bytes)?;
    layers::build(&doc)
}

/// A Krita document as a fresh, rig-ready model. Layers Krita stores in
/// other colour models are skipped and counted.
#[cfg(feature = "documents")]
pub fn from_kra(bytes: &[u8]) -> Result<Imported, ImportError> {
    let (doc, skipped) = kra::read_kra(bytes)?;
    let mut out = layers::build(&doc)?;
    out.skipped += skipped;
    Ok(out)
}

#[cfg(feature = "documents")]
pub use cmo3::{Cmo3, Cmo3Export, from_cmo3, to_cmo3};
pub use exp3::{from_exp3, to_exp3};
pub use moc3::from_moc3;
pub use moc3_export::{ExportVersion, VTS_NOTE, export_warnings, to_moc3};
pub use motion3::{from_motion3, to_motion3};
pub use physics3::{cubism_json, from_physics3, to_physics3};

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
    #[error("exp3.json: {0}")]
    Expression(String),
    #[error("motion3.json: {0}")]
    Motion(String),
    #[error("export: {0}")]
    Export(String),
}
