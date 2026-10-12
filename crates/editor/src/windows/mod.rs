// Floating windows: About, Settings, Model Info, Model Statistics and
// Export as moc3. Docked palettes live in panels.

pub(crate) mod about;
mod dialog;
mod export_moc3;
mod model_info;
pub(crate) mod settings;
mod stats;

pub(crate) use dialog::{Dialog, form};
