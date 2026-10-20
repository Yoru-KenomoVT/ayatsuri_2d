# Diagrams for the C ABI reference

The diagrams [`../reference.md`](../reference.md) shows, one SVG each in
this folder. They are hand-written SVG with real text and named groups
(`<g id="...">`), so they open cleanly in Inkscape or Figma and edit by
hand too. A shared `<style>` block at the top of each sets the fonts and
colours. Keep to the app's dark palette so they sit on the docs pages
without a frame: background
`#1e1e21`, panels `#29292d`, lines `#3a3a40`, text `#e4e4e8`, faint text
`#8e8e96`, accent `#f08e1e`, plus green `#4cc06a` and red `#e86a6a` where a
yes/no is shown.

| File | Section | Shows | Must get across |
|---|---|---|---|
| `model-anatomy.svg` | How it works | parameters → deformers → meshes → drawables; parts grouping meshes; physics reading some parameters and writing others | the host only touches parameters and reads drawables |
| `bundle-loading.svg` | Loading a model | a model3.json bundle's files each going to its call; an .aom going straight into `ay2d_load` | .aom is one call, a bundle is one call per file |
| `update-order.svg` | Each frame | inbox → motions → expression → physics → evaluate → publish, top to bottom | the order, and that drawables come from the published result |
| `threads.svg` | Threads | three lanes over time (tracking, frame, render) and the model's inbox, two frames and core between them | set and read never wait; only the frame lane drives; render reads the front while an update writes the back |
| `drawable-anatomy.svg` | Drawing | one mesh with numbered vertices in model space beside the same vertices on the texture page; the triangle list | positions and uvs pair up by index; indices are triangles |
| `masks.svg` | Drawing, Masks | mask mesh, masked mesh overflowing it, the stencil, the clipped result; a second row for the inverted case | the mask decides where the mesh shows |
| `canvas-coords.svg` | Drawing, Framing | the canvas in pixels with its origin; the same rectangle in model units; the conversion | `pixel = unit × ppu + origin`, y down |
| `crossfade.svg` | Motions | weight over time for two motions during a switch | one fades in as the other fades out |
| `physics-pendulum.svg` | Physics | input parameters pushing the top of a pendulum chain; outputs read at chosen nodes | inputs push, outputs read the swing |
| `view-lifetime.svg` | Memory and lifetimes | updates N, N+1, N+2 on a timeline; a view taken after N valid through N+1, stale when N+2 starts | the "update after next" rule |
