# Changelog

## 0.5.3 beta (2026-10-12)

### Added

- **Import PSD into an open model.** File > Import PSD… adds a Photoshop
  or Krita file's folders and layers to the model you have open, as new
  parts and meshes on new texture pages. Start from File > New Model and
  import your art, or bring in more art later. A model with no art yet
  takes the file's canvas; otherwise layers line up from the top-left.
  Models opened from a moc3 get the new meshes in their draw order.
  Names, animations, expressions and parameter folders stay, and one
  undo takes the import back out.
- **Lasso select, copy and paste for mesh points.** Mesh Edit has a Lasso
  select mode: drag around points (Shift adds, click picks one). Ctrl+C
  copies them with the faces between them, and Ctrl+V pastes them into
  the mesh being edited at the same canvas spots. Points landing on
  existing ones merge into them, so two meshes line up for glue. New
  points take the art under them and follow the mesh in every pose.
  Delete removes the picked points. Edit > Copy and Paste work too.

### Changed

- The Edit Mesh popup now shows the same controls as Tool Details' mesh
  section, so both always match.
- The Lasso in Tool Details now works (it was a placeholder), and the
  unbuilt "Add more vertices" paint button is gone.

## 0.5.2 beta (2026-10-12)

### Added

- **Delete art meshes.** Right-click a mesh in the Parts or Deformer
  palette and choose Delete Art Mesh, or press Delete with the Arrow tool.
  A multi-selection goes together. Glues on the mesh are removed and other
  meshes stop clipping by it. One undo step brings it back with its name.
- **Opacity for meshes and deformers.** Properties has an Opacity slider
  that sets the keyform the parameters sit on, as in Cubism: put the
  parameters on a key, set the opacity, and the runtime fades between keys.
  The art fades live while dragging. A deformer's opacity fades everything
  under it. Exported to moc3.
- **Clipping mask picker.** Properties shows the masks as chips (click one
  to remove it), Pick on canvas to click the mask mesh (Shift+click for
  several, Esc ends), and a + list. Clip by… is also in the palette menus.

### Fixed

- **Reflect** now works like Cubism's: it mirrors the key under the slider
  into the opposite key, after asking before it overwrites that keyform.
  It used to swap the end keys silently.
- The part opacity slider snapped back to 0% or 100%. It now holds any
  value. It is labelled View opacity: moc3 parts have no opacity, so it is
  not saved or exported. To fade a part by a parameter, key the Opacity of
  a deformer that holds it.

## 0.5.1 beta (2026-10-12)

The first public build of Ayatsuri2D: a free, open-source 2D rigging editor
that reads and writes Live2D Cubism models. It is a beta. Save often, and
please report anything that breaks.

### Downloads

| System  | File | Notes |
|---------|------|-------|
| Windows | `ayatsuri2d-0.5.1-windows-x64-setup.exe` | Installer: Start Menu entry, optional desktop shortcut and `.aom` association |
| Windows | `ayatsuri2d-0.5.1-windows-x64.zip` | Portable, unzip and run `ayatsuri-launcher.exe` |
| macOS   | `ayatsuri2d-0.5.1-macos-universal.dmg` | Apple Silicon and Intel, macOS 11 or later. Not notarized: the first time, right-click the app and choose Open |
| Linux   | `.AppImage` | Any distro, mark it executable and run |
| Linux   | `.deb` / `.rpm` | Debian, Ubuntu, Mint / Fedora, openSUSE |
| Linux   | `.tar.gz` | Run `./install.sh` (per user, or `--prefix`, `--uninstall`) |

`SHA256SUMS.txt` holds the checksum of every file.

### Models in and out

- Open Cubism models (`.model3.json` bundles and `.moc3` 3.0 to 5.3) with
  their textures, physics, display info, motions and expressions.
- Open Photoshop and Krita documents as new models: groups become parts,
  layers become meshes packed into texture atlases, clipping becomes masks.
