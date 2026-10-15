# Changelog

## Unreleased

### Added

- **Timelapse**: every step of building a model (each edit, undo and redo)
  is kept in a small `.timelapse` file beside it, as compressed changes.
  Window > Timelapse plays it back with a scrubber and exports PNG frames
  and a GIF, plus an MP4 when ffmpeg is installed. Turn it off in
  Settings > General.
- **ArtPath**: draw Bezier paths on the canvas (click for a corner, drag
  for a smooth point) with a stroke and an optional fill. Each becomes an
  ordinary ArtMesh, so it clips, sits under deformers, takes keys and
  exports to moc3. Select one with the ArtPath tool to edit its points.
- **Clip with a path**: Clip meshes with this path, then click meshes on
  the canvas (Alt+click or Inverted for an inverted mask). A path used as a
  mask turns **Mask only**: it still clips but doesn't draw.
- **Eyedropper** for ArtPath colours, picked from the art itself.
- **Deform Brush**: push the selected meshes' points, or a warp's grid,
  along the stroke, with size, strength and hardness. Live, one undo step.
- **Guide lines**, as in Cubism: right-click the canvas > Guide Line to
  add horizontal or vertical lines at the click, in the center or at the
  selected vertex, or drag them out of the rulers. View > Guide Line has
  the settings window (type, position, colour, memo per line), a rulers
  toggle and snapping: moving vertices land on guide lines, which turn
  green. Saved with the model.
- The Show menu is now called **View**.
- **Several selected meshes edit as one**: one box moves, scales and
  rotates them all (#8).
- The update banner and About show **What's new** for the release on
  offer: its changelog, folded away until opened.
- The Clip by list has a search box and lists ArtPaths first.

### Fixed

- Running out of video memory no longer crashes the editor (and no longer
  crashes it again every frame); the canvas says so instead (#3).
- Re-import PSD no longer piles up texture pages: pages no mesh uses are
  dropped after a re-import and when a model opens (#4).
- Mesh Edit holds one full-size texture page at a time instead of keeping
  every page it showed (#5).
- No CPU use while nothing moves with Follow or Physics on (#6).
- Ctrl+click toggles a mesh in and out of the selection in the Parts
  palette, the Deformer palette (which gets multi-select) and on the
  canvas (#7).
- Every brush sizes the same way: B + drag, or [ and ] (#9).
- Warp Bezier handles stay as you leave them instead of springing back
  to a third of the span, and are saved with the model (#10).
- Undo history is capped by memory (256 MB) as well as by steps, so big
  rigs can't fill RAM with undo.

## 0.5.5 beta (2026-10-14)

### Added

- **Re-import PSD** (File > Re-import PSD…): an updated PSD's layers find
  their meshes by folder and name and swap in the new art; the rig stays
  exactly as it is. Layers and meshes that don't match are listed, and one
  undo puts the old art back.
- **Interface languages**: Afrikaans, Dutch, Japanese, Korean and
  Simplified Chinese besides English, in Settings > General > Language
  (follows the system by default). The translations were made with tools
  (AI and machine translation such as Google Translate), so corrections
  are welcome. Each language is one JSON file in `assets/i18n`; a file of
  the same name in the config folder's `i18n` directory overrides it or
  adds a new language without a rebuild. Chinese, Japanese and Korean use
  the system's fonts.
- **Updates**: the launcher asks GitHub for a newer release while the
  splash shows, the home screen shows a banner, and About has Check for
  updates. **Update now** downloads the release for your install, checks
  it against the release's SHA256SUMS.txt, puts it in place (the AppImage,
  the Linux tarball install, or the Windows installer run quietly) and
  Restart now starts it. Package-manager installs, the portable zip and
  macOS get the release page instead. Turn the check off in Settings >
  General.
- **Search** in the Parts and Deformer palettes.
- **Multi-select in the Parts palette**: Ctrl+click and Shift+click for
  parts and meshes; Hide/Show acts on the whole pick.
- Hovering a Parts row outlines its meshes on the canvas, and a mesh row
  shows its art enlarged.
- Deform paths show under their mesh in the Deformer palette, with edit
  and delete.

### Changed

- Add keys, Delete key and Opacity act on **every selected mesh**.
- A deformer created over pieces under different deformers goes under
  their shared parent and wraps their branches, so a toggle warp can sit
  above several physics warps.
- Deleting a parameter that is still used asks to unbind it everywhere,
  keeping each object's default shape, as one undo step.
- Exporting to a folder that already has a model asks to overwrite, then
  replaces exactly the files the old model listed; VTube Studio's settings
  and other files stay.
- Turning Physics off returns physics-driven parameters to rest, so they
  sit on their keys for editing.
- Parameter value boxes line up in one column.
- README: system requirements measured on 2 cores and integrated graphics.
- README: new Mesh Edit and rigging screenshots, with art by マパ\*MAPA
  ([pixiv](https://www.pixiv.net/users/99163365)), used with permission.
  Thank you!

## 0.5.4 beta (2026-10-13)

### Fixed

- **Clipped layers no longer flicker, and nothing hides behind a sibling
  it should cover.** Draw orders are now treated as whole numbers, as in
  Cubism. A keyform blend that landed on 500.00003 drew a mesh above every
  other mesh at 500 (an eye white covering its irises, say), and physics
  nudging it either side of 500 made clipped layers flicker frame to frame.

### Changed

- New README with screenshots of a rigged model, Mesh Edit, warp
  deformers, the Physics page, the timeline and the home screen.
- Ko-fi and Buy Me a Coffee links, and a Sponsor button on the repository.

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
