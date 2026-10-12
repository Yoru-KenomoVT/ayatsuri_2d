// The Ayatsuri API: Cubism Editor's External Application Integration API
// (docs.live2d.com) served by Ayatsuri, on a port set in Settings, so apps
// built for Cubism Editor (VTube Studio's param sync among them) work with
// Ayatsuri unchanged.

pub(crate) mod protocol;
pub(crate) mod server;

/// Cubism Editor's default port, so clients find us without setup.
pub(crate) const DEFAULT_PORT: u16 = 22033;
