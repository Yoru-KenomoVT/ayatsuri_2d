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
