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
| Load a model from bytes, validated | moc3 | **have** (`.aom` and `.moc3`) | the moc3 parser comes in through the import crate with its paint and project parsers compiled out |
| Canvas size, origin, pixels per unit | yes | **have** | |
| Parameter count, ids, min, max, default, values | yes | **have** | arrays by index |
| Parameter key values (the authored keyform positions) | yes | **have** | `ay2d_parameter_keys` |
| Parameter repeat flag | 5.x | **have** | `ay2d_parameter_repeat` |
| Part count, ids, opacities | yes | **have** | |
| Part parent index | yes | **have** | `ay2d_part_parent` |
| Drawable count, ids | yes | **have** | |
| Vertex positions, uvs, indices | yes | **have** | views, no copies |
| Render order (back to front) | yes | **have** | `ay2d_drawables` is already sorted |
| Draw order values | yes | **abi** | rarely needed by a host |
| Texture index per drawable | yes | **have** | |
| Blend mode, double sided, inverted mask | yes | **have** | flags |
| Masks (clipping) | yes | **have** | mesh indices; geometry call for invisible masks |
| Opacity per drawable | yes | **have** | |
| Multiply and screen colour per drawable | 4.2+ | **runtime** | not in the model format, the editor or the core; three-crate change |
| Drawable parent part | yes | **have** | `ay2d_mesh_part` |
| Dynamic flags: visible, visibility changed, opacity changed, draw order changed, render order changed, vertices changed | yes | **have** | `ay2d_mesh_changes`, four bits per mesh |
| Update | yes | **have** | |
| Use from several threads | host's job | **have** | set and read from any thread; one driving thread, a second gets `BUSY`; render reads frame N while N + 1 is made |
| Parameter and part values written by physics read back | yes | **have** | |
| Moc consistency check / version | yes | **host** | `.aom` validates on load; `ay2d_version` |

## Framework level (what CubismFramework adds)

| Capability | SDK | Ayatsuri | Notes |
|---|---|---|---|
| model3.json bundle loading (textures, physics, motions, expressions, display info, pose) | yes | **have** | one `.aom` carries all of it; a moc3 bundle comes in piece by piece with `ay2d_add_physics3`, `ay2d_add_motion3`, `ay2d_add_expression3`; the host decodes textures and reads the small model3.json itself |
| Motion playback: play, fade in and out, loop, finished, seek, speed | yes | **have** | `ay2d_motion_*`; one motion in front with a crossfade to the next, written over the pose before physics. No priority queue: the host decides what plays. |
| Expressions with fades, add/multiply/overwrite | yes | **have** | `ay2d_expression_*` |
| Motion events (UserData fired at a time) | yes | **have** | `ay2d_motion_events`, polled after update |
| Physics | yes | **have** | plus wind and reset, which the SDK lacks |
| Pose (pose3: part groups that fade against each other) | yes | **runtime** | the format has no pose groups; part opacities cover the manual case |
| Eye blink | yes | **host** or helper | a few lines in any host; an optional `ay2d_blink` helper could come later |
| Breath | yes | **host** or helper | as above |
| Look at / drag | yes | **host** | input mapping to angle parameters |
| Lip sync from audio volume | yes | **host** or helper | the host has the audio; a volume-to-mouth helper is small |
| MotionSync (phoneme lip sync) | 5.x | **runtime** | out of scope for ABI 1 |
| Display info (cdi3: names, parameter groups) | yes | **have** from `.aom` | `ay2d_*_name`, `ay2d_parameter_group*`, `ay2d_model_group*`; reading a bundle's cdi3.json is still to do |
| User data per drawable (userdata3) | yes | **runtime** | not in the format |
| Renderers (OpenGL, Metal, Direct3D, Vulkan, Cocos, Unity) | yes | **host** | the ABI hands geometry and leaves drawing to the engine; reference renderers (a wgpu sample, a Unity sample) would lower the first step |
| Compositions (scenes layered over one clock) | no | **have** | Ayatsuri only; they play as motions |
| Physics playlists | no | editor only | not a runtime feature |
| Art paths, glue | no | **have** | evaluated inside update; nothing for the host to do |

## What ABI 1 covers today

Rig evaluation from `.aom` or `.moc3`, parameters and part opacities as
arrays, physics with wind and reset, drawables in order with blend flags
and masks, per-mesh geometry and change flags for GPU hosts, textures as
stored, motions with fades, loops and events, expressions, the bundle's
physics, motion and expression files, display names and groups, error
reporting, the panic guard, and threading: values in and results out from
any thread, one thread driving. A streaming app (tracking, idle motions,
expressions, physics) is complete on ABI 1 with its own renderer.

## Next chunks, in order

1. **cdi3.json from a bundle** (`ay2d_add_display3`), so a moc3 bundle
   shows its names too.
2. **Multiply and screen colours**: format, editor and core first, then
   two floats per drawable in the ABI (fields appended, no version bump).
3. **Pose groups**: a format addition and a small runtime, then
   `ay2d_pose_*`.
4. **Helpers** (blink, breath, lip sync from volume): optional, tiny, and
   they make the first host shorter.
5. **Reference renderers and bindings**: a wgpu sample host, a Unity
   package, a Godot extension, a Python wheel. Nothing in the ABI, all in
   the reach of it.
6. **A moc3 to `.aom` converter** on the command line, so an existing
   model library moves over in one step.

## What stays out, and why

- **Paint and project parsers in the runtime.** PSD, Krita, Clip Studio
  and cmo3 reading stay behind the import crate's `documents` feature,
  which the ABI compiles out (`cargo xtask layers` enforces it). The moc3
  reader is a safe, bounds-checked parser and comes in.
- **Texture decoding.** Every engine has an image loader; shipping one
  in the ABI would be a second copy with its own bugs.
- **Rendering.** The SDK ships renderers because it must; an ABI that
  hands geometry works in every engine and every graphics API at once.
- **Networking, tracking, audio.** Host concerns. The editor speaks the
  VTube Studio plugin protocol for tool integration; the runtime does not.
