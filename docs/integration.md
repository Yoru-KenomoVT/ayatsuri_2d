# Integration

## The C ABI

Everything binds through one header. Unity, Unreal, Godot, Python, JavaScript
and anything else that can call C.

Two rules make it cheap:

**Coarse, not chatty.** Arrays in, arrays out. Never a call per object.

    chatty     set_param(model, "AngleX", 0.5)  x 100 params x 60fps
               = 6000 calls/sec, and every binding pays for it

    coarse     set_params(model, ids, values, 100)  x 60fps
               = 60 calls/sec, free in any language

**Both vertex paths from day one.** An embedder that wants CPU vertex data gets
it; one that can keep everything on the GPU gets a buffer handle and skips the
round trip entirely. A fast path cannot be added to an ABI later without
breaking everyone, so it is there from the first published header.

Panics never cross the boundary: an embedder crash is our crash.

The full developer reference, with every call, is
[`crates/capi/docs/reference.md`](../crates/capi/docs/reference.md). What
the ABI still needs to stand in for the Cubism SDK is in
[`replacing-the-sdk.md`](../crates/capi/docs/replacing-the-sdk.md).

In short:

- **Models** load from an `.aom` (everything in one file) or a `.moc3`
  (the rig; the bundle's physics, motion and expression JSON files are
  added as bytes). The paint and project parsers never compile into the
  library: it takes the import crate with its `documents` feature off, and
  `cargo xtask layers` checks it.
- **A frame** is set, update, draw: parameters in as arrays by index, one
  update that runs motions, expressions and physics and publishes the
  result, then the drawables back to front as views into it.
- **Threads**: setting values and reading the published result work from
  any thread at any time; the calls that drive the model (update, motions,
  expressions, physics settings) belong to one thread, and a second one
  gets `BUSY`. Update writes one published frame while hosts read the
  other, so a render thread draws frame N while N + 1 is made.
- **Safety**: every raw pointer is turned into a reference in one file
  (`handle.rs`), a test keeps `unsafe` out of the rest, and a panic inside
  is caught at the boundary and returned as a status.

### The header

`crates/capi/include/ayatsuri.h` is generated from the Rust source by
`cargo xtask capi` (cbindgen), and `cargo xtask capi --check` in CI fails
when the committed copy is stale. Edit the Rust, never the header.

### Linking

`cargo xtask capi` leaves `target/capi/` with the header, the shared library
for plugin hosts and the static one to compile in, built with full LTO and
stripped symbols. With the static library the host's own linker
(`--gc-sections`, `-dead_strip`, `/OPT:REF`) drops every function the host
never calls. `crates/capi/examples/host.c` is a complete host and the check
CI runs.

## Plugin protocol

Ayatsuri2D speaks the VTube Studio plugin protocol, which is publicly
documented. Existing tracking bridges, stream integrations and scripts work
unchanged; they point at a different port.

    talk to other apps    websocket client
    be driven by them     websocket server, same protocol, plus our own
                          endpoints for things the original cannot express

## The network never touches the frame path

    network thread  --write-->  triple-buffered params  --read-->  render thread

No locks, no allocation and no async in the frame loop. JSON parsing at 60fps
is exactly what the predictability requirement forbids, so the network side
owns its own thread and hands over by buffer swap.

## Standard parameters

The conventional parameter names (`ParamAngleX`, `ParamEyeLOpen`,
`ParamMouthOpenY` and the rest) are published by Live2D and adopted here
natively.

Together with the plugin protocol and rig presets, that means a model rigged in
Ayatsuri2D works with existing face tracking on the first try, with no mapping
step.