- Save as `.aom`, Ayatsuri's own open model format.
- Export as moc3 with the full bundle: `.moc3`, `model3.json`, `physics3.json`,
  `cdi3.json`, textures, one `motion3.json` per animation and one `exp3.json`
  per expression. Exports load in VTube Studio and OpenVT.
  - Versions 3.0 (with a warning), 3.3, 4.0, 4.2, 5.0 and 5.3. The default is 5.0. 5.3 does not
    open in VTube Studio or OpenVT yet (their Cubism Core stops at 5.0),
    and the export dialog says so.
  - EyeBlink and LipSync groups are kept, or filled from the standard
    parameters when a model has none.

### Modeling

- Mesh editing: add points anywhere, auto connect, connect and cut edges,
  erase brush, 4-division and reset, auto mesh that follows the artwork
  outline (Standard, Deform and Light presets).
- In Mesh Edit the art stays still: points slide over the texture instead
  of stretching it.
- Glue: bind overlapping points of two meshes, weight brush, glue tags.
- Warp deformers with grid points, the Cubism box (move, scale, rotate),
  Bezier control points and edit levels 1, 2 and 3.
- Rotation deformers. Dragging the centre moves the pivot, not the mesh.
- Deform paths, lasso and marquee point picking, brush selection with
  per-point weights.
- Deformer palette: right-click to create, rename, delete and select
  children. Drag a row onto another to re-parent it; the art does not move.
  Deleting a deformer keeps what it held in place.
- Every drag lands exactly where you let go, and the art follows the drag
  live (meshes, warp grids, Bezier handles and rotation).
- Shift or Ctrl+click to multi-select meshes.

### Parameters, physics and animation

- Parameter palette with folders, Cubism's menus, four-corner forms,
  motion mirroring, parameter pads, and multi-select (Ctrl+click,
  Shift+click a range) for grouping.
- Physics Settings page laid out like Cubism's, live pendulum preview that
  follows the real parameters, panel sizes remembered.
- Animation scenes with curves: auto smooth, linear, stepped, inverse
  stepped and free Bezier keys, events and fades. `motion3.json` reads and
  writes exactly (tested key-for-key on real Cubism motions).
- Expressions: `exp3.json` read and write with Add, Multiply and Overwrite.

### Ayatsuri API

- Cubism Editor's External Application Integration API on a port you set
  (default 22033), so apps made for Cubism Editor connect unchanged.
  VTube Studio's parameter sync works with it.
- The API menu turns it on and off, and the API window shows connected
  apps, the parameters they drive and a live message log. Apps must be
  allowed before they can drive anything; localhost only unless you open
  it to the network in Settings.

### The app

- Launcher with a particle splash in your primary colour (can be skipped
  or turned off).
- Settings: General, Appearance (theme, primary colour with alpha, scale,
  text size, density, corners, reduce motion, canvas and overlays),
  Performance, Keyboard (30 rebindable shortcuts with Cubism defaults) and
  API.
- Home screen with recent models and a right-click menu (open, show in
  folder, rename, duplicate, copy path, remove).
- About window with full open-source credits for every bundled crate.
- Crash safety: a crash undoes the edit it interrupted, saves a recovery
  copy and writes a full log. Autosave every two minutes. On the next
  start, Recover Unsaved Work offers anything left behind.
- File dialogs never freeze the window.
- Discord Rich Presence (optional).

### Known limits

- Builds are not signed by Apple or Microsoft, so macOS asks once (right
  click, Open) and Windows SmartScreen may warn ("More info", "Run anyway").
- Not yet built: the art path brush, texture atlas editor, skinning,
  model templates, the temporary deform tool, an expression editor, and
  the full timeline (curve editor, graph view).
- Art meshes cannot be deleted yet, and mesh opacity cannot be keyed to
  parameters (the part opacity slider only changes the view). Clipping is
  set by typing mesh IDs into Clip by in Properties.
- Dragging a weighted selection across several meshes at once is not
  supported yet.
