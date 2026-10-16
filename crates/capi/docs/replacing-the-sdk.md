# Replacing the Cubism SDK: what the ABI needs

A host that today ships Live2D's Cubism SDK (the native Core plus the
Framework) should be able to put `ayatsuri.h` in its place. This is the
checklist: what such a host gets from the SDK, what the Ayatsuri runtime
already does, what the C ABI exposes, and what is left. Each row says
whether the gap is ABI work only (the runtime has it; it is a matter of
exposing it) or needs runtime work first.

Legend: **have** in ABI 1; **abi** the runtime has it, the ABI does not
yet; **runtime** neither has it; **host** belongs in the host by design.

## Core level (what Live2DCubismCore does)

| Capability | SDK | Ayatsuri | Notes |
|---|---|---|---|
| Load a model from bytes, validated | moc3 | **have** (`.aom`) | moc3 is read by the editor and the import crate, never by the ABI; a model reaches the runtime as `.aom`. A command-line converter (moc3 bundle to `.aom`) would close the workflow gap for existing models. |
| Canvas size, origin, pixels per unit | yes | **have** | |
| Parameter count, ids, min, max, default, values | yes | **have** | arrays by index |
| Parameter key values (the authored keyform positions) | yes | **abi** | in the model's bindings; a UI that shows key dots wants it |
| Parameter repeat flag | 5.x | **abi** | in the model |
| Part count, ids, opacities | yes | **have** | |
| Part parent index | yes | **abi** | in the model |
| Drawable count, ids | yes | **have** | |
| Vertex positions, uvs, indices | yes | **have** | views, no copies |
| Render order (back to front) | yes | **have** | `ay2d_drawables` is already sorted |
| Draw order values | yes | **abi** | rarely needed by a host |
| Texture index per drawable | yes | **have** | |
| Blend mode, double sided, inverted mask | yes | **have** | flags |
| Masks (clipping) | yes | **have** | mesh indices; geometry call for invisible masks |
| Opacity per drawable | yes | **have** | |
| Multiply and screen colour per drawable | 4.2+ | **runtime** | not in the model format, the editor or the core; three-crate change |
| Drawable parent part | yes | **abi** | in the model |
| Dynamic flags: visible, visibility changed, opacity changed, draw order changed, render order changed, vertices changed | yes | **abi** | the core knows what moved; exposing per-mesh dirty bits lets a GPU host skip uploads |
| Update | yes | **have** | |
| Parameter and part values written by physics read back | yes | **have** | |
| Moc consistency check / version | yes | **host** | `.aom` validates on load; `ay2d_version` |

## Framework level (what CubismFramework adds)

| Capability | SDK | Ayatsuri | Notes |
|---|---|---|---|
| model3.json bundle loading (textures, physics, motions, expressions, display info, pose) | yes | **have** by design | one `.aom` carries all of it; the host decodes textures |
| Motion playback: play, fade in and out, loop, priority, queue, finished callback | yes | **abi** | animations are in the file with curves and events; the core samples curves (`anim::sample`). Needs an `ay2d_motion_*` surface: list, start by name or index with fade and loop, stop, advance with the frame's `dt`, weights blended into the pose before update. |
| Expressions with fades, add/multiply/overwrite | yes | **abi** | in the file; blend rules in the editor. Needs `ay2d_expression_*`: list, set with fade, clear. |
| Motion events (UserData fired at a time) | yes | **abi** | in the animations; a polled event queue, no callbacks |
| Physics | yes | **have** | plus wind and reset, which the SDK lacks |
| Pose (pose3: part groups that fade against each other) | yes | **runtime** | the format has no pose groups; part opacities cover the manual case |
| Eye blink | yes | **host** or helper | a few lines in any host; an optional `ay2d_blink` helper could come later |
| Breath | yes | **host** or helper | as above |
| Look at / drag | yes | **host** | input mapping to angle parameters |
| Lip sync from audio volume | yes | **host** or helper | the host has the audio; a volume-to-mouth helper is small |
| MotionSync (phoneme lip sync) | 5.x | **runtime** | out of scope for ABI 1 |
| Display info (cdi3: names, parameter groups) | yes | **abi** | in the file (`display`): names for a UI that shows them |
| User data per drawable (userdata3) | yes | **runtime** | not in the format |
| Renderers (OpenGL, Metal, Direct3D, Vulkan, Cocos, Unity) | yes | **host** | the ABI hands geometry and leaves drawing to the engine; reference renderers (a wgpu sample, a Unity sample) would lower the first step |
| Compositions (scenes layered over one clock) | no | **abi** | Ayatsuri only; `comp::sample` exists; worth exposing with motions |
| Physics playlists | no | editor only | not a runtime feature |
| Art paths, glue | no | **have** | evaluated inside update; nothing for the host to do |

## What ABI 1 covers today

Rig evaluation, parameters and part opacities as arrays, physics with wind
and reset, drawables in order with blend flags and masks, per-mesh
geometry for GPU hosts, textures as stored, error reporting and the
panic guard. A viewer that tracks a face and renders the model is
complete on ABI 1. An app that also plays idle motions and expressions
has to sample them on the host side until the next chunk lands.

## Next chunks, in order

1. **Motions and expressions** (`ay2d_motion_*`, `ay2d_expression_*`,
   events): the runtime has every piece; this is the biggest host-side
   burden left and the one every streaming app hits first.
2. **Display info and model facts** (`ay2d_parameter_name`,
   `ay2d_parameter_keys`, parent indices, repeat flag): read-only, cheap,
   wanted by any UI.
3. **Dirty flags per mesh** after update, so GPU hosts upload only what
   moved.
4. **Multiply and screen colours**: format, editor and core first, then
   two floats per drawable in the ABI (fields appended, no version bump).
5. **Pose groups**: a format addition and a small runtime, then
   `ay2d_pose_*`.
6. **Helpers** (blink, breath, lip sync from volume): optional, tiny, and
   they make the first host shorter.
7. **Reference renderers and bindings**: a wgpu sample host, a Unity
   package, a Godot extension, a Python wheel. Nothing in the ABI, all in
   the reach of it.
8. **A moc3 to `.aom` converter** on the command line, so an existing
   model library moves over without opening each file in the editor.

## What stays out, and why

- **moc3 reading in the runtime.** The parser is the hostile-input
  surface; it lives in `ayatsuri-import` and the ABI never compiles it
  (`cargo xtask layers` enforces this).
- **Texture decoding.** Every engine has an image loader; shipping one
  in the ABI would be a second copy with its own bugs.
- **Rendering.** The SDK ships renderers because it must; an ABI that
  hands geometry works in every engine and every graphics API at once.
- **Networking, tracking, audio.** Host concerns. The editor speaks the
  VTube Studio plugin protocol for tool integration; the runtime does not.
