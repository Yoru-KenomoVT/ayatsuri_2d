# Ayatsuri2D, instructions for agents

A clean-room, open, pure-Rust alternative to Live2D Cubism: a real-time 2D
puppet runtime reached through a C ABI, an editor on top, full `.moc3`
round-trip. Built the way FilmCraft (`~/filmcraft`) is built, with the same
conventions: one workspace, small focused crates, native egui/eframe UI,
everything from the published spec up.

## Start here

1. `README.md`, then `docs/goals.md` and `docs/architecture.md`.
2. The format research lives in `../ayatsuri2d-research/`; the file to code
   against is `docs/16-parser-guide.md` (verified against 5 models across 3
   versions). Where a code comment and that guide disagree, the guide wins.

## Non-negotiables

- **Never crashes** (docs/goals.md). Model files are hostile input.
  No `unwrap`/`expect`/`panic!`/indexing in non-test code; the workspace lints
  deny them and tests are exempt via `clippy.toml`. Bounds-check before every
  dereference, read index fields signed (-1 means none), use checked or
  saturating math on anything file-derived.
- **Never lags** (docs/goals.md). Allocation at load, nothing per frame,
  benchmarks assert p99.9 once they exist.
- **Layering**, enforced by `cargo xtask layers`: `ayatsuri-core` and
  `ayatsuri-capi` never depend on `openmoc3`; nothing below `crates/editor`
  touches egui/eframe.
- **Clean-room.** No Live2D software installed, no binary disassembled, no
  Live2D licence accepted. Format knowledge comes only from
  `../ayatsuri2d-research` (published specs, permissively licensed community
  work, lawful model analysis). See `docs/provenance.md`.
- **Models are never committed.** Only fingerprints and manifests (see
  `.gitignore` and `../ayatsuri2d-research/docs/09-test-corpus.md`). The one
  committed-adjacent model is the CVE-2023-27566 exploit file in the research
  corpus, explicitly licensed for building readers.
- **Licensing is split on purpose**: `openmoc3` is MIT, `crates/capi` is
  Apache-2.0, the rest is GPL-3.0. Do not blend code across those boundaries.

## Quality gates

