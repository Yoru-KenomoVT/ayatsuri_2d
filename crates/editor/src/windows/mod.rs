// Floating windows: About, Settings, the API window, Model Info, Model
// Statistics and Export as moc3. Docked palettes live in panels.

pub(crate) mod about;
mod api;
mod dialog;
mod export_moc3;
mod model_info;
pub(crate) mod recovery;
pub(crate) mod settings;
mod stats;

pub(crate) use dialog::{Dialog, form};
