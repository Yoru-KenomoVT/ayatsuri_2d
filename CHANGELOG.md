# Changelog

## 0.6.0 (2026-10-29)

Blend shapes, the feature promised for the end of October: on every kind
of object, with weight limits, in moc3 from 4.2 on. Alongside them come
multiply and screen colours, offscreen parts, a run of Cubism tools for
meshes and the Parts palette, and the requests from Discord testing.

### Added

- **Blend shapes** (#23), as in Cubism 5.0: mark a parameter as a blend
  shape and its keys add a difference on top of everything else, instead
  of blending with the other parameters. They work on art meshes, warp and
  rotation deformers and parts (glue keeps the ones it is imported with),
  and change shape, opacity, multiply
  and screen colour, angle, origin, scale and draw order. Add Keys, the
  grid tools, the rotation handle and the opacity and colour rows edit the
  key you sit on, and the Inspector says when a blend shape key is the one
  being edited.
- **Weight limits for blend shapes** (#24): Parameter palette menu > Limit
  Settings for Blend Shape Weights. Another parameter scales a blend
  shape's weight along a graph, from a preset or drawn by Edit Graph, on
  meshes, deformers and parts.
- **Multiply and screen colour per keyform** (#26), as in Cubism 4.2: tint
  meshes and deformers per key in the Inspector. They go into moc3 4.2 and
  later and into .cmo3, and come back from both.
- **Offscreen drawing on parts** (#25), Cubism 5.3's way: a part draws its
  children as one layer, with its own opacity, blend mode and clipping, so
  overlapping children show no seams. PSD folders that blend as a whole
  (not pass-through, not plain Normal at 100%) import as offscreen parts.
  moc3 5.3 stores them, blend shapes on their opacity included; older
  versions say what they drop.
- **moc3 export of all of it**: blend shapes on meshes and warps from 4.2,
  on rotations, parts and glue from 5.0, offscreen parts from 5.3. The
  layout was checked against an open reference core, which reads the files
  back and plays them as we do.
- **Clip by in one row**: type part of a mesh's name or ID and pick it from
  the list, paste IDs (`A, B, C`) and press Enter, or pick masks on the
  canvas. The masks show as chips under the field. **Copy ID** on a mesh
  (right-click, or the button beside its ID in the Inspector) and the copy
  button by Clip by put IDs on the clipboard for another mesh, and a mesh
  copied with Ctrl+C pastes into Clip by as its ID.
- **Conversion division in Tool Details**: select a warp, type the new
  columns and rows and press Apply. The Inspector shows it too, and a
  warp's blend shapes now follow a division change.
- **Model Template** (Modeling > Model Template): Live2D's Standard
  Parameter List, 27 or all 40, filed into the standard groups with the
  IDs face trackers and VTube Studio expect. Save your own model's
  parameters, ranges and folders as a template for the next model.
- **Parts palette as in Cubism**: drag parts and meshes to reorder and
  regroup them, New Part, and Group into New Part on the context menu.
- **Rename parts and art meshes and edit their IDs**, from the Parts and
  Deformer palettes. F2 renames the selected part, deformer or art mesh,
  or opens Edit Parameter on the selected parameter.
- **Automatic mesh generator settings and presets** as in Cubism. Auto
  mesh starts from the layer's art and runs a row of points down thin
  strokes instead of boxing them in.
- **Drawn lines guide Auto connect**, and Add vertex chains as in Cubism.
- **View > Solo**, **Highlight Deformer's Child Elements** and **Highlight
  Overlapping Polygons**, as in Cubism.
- **Custom texture atlas page sizes**, up to what your GPU takes.
- **Verify placement** in the Texture Atlas, as in Cubism: pieces that
  overlap another or reach past the page show in red, with a count, and
  the button selects them one by one.
- **Texture Atlas View** (T, or View menu), as in Cubism: the canvas shows
  every pixel each mesh covers on the atlas, as the export will, so a
  mesh grown over its neighbour shows it.
- **A shortcut for every menu action** in Settings > Keyboard: tools such
  as the Deform Brush, Extended Interpolation and the other parameter
  commands, the File, View, Window and Help items, and more. New keys:
  F2 renames the selected object, Shift+F2 the selected deformer (or the
  selected mesh's), D picks the Deform Brush, Ctrl+Shift+I opens Extended
  Interpolation, Ctrl+Alt+C copies the selected meshes' IDs, Ctrl+, opens
  Settings and Ctrl+Q exits; Ctrl+Shift+N (New Animation) and Ctrl+T
  (Texture Atlas) are Cubism's. The rest have no key until you pick one;
  menus and toolbar tips show the keys.
- **Shift holds a Deform Brush stroke** to the X or Y axis, whichever it
  moves along first.
- **Experimental features** (Settings > General, at the bottom): one
  switch for all of them and one per feature. In this release: clip
  groups, inherited clipping (exported to moc3 as hidden helper masks) and
  stroke mesh-mapping, whose 1, 2 and 3 vertices across lay the mesh out
  as Cubism's do.
- **Delete parts** in the Parts palette, as in Cubism: Delete or
  Backspace (or Delete in the right-click menu) removes the selected parts
  with everything inside them. **Delete Part (Child objects will remain)**
  removes only the parts and moves their contents up to the parent part.
  Undo brings the parts back with their names.

### Changed

- **The texture atlas waits for export**: leaving Mesh Edit no longer lays
  the atlas out again by itself, so rigging and animating never rewrite
  the textures. When a mesh has grown past its art, the moc3 export says
  so and offers **Lay out the atlas**; Settings can turn the old automatic
  layout back on.
- **Opening a PSD, Krita or Clip Studio file no longer packs a texture
  atlas**, as in Cubism: each layer keeps a texture of its own, saved in
  the .aom, so the paint file isn't needed afterwards. Make the atlas in
  Modeling > Texture Atlas when you're ready; until then the Texture
  panel shows no pages, and it and the moc3 export offer to make it.
- **Delete edge keeps the other lines**: cutting one side of a triangle
  leaves its other two sides as lines instead of removing them too.
- **Parameters drag like layers**, into and out of folders.
- **Clicking another mesh inside the selected one's box** switches to it
  instead of grabbing the box.
- **Blend shape keys** draw as rectangles, with a border on the base key,
  as in Cubism.
- **Every dialog's close button** sits in its top right corner, however
  wide the dialog comes out.

### Fixed

- **Pasted copies clip to each other**: copy an eye's white and iris
  together and the pasted iris clips to the pasted white, not the old one.
  Such a copy also leaves the original's clip group, so it can be clipped
  by other meshes without changing the original. A pasted copy takes a new
  `ArtMeshN` ID instead of `<id>_copy`.
- **Clip by took only mesh IDs**: a typed name (`eye`, or one with spaces
  like `Highlight L T1`) now picks that mesh when the field is left, not
  only when Enter is pressed.
- **Dragging felt wrong with Pen mode on**, pen and mouse alike: it let a
  press drift 14 px before a drag began, so every drag started late. In
  Mesh Edit the point was then missed, or trailed the pointer and its
  texture shifted. Pen mode no longer changes when a drag starts, and
  Mesh Edit grabs the point where the press began, and shows a closed
  hand while a point is held.
- **Mesh Edit showed the edited mesh twice**: its frozen art above the
  dark backdrop and the live mesh under it, which changed with every
  edit. Only the frozen art shows now; meshes clipped to it still follow
  its new shape.
- **Shift+click in the Deformer palette** only reached rows of its own
  kind: from a mesh to a deformer it picked just the deformer. It now
  picks every row of the clicked kind between the two.
- **Temporary Deform Tool**: Perspective has the side handles too, as in
  Cubism, sliding that side along itself; handles are bigger squares,
  grab where the press began, and show a hand cursor.
- **Perspective and Distort move the art as Cubism's do**: evenly, with
  the box's middle at the average of its corners, instead of toward a
  vanishing point. A Perspective corner keeps to the side the drag mostly
  goes along again, its partner mirroring it, and the anchor rides with
  the box.
- **The Solo chip's close mark** showed as an empty box.
- **Undo ended the Temporary Deform Tool.** It stays on now, with a
  fresh box around the art where the undo left it; changing parameters
  still ends it.
- **Mirror Edit did nothing in Mesh Edit.** Dragging a point now moves
  its mirror across the axis the other way (live, one undo step), and
  deleting a point deletes its mirror.
- **Stroke mesh-mapping**: a new line that would redraw a stroke you have
  not confirmed asks first (Replace, Keep both, or Discard the new line),
  so a slip no longer throws it away. Grabbing a control point takes the
  one where the press began. With Maintain current mesh off, the points
  and lines that confirming removes are marked while you draw.
- **Brush circles** (Mesh Edit's eraser, Deform Brush, weight painting)
  could be hidden by the art under them; they now draw on top, with a
  dark edge.
- **Undo after making the texture atlas or re-importing art** put meshes
  back on texture pages that were already gone, so they showed the wrong
  art. Old pages now stay until the model is saved.
- **Texture atlas**: a mesh could show a sliver of its neighbour's art at
  its edge. Each piece is now masked to its own art, with wider gaps.
- **Zooming at the limit** no longer flings the canvas away.
- **Auto connect** keeps every face that holds art and peels slivers off
  concave edges.
- **PSD import**: layer names some programs end with a hidden character
  come through clean, and sibling folders with the same name stay apart
  instead of merging into one part.
- **Inspector notes** wrap inside the panel instead of running past it.
- **A rotation deformer made inside a warp** no longer blows up the art it
  holds: what it holds is carried over through the canvas, so it stays
  where it was, and a rotation deformer wrapped by a new deformer keeps
  its turn.
- **Grabbing a rotation deformer's centre or handle** works with Pen mode
  on (for the mouse too): what a drag grabs is where the press began.
- **After making a deformer** the canvas goes back to the Arrow, so the
  next click edits it instead of opening New Deformer again.
- **Timelapses recorded before 0.6.0 play and export again**: a field
  added to the middle of the model shifted what older steps held, so they
  could not be read ("step 0 could not be read"). They are put right as
  they are read, and new steps are written in a way a later field can
  never shift.
- **After Undo** a selected object the undo took away is let go, so the
  next action never works on one that is gone.
- **Several deformers can be selected at once**, as in Cubism. In the
  Deformer palette Shift+click picks the run between two clicks and
  Ctrl+click adds or removes one; on the canvas Shift+Alt+click adds the
  deformer of the mesh under the pointer. Only top-level ones join: a
  deformer inside another picked one is left out, so a rigged child taken
  in by mistake is never moved or keyed with the group. The pick moves,
  scales and rotates in one box (the art follows live), takes keys
  together, and deletes, locks, hides and re-parents as a group; a new
  deformer made as their parent wraps all of them.
- **The Deformer palette drags like the Parts palette**: drop on the top
  or bottom edge of a row (a line shows) to move beside it, out of a
  deformer and into its parent's level; drop on the middle of a deformer
  (a box shows) to move into it.
- **Rotation deformers take Cubism's keys**: dragging the centre moves the
  deformer with its art (Ctrl+drag moves only the pivot), Ctrl+drag on the
  handle turns the deformer while its art stays, Shift turns in 45 degree
  steps, and Alt+drag makes only the handle longer. The round ring
  around the centre is the scale handle: drag it to enlarge the deformer
  with its art, Ctrl+drag to enlarge the deformer alone.
- **Ctrl+drag moves a warp without what it holds**, as in Cubism: the box
  or its points move, scale or rotate while the art and deformers inside
  stay where they are, to fit a warp after it was made. Works on several
  picked deformers too (a rotation's centre then moves as its pivot).
- **Flip for deformers** (Modeling > Edit Form, or Flip on a deformer's
  right-click menu): rotation deformers flip too, turning the other way and
  mirroring through their Reflect flag, and rotation deformers nested inside
  a flipped deformer stay a true mirror. Several picked objects flip
  together across the middle of the pick; art meshes have the same Flip
  on their right-click menu. Flip Warp Only mirrors a warp's
  grid while what it holds stays put. All have shortcut entries.
- **More on the canvas right-click menu**: on an art mesh, Edit Mesh,
  Rename, Clip by, Copy ID, Create Warp or Rotation Deformer, Select Parent
  Deformer, Flip, Hide, Lock and Delete; with a deformer selected, Rename,
  Flip, Conversion Divisions and Delete; in Mesh Edit, Merge Vertices,
  Delete Points, Auto Connect, Divide into 4, Reset Mesh, Flip and Finish.
- **Merge Vertices** in Mesh Edit, as in Cubism: pick two or more points
  (the ends of two strokes, say) and they become one, at their middle,
  with their edges. Also on a shortcut entry.
- **The canvas right-click menu shows the deformers above** the mesh under
  the pointer as a tree to select from (Shift+click adds to the pick), as
  in Cubism; Settings > Canvas can turn it off.
- **Pen tablet support** (Settings > Appearance > Pen tablet): pen pressure
  can size and weight the Deform Brush, Brush Selection, the glue weight
  brush and the mesh eraser, through a pressure curve and a lightest-touch
  floor, smoothed against jitter, with a meter to test the pen. Pen mode
  gives a pen's double tap longer. Pressure works on Windows and Linux and
  is in for macOS but untested there. On Wayland the pen comes through the
  tablet protocol, as in Krita, so it clicks and drags even where the
  compositor's own fallback drops a pen's clicks (Hyprland 0.56); on X11
  the tablet is read directly (Settings explains access if needed).
- **Boxes over several meshes** move the art live while dragged, like a
  single mesh, instead of only on release.
- **Clip by on several selected meshes** clips all of them, not only one,
  from the canvas pick and the Inspector field alike, as one undo step.

## 0.5.8 (2026-10-24)

The rest of @seoriboi's requests from #29, their round of testing on
Discord, a window for updates, and a few tools brought closer to how
Cubism does them.

### Added

- **Change a warp's conversion divisions after it is made** (#34):
  right-click a warp deformer and choose Conversion Divisions… Every
  keyform's grid is sampled again at the new columns and rows, so what the
  warp holds keeps its shape: exactly when the division gets finer, at the
  points that remain when it gets coarser. Its children need no change.
- **Smooth movement** (#36): dragged points and objects ease after the
  pointer instead of jumping with it, and land where you let go. Turn it
  on in View > Smooth movement; Settings > Canvas sets how much.
- **What's new**: the first start of a new release shows its part of this
  changelog, once (test builds of it don't bring it back). Help > What's New
  and the What's New button in About open it again.
- **Join the Discord**, in Help, About and What's new.
- **A list of the meshes under the cursor**: Ctrl+right-click the canvas,
  as in Cubism, or right-click > Select Layer. Each row shows the mesh's
  own art, big on hover, and the hovered mesh is outlined on the canvas.
- **Drag meshes from the Part palette onto a deformer** in the Deformer
  palette to put them under it; dragging one of several picked meshes
  moves them all. They stay where they are on screen.
- **Batch Reflect…** in the Parameter palette's menu: Reflect on every
  object keyed on the ticked parameters.
- **Motion Mirroring has a shortcut slot** in Settings > Keyboard (no key
  until one is set).

### Changed

- **Updates open a window** instead of the banner on the Home screen. It
  comes up once per launch, wherever you are (never over another dialog
  or in the middle of a drag), with the release's notes and Update now,
  Later (asks again next launch) and Skip this version. Help > Check for
  Updates asks GitHub again.
- **Face standard automesh** now lays a face out the way a hand-built one
  is: the outline about a quarter spacing outside the art, a second ring
  just inside the drawn edge set between the outline points, so the edge
  sits in a thin zigzag strip when a warp turns the head, and an even
  inside at twice the outline spacing instead of two hubs.
- **ffmpeg downloads go to `%LOCALAPPDATA%`** (`~/.local/share` on Linux
  and macOS) instead of the roaming settings folder: 165 MB has no
  business roaming with a Windows profile. A copy an older version put in
  the settings folder is still found.
- **The ffmpeg offer** uses the same window as Settings and About, with a
  close button that hides while ffmpeg downloads.
- **The Warp and Rotation Deformer buttons open the creation dialog** at
  once, as Cubism's do. It follows the selection while it is open, so
  picking what goes inside can come before or after.
- **A click on the canvas picks the mesh whose art is drawn there.**
  Transparent margins, hidden parts and clipped-away areas no longer catch
  it, and a second click on the same spot picks the mesh underneath.
- **Edit Mesh keeps to the selected mesh** when it is under the click, and
  a click where nothing is drawn says to click an art mesh instead of
  opening a mesh you can't see.
- **The Temporary Deform Tool pauses physics** while it is bound (physics
  kept the parameters between keys, so the tool would not start) and says
  in a window why it can't start.
- No more "beta" on releases.

### Fixed

- **Keyboard settings**: a shortcut waiting for its new key stopped every
  other shortcut until a key was pressed, even after Settings was closed.
  It now stops waiting when Settings closes or another page opens.
- **Shortcuts with modifiers** (Ctrl+Shift+S): pressing Ctrl or Shift no
  longer ended the capture. It waits for the key that completes the
  combination and shows the modifiers held so far. Ctrl+C, X and V can be
  bound, and a shifted digit binds as the digit.
- **Reflect** does what Cubism's does: the key values turn around across
  the parameter's range on the selected objects, so a mouth keyed open at
  0 and shut at 1 becomes shut at 0 and open at 1.
- **Reset parameters (Ctrl+1)** left meshes in the old pose while the warp
  lattice showed the new one.
- **Perspective** follows a diagonal drag on both axes instead of keeping
  only the larger one.
- **Edit Mesh's Done** waits for at least three points and a triangle and
  says why; a delete that would leave fewer than three points says so.

## 0.5.7 (2026-10-23)

Fixes and requests from @seoriboi's feedback in #29.

### Added

- **Deformer names apart from their IDs** (#35), as in Cubism. Rename
  Deformer (right-click a deformer) edits both after it is made: the name
  is free text shown in the palette, the ID is what exports and the
  runtime use and must be unique. The inspector lists both. Names are
  saved in `.aom` (older files show the IDs, as before) and come in and
  go out with `.cmo3` projects.

### Fixed

- **Settings are kept on Windows** (#31). The editor looked for its
  settings folder only through `HOME` or the XDG folders, which Windows
  does not set, so nothing set in Settings (or anywhere else that is
  remembered: workspaces, recent files, export folders) survived a
  restart, and no log file was written. Settings now live in
  `%APPDATA%\ayatsuri2d`, the log and thumbnails in
  `%LOCALAPPDATA%\ayatsuri2d`. Linux and macOS keep their folders.
- **Picking a parameter keeps the selected deformer** (#32), mesh or
  part, so keys can be made on one parameter after another without
  selecting the object again. Delete leaves a selected mesh alone until
  the next click outside the parameter list.

### Changed

- **Double-click a row in the Parts or Deformer palette to deselect**
  (#30): nothing stays selected, so no key is made on an object by
  mistake. Selected rows now have an accent bar down their left edge,
  one row or many.
- **Shift keeps a Bezier point or handle level or upright** while it is
  dragged (#33): in the graph editor, on ArtPaths and on a warp
  deformer's Bezier lattice.

## 0.5.6 (2026-10-23)

### Added

- **Cubism Editor projects** (`.cmo3`) open as whole models: art, meshes,
  parts, the deformer tree, parameters in their folders, every keyform,
  clipping, blend modes and physics, with the names the rigger gave things. Built from
  the project files themselves; checked against a project rigged in
  Cubism, deformer for deformer.
- **The C ABI** (`crates/capi`, Apache-2.0): the runtime for anything that
  calls C, with `ayatsuri.h` generated from the Rust source.
  - Loads an `.aom`, or a `.moc3` with the bundle's physics, motion and
    expression JSON files added as bytes. The paint and project parsers are
    left out of the library.
  - Parameters and part opacities go in as arrays by index;
    `ay2d_update` plays motions (scenes and compositions, with crossfades,
    loops, seek, speed and polled events) and expressions, steps physics
    and publishes the result.
  - `ay2d_drawables` hands back every visible mesh back to front as views
    into that result (positions, uvs, triangles, masks, blend flags);
    `ay2d_mesh_geometry` serves masks and GPU hosts, `ay2d_mesh_changes`
    says which meshes moved.
  - Display names, the keys each parameter is rigged at, parents, folders
    and the EyeBlink and LipSync groups for a UI.
  - Values are set and results read from any thread; one thread drives
    the model and a second gets `BUSY`; a render thread draws frame N
    while N + 1 is made.
  - Every call returns a status and a failing one changes nothing; panics
    never cross the boundary. `cargo xtask capi` builds the shared and
    static libraries with symbols stripped, writes the header and runs a
    C host against the result.
- **Cubism Editor project export** (File > Export Model > cmo3): the model
  goes out as a `.cmo3` the way the Editor saves one. Each mesh's art is
  cut from the texture back into a layer of one layered image and packed
  into fresh atlases the uvs point into; parts, the deformer tree,
  parameters in their folders, every keyform in its parent's space,
  clipping and physics follow. Animations and sound are not part of a
  project and stay with the motion export. Before a project is written it
  passes a strict reading: every reference resolves to an object of the
  right class, every list and array holds what its count says, and each
  object has the fields, value types and attributes a project saved by
  the Editor gives it. Written from project files alone and checked by
  reading the result back.
- **Clip Studio Paint files** (`.clip`) open, import and re-import the way
  PSD and Krita files do: folders become parts, every drawn layer becomes
  a mesh with its mask applied, paper and adjustment layers are left out.
  Vector, fill, gradient and text layers come in as Clip Studio draws
  them. The reader was built from the files themselves and treats them as
  hostile input, like every other format.
- **Mirror Edit** (View > Mirror Edit, Ctrl+Shift+M): moving points in
  Mesh Edit, on a warp grid or on selected meshes also moves their
  mirror across the vertical axis, flipped. The axis is the vertical
  guide line nearest the centre, else the canvas centre, and shows as a
  dashed line while the mode is on.
- **Texture Atlas** (Modeling > Texture Atlas…, or Edit atlas… in the
  Texture palette): the pages laid out with every mesh's piece drawn from
  its own pixels. Drag pieces around a page or onto another page's tab,
  add and delete pages, pick a page size from 1024 to 8192, and **Auto
  layout** packs everything onto as few pages as fit (pieces larger than
  a page scale down when asked). Apply rewrites the pixels and every uv
  in one undo step; the model draws exactly as before.
- **Texture memory stays bounded.** A model's texture pages no longer go
  to the GPU whole at load: each page keeps a small proxy there, and the
  full page streams in from its PNG on a worker thread when a visible
  mesh draws it at a zoom the proxy can't carry, then leaves again after
  five seconds unused, after ten seconds idle or minimised, or when the
  budget (Settings > Performance > Texture memory, 512 MB by default) is
  full. A model with more texture than fits still opens; the pages past
  the budget draw from their proxies. Load keeps one decoded page in RAM
  at a time instead of all of them. Model Statistics shows what is
  resident. `cargo xtask soak <model.aom> [seconds]` runs the evaluation,
  undo, decode and physics paths headless and fails if memory grows.
- **The Timeline is a dope sheet in the After Effects layout.** A column
  of names on the left: the scene, the parameter folders with their
  parameters (every one, or only the animated ones), and the parts with
  an opacity curve. Each row has a value that drags or types, and
  previous key, key here, next key buttons; the key button fills when the
  playhead sits on a key and adds or removes one. Keys in the lanes are
  drawn by interpolation (a diamond for auto smooth and
  linear, a square for a step, an hourglass for a bezier); they select,
  Shift-select, box select, drag by whole frames with Shift snapping to
  other keys and the playhead, Ctrl-click to flip linear and auto, and
  right-click for the interpolation, select the row or delete. A folder
  row sums up its children's keys and drags them together. A frame
  ruler scrubs, Alt with the wheel zooms at the pointer (a slider in the
  footer too), and the work area bar under the ruler bounds playback:
  drag its handles, or press Shift+B and Shift+N to put its ends at the
  playhead.
  The scene row's bar drags to change the length. S keys the selected
  parameter, Alt with an arrow nudges the selected keys a frame (ten
  with Shift), Delete removes them and Ctrl+Alt+A takes them all; every
  Timeline key sits in Settings > Keyboard, under Animation, and can be
  rebound, and none of the defaults clashes with an editor binding. `Animation > Edit Animation Details…` edits the name,
  frame rate, length in seconds and loop, and inserts or removes frames
  at the playhead. Space (a tap, not a pan) or Enter plays and pauses.
  Key edits have
  their own undo, one step per drag, ahead of the model's history while
  the Animation workspace is up. The chart button opens a **graph
  editor** in place of the lanes: the value curves of the selected keys'
  parameters (else the selected parameter, else every animated curve)
  over a fitted axis with a grid, square keys that drag in time and
  value, handles on every selected key (a drag turns the segment into a
  bezier; the other handle swings with it unless Alt is held), and pills
  for the interpolation. Auto smooth
  keys draw as diamonds, like Live2D; a hold is a square and a free
  bezier an hourglass. The top row folds everything under it.
- **Compositions.** `Animation > New Composition…` makes a timeline of
  layers, the way After Effects nests comps: each layer plays a stretch
  of a scene (or another composition) from a start time, at a speed,
  faded in and out, and mixes over the layers beneath it by its weight.
  The Timeline shows a composition as bars: slide one, trim either end
  (Shift snaps to other layers and the playhead), drag the square at a
  top corner inward for a fade in or out, right-click it for fades,
  speed, split at the playhead, order, duplicate or delete, and
  double-click it to open its scene (a button leads back), and drag its
  name up or down the list to reorder the stack. `[` and `]`
  put a layer's start or end at the playhead, `Shift+D` doubles it,
  `Ctrl+Shift+D` cuts it there. A layer twirls open to show its scene's
  keys where they land. Play, the Render window and the model export
  all take compositions: export bakes one into a motion with a key on
  every frame. Compositions ride in the `.aom`; older builds ignore them.
  An opened layer's keys edit in place: select, drag, key and jump on
  them as in a scene, and the edit lands in the scene through the
  layer's start, in point and speed. Insert and Remove frame cut a layer
  that spans the playhead and move its tail, as a ripple should.
- **Sound in compositions.** Add layer (or `File > Import Audio…`) takes
  a WAV, MP3, FLAC or Ogg file. It draws its waveform across its bar, plays through the system
  output while the composition plays (from the playhead, stopped with
  it), and the Render window mixes every sound layer down and hands it to
  ffmpeg beside the frames, so MP4, WebM and MOV come out with sound. The
  file rides by path, relative to the `.aom`. On Linux the sound goes to
  PipeWire (or PulseAudio) over its own protocol, in pure Rust, so
  Bluetooth and virtual outputs show up and an underrun does not kill the
  stream; cpal is the fallback and the path on Windows and macOS.
  **Settings > Audio** picks the output device, the volume and the buffer
  size, with a test tone. The **Audio Mixer** palette is laid out like a
  video editor's: per sound layer a dB scale, a fader (to +6 dB), live
  level meters (two bars for stereo, with a held peak), the level read
  out, mute, solo and a pan knob, and the name in a box; a gutter drags
  a strip down to just its meters; a main-out bus shows the editor's
  level and the device it goes to. Level, mute, solo, pan and fade
  changes reach the sound playing in place, eased over a few
  milliseconds, so nothing clicks. A stereo file twirls open on the
  timeline into its left and right channels. Layer rows are taller, a bar's name sits on a fade
  of the layer's colour so it reads over a waveform, and a square at
  either top corner drags inward for a fade in or out.
- **Expression palette** (Window > Expression; a tab beside the Timeline
  in the Animation workspace): the model's exp3 expressions, listed
  across the palette's full width. The + makes an empty one; Add
  parameter opens a list to filter and pick from, with a slider and a
  value box, and Set from pose fills one from every parameter moved off
  its default. Each parameter row drags its value with Add, Multiply or
  Overwrite, fades are optional, and the model shows the expression as
  its values change.
- **Skinning** (Modeling > Skinning > Use Deform Path for Skinning), as
  in Cubism: draw a deform path on a long mesh from root to tip and it
  becomes a chain of rotation deformers, one per path point but the tip,
  each the child of the last. The mesh is cut into a piece per deformer
  midway between them, the pieces are glued at the cuts so they bend as
  one, and every deformer gets a parameter (-45 to 45) in a folder named
  after the mesh. The original mesh is hidden and kept. It is all plain
  deformers, glue and parameters, so it exports to moc3 for any version.
- **Skinning onto rotation deformers you placed** (Modeling > Skinning >
  Skinning…) and **Generate Parameters for Rotation Deformers…**, Cubism's
  other way in: tick the rotation deformers along a mesh, root first, and
  the mesh is cut midway between them and glued, each piece under its
  deformer; then give each deformer an angle parameter (-30 to 30 by
  default) in a Rotation folder.
- **Temporary Deform Tool** (Modeling > Temporary Deform Tool), as in
  Cubism: throwaway control points around the selected meshes or warp
  deformer, gone once the tool is released. Enlarge/Reduce, Rotation, Free
  Transform, Perspective and Distort put a box around the art, Temporary
  Warp Deformation a grid (columns and rows in Tool Details), Temporary
  Path Deformation a path you click out. Drag a control point to deform,
  Ctrl+drag to move the point only, Shift+drag to move straight; the
  anchor is the centre for Rotation and Enlarge/Reduce. Each drag lands on
  the current keyform as one undo step; the green check in the view, Unbind
  or Esc lets go (#14).
- **Blend modes from Cubism 5.3**: a mesh's colour blend can be Add,
  Add (Glow), Darken, Multiply, Color Burn, Linear Burn, Lighten, Screen,
  Color Dodge, Overlay, Soft Light, Hard Light, Linear Light, Hue or
  Color, besides Normal and the two older Add and Multiply, and its alpha
  blend Over, Atop, Out, Conjoint or Disjoint (Properties > Blend mode,
  Alpha blend). The canvas, Render exports and thumbnails draw them; PSD,
  Krita and Clip Studio layers and Cubism projects bring theirs in; moc3
  5.3 keeps them, and an export to an older version draws each with the
  nearest of Normal, Add and Multiply and says so. The C ABI reports them
  through `ay2d_mesh_blend`.
- **Timeline context menus**: right-click a row name. The scene row edits
  the details, adds a parameter track, keys the selected parameter,
  selects every key, sets the work area, inserts or removes a frame and
  makes scenes and compositions; folders fold, key, select or delete
  their keys; parameter and part rows key, remove, step, select,
  re-interpolate, delete and reset; a composition's rows add scene and
  composition layers and import audio.
- **Website** button in About, beside GitHub, opens ayatsuri-2d.org.
- **Physics playlists** (Physics Settings > Playlist): groups of scenes
  played top to bottom with a crossfade, a group's scenes at once, each
  overwriting or adding at a weight, so the physics can be watched
  against real motion. The bar picks, plays, loops and sets the fade, the
  Playlist menu makes and deletes them, and they ride in the `.aom`.
- **Export Animation** and **Export Timelapse** open the new **Render**
  window: formats down the left (GIF, PNG sequence, MP4, WebM, MOV),
  settings in the middle (scene, an expression on top, work area, frame
  rate, width, transparent background, folder), a preview and a render
  queue on the right. Frames render on a worker thread; GIF and PNG are
  built in, the video formats stream to ffmpeg when it is on PATH and the
  window says so when it isn't. Physics runs during an animation render
  when it is on in the editor. The preview is a player: a transport bar
  with start, play, end, a scrubber, timecode, loop and a speed for the
  timelapse, and a full-screen view; it sizes to the window, frames come
  from a worker thread so playback stays smooth, and Space and the arrow
  keys work over it. Beside it an
  animation lists the parameters the scene drives with their live values,
  and a timelapse lists what each step changed (meshes, deformers,
  parameters, parts, physics) and draws it over the frame: edited meshes
  in orange, new ones in green, warp grids and rotation pivots. A timelapse
  plays as the edits were made: between steps the model is blended, so a
  dragged point eases from where it was to where it went and a new mesh
  fades in, at a chosen number of steps per second (6 by default) in both
  the player and the export. Each step is posed at the parameters the edit
  was made on, so an eye moved at X 1, Y 1 is seen moving there, and the
  panel says where. Steps that were undone are left out, the undo with
  them; a redo plays once. Frames are drawn at twice the size and averaged
  down, in bands across every core, so edges are smooth in the video and
  in the paused preview and a tall canvas renders in a fraction of the
  time; art is sampled bilinearly, so it is never blocky. The preview
  draws with the full art once the pages are decoded. In full screen the
  player and its panel sit together in the middle with a wider panel. A
  render that fails says why, with ffmpeg's own words for a video, and
  every outcome goes to the log. Frames are drawn by the GPU (**Draw
  with**): the viewport's renderer draws each one off screen with MSAA
  and the pixels are read back, so the processor only encodes, with
  ffmpeg held to half the cores. Without a usable GPU, or with CPU
  chosen, frames are drawn on the processor, with **Smooth edges** to
  turn the supersampling off when that is too slow. The queue is a list
  of cards: name and format, a bar with frames done and the time left,
  a stop button while a job runs, how it ended, and a button to open the
  folder; it keeps going with the window closed and is there when it
  opens again. With the eye on, a timelapse video carries the change
  marks the player shows, drawn into each frame. The window remembers
  its folder, format and settings between sessions, as the moc3 and
  image exports remember their folder, and every file dialog opens
  there.
- **Export Model** replaces Export Image and Export as moc3: one window
  with moc3, cmo3 and Image tabs. Image exports PNG or JPEG at
  any width, transparent or on white, of the model as it stands.
- **Project** replaces Model Info, and New Model is New Project. Both
  carry a **Record timelapse** switch saved with the project; Settings
  keeps the default for new ones.
- **Timelapse**: every step of building a model (each edit, undo and redo)
  is kept in a small `.timelapse` file beside it, as compressed changes.
  Window > Timelapse plays it back with a scrubber and exports PNG frames
  and a GIF, plus an MP4 when ffmpeg is installed. Turn it off in
  Settings > General. Each step names the state it was recorded on, so
  two copies of the app writing one project keep their own chains and a
  file opened again after a save carries on from where it was; the
  timelapse plays the chain the file ends on.
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
- **Key dots tell keys apart**: keys mirrored across the default share a
  colour (the default key is white), so Motion Mirroring's pairs match up.
  Their outline shows the selection, as in Cubism: green when every
  selected object has the key, yellow when only some do, a red border on
  the key the slider sits on, and a red glow on the key Motion Mirroring
  just wrote to. **Right-click near a key** and the slider sticks to its
  exact value.
- **Canvas colour** (Settings > Appearance > Canvas): the page under the
  art can be any colour, with grey quick picks for long sessions.
- **Shortcuts for the Temporary Deform Tool** in Settings > Keyboard: one
  per method and one to deselect it, not set until you pick keys, as in
  Cubism. The Built in list now has right-click to put a slider on a key.
- **Extended interpolation** (Parameter palette > Extended Interpolation…,
  or Modeling > Parameter), as in Cubism: keyforms blend along a curve
  instead of a straight line, so a sway made of turned keys keeps its size.
  Ellipse (with Scale) or SNS, on the selected objects, with three keys or
  more. A moc3 or cmo3 export adds Points keys between each pair so other
  apps follow the curve; keys added on a curved parameter take the curve's
  shape (#21).
- **Colour picker** for every label colour (parameters, folders, keys): a
  palette of presets, and a colour-wheel tile that opens the same colour
  picker as Settings, in a small window. A folder's colour now fills its
  whole header, as in Cubism; parameters keep the thin stripe, with their
  names set clear of it.
- **Key colours**: right-click the key the slider sits on for Key color, to
  tell keys apart; saved with the model (#19).
- **Shift+drag moves straight**, horizontally, vertically or at 45°, for
  mesh points, warp points and boxes, and the Temporary Deform Tool's
  points (#15).
- **ffmpeg installs itself**: MP4, WebM and MOV no longer need ffmpeg on
  PATH. When none is found, the editor asks at start and the Render window
  offers **Install ffmpeg**; the Windows installer and the Linux
  `install.sh` offer it too. It comes from its builders (BtbN's GitHub
  builds on Windows, martin-riedl.de on Linux and macOS), is checked
  against their published checksum, and stays with your settings: no admin
  rights, nothing added to PATH. An ffmpeg already on PATH is used first;
  **Offer ffmpeg** under Settings > Updates turns the question off.
- **Render sound**: the Render window's player plays a composition's
  sound, and a **Sound** row sets whether the mix goes into the video and
  its bitrate (AAC for MP4, Opus for WebM, 96 to 320 kbps; MOV keeps it
  uncompressed).
- **Downloads show their progress**: the ffmpeg download and Update now
  draw the Render queue's bar, with megabytes, time left and a percentage.
- **Copy, cut and paste art meshes**: Ctrl+C, Ctrl+X and Ctrl+V (and
  Edit > Cut, Copy, Paste outside Mesh Edit) carry the selected meshes with
  every keyform. A copy lands right above its original as "<name> copy",
  and a cut and paste is a move. They paste into another project too:
  parameters are matched by ID (a missing one is created with the same
  range), parents by ID, and the artwork comes along as a texture page of
  its own.
- **Reorder parameters by dragging**: drag a parameter's name onto another
  to put it before it, onto a folder header to move it in, or below the
  last row to send it to the end; drag a folder by its header. Parameters
  picked with Ctrl move together.
- **Face standard** in Automatic mesh: a face laid out the way riggers
  build one, two rings along the edge and two hubs on the centre line. The
  presets are now a dropdown, which reads Custom once a number is edited.
- **Mirror Edit** can be switched from Mesh Edit's own panel, and the main
  tool strip greys out in Mesh Edit to point at that panel.
- **Faster playback.** A deformer or mesh whose keyforms blend the same
  as last frame, under a parent that did not move, keeps its points
  instead of making them again, and warp deformers carry points faster.
  What is drawn is the same to the bit.
- Every new string of this release is in all six languages.

### Fixed

- A right-click on a parameter slider, the 2D pad or a mixer fader no
  longer moves it, so Reflect, Delete Keyform and Motion Mirroring act on
  the key the slider was on. Delete Keyform says why when it finds nothing.
- Motion Mirroring pairs each point of a mesh with the point across its
  middle, so a turn mirrors into a true mirror image.
- API > API Settings… opens the API tab and File > Keyboard Shortcuts…
  the Keyboard tab; both opened the tab before it.
- Ctrl+V pastes points copied in Mesh Edit; it did nothing, since the
  system clipboard was empty.
- Interface wording: the Keyboard tab names New project and Export model as
  the menus do and shows [ and ] for the bracket keys; the Guide Line menus
  and the workspace dropdown are in Title Case; Settings > Audio's heading
  is in capitals; and the Timeline, Render window and File menu say
  animation where they said scene.

- A middle-button drag only pans the view; it used to move the mesh or
  points under the pointer as well.
- The Select tool is called Arrow, as its shortcut always said.
- Layers from PSD, Krita and Clip Studio files keep their blend mode on
  import: Multiply and Add (Linear Dodge) as the modes every moc3 has, the
  rest as Cubism 5.3's; Vivid Light becomes Linear Light, the nearest, and
  a mode with nothing close is drawn normal with a note in the log.
- Re-import PSD applies each layer's blend mode to the mesh it updates;
  it used to keep the mesh's old mode, so a layer set to Multiply after
  the first import stayed Normal.
- The workspace picker in the toolbar is a dropdown styled like the menu
  bar's menus, with the open workspace in the accent colour.
- Cubism 4 projects open: their archive table has no tag list (a raw
  all-ones count) and their deformers carry no original shape, both of
  which the .cmo3 importer refused.

- Discord Rich Presence shows the app's picture under any Application ID:
  the image is fetched from ayatsuri-2d.org instead of an uploaded asset.
- Running out of video memory no longer crashes the editor (and no longer
  crashes it again every frame); the canvas says so instead (#3).
- Re-import PSD no longer piles up texture pages: pages no mesh uses are
  dropped after a re-import and when a model opens (#4).
- Mesh Edit holds one full-size texture page at a time instead of keeping
  every page it showed (#5).
- No CPU use while nothing moves with Follow or Physics on (#6).
- The editor sleeps fully at rest again: the Parts and Deformer palettes
  no longer redraw every frame waiting for previews of rows scrolled out
  of view, the API server waits for a connection instead of checking
  twenty times a second, and the sound output closes when playback stops
  instead of streaming silence.
- The physics pendulum preview's cross sits on the pendulum's root and
  turns with the group's angle inputs as they move, with the angle shown
  beside it, as in Cubism.
- The Physics page no longer has a Help menu linking to Cubism's manual;
  Ayatsuri2D's own manual comes later.
- The Texture Atlas page zooms (scroll or pinch around the pointer, the
  zoom buttons, Fit or a double-click to see the whole page) and pans
  (drag empty space, or the middle button anywhere).
- The Texture Atlas, Render, Timelapse and Guide Line Settings windows
  use the same frame as every other dialog (title strip to move it, the
  same close button) and can be resized from the corner.
- An idle or minimised window gives its full-size textures back after
  ten seconds again; nothing woke the editor to do it, and a minimised
  window never passed the drop to the GPU.
- Ctrl+click toggles a mesh in and out of the selection in the Parts
  palette, the Deformer palette (which gets multi-select) and on the
  canvas (#7).
- Every brush sizes the same way: B + drag, or [ and ] (#9).
- Warp Bezier handles stay as you leave them instead of springing back
  to a third of the span, and are saved with the model (#10).
- Undo history is capped by memory (256 MB) as well as by steps, so big
  rigs can't fill RAM with undo.

### Contributing

- **CONTRIBUTING.md** and a **code of conduct** are in the repository: how
  to report bugs (and security problems, privately), ask for features,
  try test builds, fix translations and send code, plus the rules the code
  keeps. Contributions must be your own work: pull requests, issues and
  comments written with AI tools are not accepted.

### Thanks

- **[@seoriboi](https://github.com/seoriboi)** asked for the Temporary
  Deform Tool (#14), tried the test builds on Windows before release, and
  sent the feedback that became #15 to #21: Shift moving straight,
  middle-button panning, keys you can tell apart, right-click onto a key,
  key colours, the canvas colour and extended interpolation. Thank you!

## 0.5.5 (2026-10-14)

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

## 0.5.4 (2026-10-13)

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

## 0.5.3 (2026-10-12)

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

## 0.5.2 (2026-10-12)

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

## 0.5.1 (2026-10-12)

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