`cargo xtask ci` runs them all: `cargo fmt --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, `cargo xtask layers`. Green before every commit.

## Running

- Editor: `cargo run -p ayatsuri` (optionally with a `.moc3` or
  `.model3.json` path to open it at startup; drag and drop works too)
- Inspect a model: `cargo run -p openmoc3 --example verify -- model.moc3`
- Headless evaluate + p99.9 timing:
  `cargo run --release -p ayatsuri --example evaluate -- model.moc3`
- A real v4.02 test model lives at
  `~/ダウンロード/Archives/Allium企划/ariu/ariu.moc3` (never commit it).

## Verifying runtime correctness

The oracle is PurismCore itself: clone github.com/SakuraMotion/PurismCore,
compile a dumper from its sources (see the session notes in memory; the
pattern is src/tests/extract.c, plus verify.c), dump every drawable's
vertices, and diff against `cargo run -p ayatsuri --example dump`. Both the
default pose and the deterministic "pose" sweep must match every drawable
to 1e-3 with y negated (the reference flips y for GL at the end; ayatsuri
keeps model space y-up and flips in the view). As of 2026-10-11 ariu matches
exactly in both states, glue included.

## Where the runtime semantics came from

PurismCore (MIT, github.com/SakuraMotion/PurismCore), which the provenance
docs clear for reference use: key snap epsilons (10^-decimals), the 2^N
keyform combo builder (first binding axis fastest), topological deformer
evaluation, warp children in normalized grid UV, the rotation-deformer
direction probe, and the load-time UV v-flip. `crates/core/src/deform.rs`
ports the warp interpolation/extrapolation faithfully; do not "simplify" it
without a real-model comparison.

## Coordinate conventions (hard-won, do not re-derive)

Core output: screen position = canvas origin + xy * pixels_per_unit, no
axis flip. UVs address the texture top-down, no v flip. The reference
runtime flips y at the end of its update for GL; ayatsuri does not, so a
numeric comparison against it negates y. examples/render.rs is the
software-rasterizer ground truth when the viewport looks wrong.

## Known gaps (the user's roadmap, rough order)

Done since: soft alpha masking through the wgpu view (Cubism-style mask
buffer in R8, one pass per mask set; binary stencil was wrong for soft
cavity masks like mouths), gamma-space blending (Live2D and egui both blend
in gamma; linear made translucent shading too dark), premultiplied-alpha
mipmapped textures (kills white edge halos), 4x MSAA, stencil masking
through the wgpu view (crates/render/src/gpu.rs
+ crates/editor/src/gpu_view.rs), draw order groups (the reference's bucket
walk, verified identical on ariu), named workspaces, floating icon toolbar,
click-to-select with mesh wireframe and deformer overlays, settings with
theme/accent/performance/keybind reference, About with the logo.

The toolbar mirrors Cubism's modeling toolbar (texture atlas, manual/auto
mesh, warp/rotation creation, arrow/lasso/brush selection, deform path,
deform brush, glue, art path), floats or docks, and Tool Details is a
palette. The menu bar carries Cubism's File and Modeling menus for real:
Export Image, Model Statistics, Environment Settings, Keyboard Shortcuts,
Close/Exit, Deformer/Parameter/ArtMesh submenus, Show/Hide Selected
(Ctrl+3), Reset Parameters (Ctrl+1), Edit Level (wired to the toolbar
state), Physics Settings.

The editing core is real now (crates/core/src/edit.rs, all tested):
keyform-product resampling for key insert/delete and unbind, warp/rotation
deformer creation, move_vertex/move_vertices (all keyforms, so pose
survives), add_vertex (barycentric split), delete_vertex (glue reindexed),
remesh (new triangulation sampled off the old mesh in uv space, keyforms
carried over), four_corner_forms (corner = x extreme + y extreme - centre),
create/delete_parameter. Session has 16-deep model undo/redo.

Mesh edit mode: double-click a mesh, the Edit Mesh tool, ArtMesh menu or
Ctrl+E. Scrim + textured redraw on top, strong wireframe, draggable vertex
handles (screen drags go through the parent deformer's local jacobian via
Puppet::deformer_point and mesh_local_positions), live preview, one undo
step per drag. The Edit Mesh dialog picks move/add/erase modes, runs the
auto mesher (grid over texture alpha, spacing/margin in px), and Done/
Cancel (cancel restores the entry snapshot). The deform path tool lays
control points on the selected mesh; dragging one pulls nearby vertices
with a smoothstep falloff (radius = brush size) through per-vertex
jacobians, one undoable edit per drag, Backspace pops a point.

Physics is in (docs/physics.md): PhysicsRig in ayatsuri-format (rides in
.aom), physics3.json parsed in ayatsuri-import (published CubismSpecs
schema), damped pendulum chains in crates/core/src/physics.rs. The
integrator honours the published field semantics (mobility keeps momentum,
delay/acceleration scale the pull, authored offsets give rest directions);
angle outputs are RADIANS measured against the authored rest (that is
what real rigs' scales of 8..40 are authored around; degrees rail
instantly), X/Y outputs are displacement from the rest chain so a held
input holds a proportional output, and the self-feedback loops real rigs
use (ariu writes ParamAngleZ from a group that also reads it) decay
softly because the radian loop gain stays under one. Frames where the
60 fps fixed step does not tick still re-apply the last outputs, or the
raw app-written input leaks through for one frame and follow mode
flickers. Toolbar has Physics and
Follow (cursor drives the standard ParamAngle/Body/EyeBall set, smoothed);
Modeling > Physics Settings shows groups, Valid, live wind, reset.
examples/physics.rs is the headless smoke test.

The parameter palette is Cubism-grade: header key buttons, right-click
menus, combined 2D pads, and a footer that adds parameters (dialog with
name/id/range/folder), adds folders, and deletes unused parameters
(delete_parameter refuses while bindings or physics reference it). Rows
take label colors from the context menu; names, folders and colors persist
through the .aom DisplayInfo block. The deformers palette is a hierarchy
(deformers nested under parents, meshes under deformers) with its own eye
column (Puppet::set_deformer_visible). Properties (ex-Inspector) scrolls,
shows render order, and clipping is editable.

Settings: Adobe-style window with General/Appearance/View/Performance/
Keyboard. Keyboard is a real rebinding editor (click, press keys, saved in
prefs; defaults mirror Cubism). Performance has an animation FPS cap
(request_repaint_after) and a model MSAA pref handed to
GpuRenderer::with_samples at startup (1 draws straight into the resolve
target). Editor-side textures load with mipmaps (mesh edit and CPU
fallback draw egui-side). The home screen shows the full logo over an
accent wash plus recent-model thumbnail cards (crates/editor/src/thumb.rs,
CPU render cached under ~/.cache/ayatsuri2d/thumbs). apps/ayatsuri-launcher
is the Adobe-style splash: fixed-size undecorated card (tilers float it),
ember particles, hands off to the ayatsuri binary; the desktop entry
points at it.

Deformers edit on canvas now: a selected warp's grid points drag (blended
parent-space grid from Puppet::warp_local_grid, jacobian-inverted, one
undo per drag, live preview), a rotation's origin dot slides and its
handle tip turns (move_rotation_origin / rotate_rotation), and rotation
creation uses the clicked point as origin even under nested parents (the
click inverts through the local jacobian around the mesh centroid). The
physics dialog is an editor: group rail with add/remove, Input / Physical
model / Output tabs editing a working copy of the rig, Apply commits one
undo step and rebuilds the sim; wind and fps live at the top. File > New
opens the New Model dialog (name, author, canvas, optional standard
parameter set from the published list); File > Model Info edits
name/author, both saved in the .aom meta. Closing the window or the
session with unsaved changes stops at a Save / Don't Save / Cancel dialog
(Session.modified flips in edit/undo/redo, clears on save). Mesh edit
mode decodes the real texture lazily (Session::full_texture) because the
palette handles are 256px thumbs. apps/ayatsuri-launcher got the gradient
card, logo glow and ember field treatment.

Still open: brush selection and deform brush behaviour, glue binding UI,
art path, texture atlas view, canvas flip, blend shapes, multiply/screen
colors (slots 105-113), timeline with a real keyframe UI, part thumbnails
in the Parts palette, physics group editing (pendulum/input/output rows),
.aom-embedded physics editing, moc3 export round-trip, the C ABI. UI
rules from the user: dark mode default, accent is the logo orange #F08E1E
(user-overridable), Cubism-style layout, Adobe-ish home, context menus
everywhere, natural lightly-formatted comments. Credit wording: the format
research is the Alterion project's own, cross-checked against PurismCore
and OpenL2D.

Cross-platform: everything is pure Rust on eframe/wgpu (Vulkan on Linux,
Metal on macOS, D3D12 on Windows), so it builds and behaves the same on all
three; the one platform-specific bit is the CJK font lookup in
crates/editor/src/theme.rs, which needs macOS and Windows font paths added
before shipping there.
